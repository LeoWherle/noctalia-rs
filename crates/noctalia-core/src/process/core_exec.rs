//! Port of `runSyncProcess` and its immediate dependents in
//! `src/core/process/process.cpp` (task 1.6.1). See the module doc comment in
//! `process::mod` for the two load-bearing divergences from the C++.

use std::io::{ErrorKind, Read};
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CANCEL_POLL_CAP_MS: i32 = 250;

/// A streaming output callback, receiving each raw chunk as it's read.
type OutputCb<'a> = Option<&'a mut dyn FnMut(&[u8])>;

#[derive(Debug, Clone, Default)]
pub struct RunResult {
    pub exit_code: i32,
    pub out: Vec<u8>,
    pub err: Vec<u8>,
    pub timed_out: bool,
    pub out_truncated: bool,
    pub err_truncated: bool,
}

impl RunResult {
    fn failed() -> Self {
        Self {
            exit_code: -1,
            ..Default::default()
        }
    }

    /// Matches the C++ `RunResult::operator bool()`.
    pub fn success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }
}

#[derive(Debug, Clone)]
pub struct EnvOverride {
    pub name: String,
    /// `None` unsets the variable in the child.
    pub value: Option<String>,
}

pub struct RunOptions {
    pub timeout: Option<Duration>,
    pub max_output_bytes: usize,
    /// When set, the run is cancellable: once the flag turns true the child's
    /// process group is terminated and the call returns.
    pub cancel: Option<Arc<AtomicBool>>,
    pub env: Vec<EnvOverride>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            timeout: None,
            max_output_bytes: usize::MAX,
            cancel: None,
            env: Vec::new(),
        }
    }
}

pub fn run_sync(args: &[String]) -> RunResult {
    run_sync_with_options(args, RunOptions::default())
}

pub fn run_sync_with_options(args: &[String], options: RunOptions) -> RunResult {
    run_sync_process(args, &options, None, None)
}

pub(crate) fn run_sync_process(
    args: &[String],
    options: &RunOptions,
    mut stdout_cb: OutputCb<'_>,
    mut stderr_cb: OutputCb<'_>,
) -> RunResult {
    if args.is_empty() || args[0].is_empty() {
        return RunResult::failed();
    }

    let mut command = Command::new(&args[0]);
    command.args(&args[1..]);
    apply_env_overrides(&mut command, &options.env);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    // New process group led by the child, so a timeout/cancel can signal the
    // whole tree (e.g. a shell spawning a real worker), not just the direct
    // child. See the module doc comment for why a failed spawn (which also
    // covers a failed exec inside the child, unlike the C++) collapses to -1.
    command.process_group(0);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return RunResult::failed(),
    };

    let pid = child.id() as i32;

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    if let Some(stdout) = &stdout {
        set_non_blocking(stdout.as_raw_fd());
    }
    if let Some(stderr) = &stderr {
        set_non_blocking(stderr.as_raw_fd());
    }

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut exited = false;
    let mut timed_out = false;
    let mut out_truncated = false;
    let mut err_truncated = false;
    let mut exit_code = -1;
    let deadline = options.timeout.map(|timeout| Instant::now() + timeout);

    loop {
        drain_available(
            &mut stdout,
            &mut out,
            options.max_output_bytes,
            &mut out_truncated,
            reborrow_cb(&mut stdout_cb),
        );
        drain_available(
            &mut stderr,
            &mut err,
            options.max_output_bytes,
            &mut err_truncated,
            reborrow_cb(&mut stderr_cb),
        );

        if !exited {
            exited = wait_no_hang(&mut child, &mut exit_code);
        }
        if exited {
            drain_available(
                &mut stdout,
                &mut out,
                options.max_output_bytes,
                &mut out_truncated,
                reborrow_cb(&mut stdout_cb),
            );
            drain_available(
                &mut stderr,
                &mut err,
                options.max_output_bytes,
                &mut err_truncated,
                reborrow_cb(&mut stderr_cb),
            );
            drop(stdout.take());
            drop(stderr.take());
            break;
        }

        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            timed_out = true;
            terminate_and_wait(pid, &mut child, &mut exit_code);
        }

        // Cancellation reuses the timed-out drain+break path below, same as the C++.
        if !timed_out
            && let Some(cancel) = &options.cancel
            && cancel.load(Ordering::Relaxed)
        {
            terminate_and_wait(pid, &mut child, &mut exit_code);
            timed_out = true;
        }

        if timed_out {
            drain_available(
                &mut stdout,
                &mut out,
                options.max_output_bytes,
                &mut out_truncated,
                reborrow_cb(&mut stdout_cb),
            );
            drain_available(
                &mut stderr,
                &mut err,
                options.max_output_bytes,
                &mut err_truncated,
                reborrow_cb(&mut stderr_cb),
            );
            drop(stdout.take());
            drop(stderr.take());
            break;
        }

        let mut fds = Vec::with_capacity(2);
        if let Some(stream) = &stdout {
            fds.push(libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        if let Some(stream) = &stderr {
            fds.push(libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }

        if !fds.is_empty() {
            let mut wait_ms = if exited { 0 } else { poll_timeout_ms(deadline) };
            // Cap the wait when cancellable so an idle stream still wakes to check the flag.
            if options.cancel.is_some() && !(0..=CANCEL_POLL_CAP_MS).contains(&wait_ms) {
                wait_ms = CANCEL_POLL_CAP_MS;
            }
            // SAFETY: `fds` is a valid, live slice of `pollfd` for the duration of
            // this call; each `fd` inside it is a valid, open descriptor owned by
            // `stdout`/`stderr` above.
            let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait_ms) };
            if rc < 0 && std::io::Error::last_os_error().kind() != ErrorKind::Interrupted {
                break;
            }
        } else if !exited {
            if deadline.is_none() {
                exit_code = wait_blocking(&mut child);
                exited = true;
                continue;
            }
            // SAFETY: a null `fds` pointer with `nfds == 0` is a documented no-op
            // poll(2) call used purely for its timeout, matching the C++'s
            // `::poll(nullptr, 0, ...)`.
            unsafe {
                libc::poll(std::ptr::null_mut(), 0, poll_timeout_ms(deadline).min(10));
            }
        }
    }

    trim_trailing_line_endings(&mut out);
    trim_trailing_line_endings(&mut err);

    RunResult {
        exit_code,
        out,
        err,
        timed_out,
        out_truncated,
        err_truncated,
    }
}

fn apply_env_overrides(command: &mut Command, overrides: &[EnvOverride]) {
    for item in overrides {
        if !valid_env_name(&item.name) {
            continue;
        }
        match &item.value {
            Some(value) => {
                command.env(&item.name, value);
            }
            None => {
                command.env_remove(&item.name);
            }
        }
    }
}

fn valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('=')
}

fn set_non_blocking(fd: std::os::unix::io::RawFd) -> bool {
    // SAFETY: `fd` is a valid, open descriptor owned by the child's stdout/
    // stderr pipe for the duration of this call.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return false;
    }
    // SAFETY: as above.
    unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == 0 }
}

trait ReadFd: Read + AsRawFd {}
impl ReadFd for ChildStdout {}
impl ReadFd for ChildStderr {}

/// Reborrowing `Option<&mut dyn FnMut(&[u8])>` via `.as_deref_mut()` at each
/// call site ties every reborrow to the outer function parameter's lifetime
/// instead of a fresh, per-call one (a known rough edge of `&mut dyn Trait`
/// reborrows flowing through a generic function boundary), which the borrow
/// checker then rejects on the second call in the loop. Reborrowing through an
/// explicit `&mut **f` here sidesteps it.
fn reborrow_cb<'a>(cb: &'a mut OutputCb<'_>) -> OutputCb<'a> {
    match cb {
        Some(f) => Some(&mut **f),
        None => None,
    }
}

fn drain_available<R: ReadFd>(
    stream: &mut Option<R>,
    out: &mut Vec<u8>,
    max_bytes: usize,
    truncated: &mut bool,
    mut callback: OutputCb<'_>,
) {
    let Some(handle) = stream.as_mut() else {
        return;
    };

    let mut buf = [0u8; 4096];
    loop {
        match handle.read(&mut buf) {
            Ok(0) => {
                *stream = None;
                return;
            }
            Ok(n) => {
                if let Some(callback) = callback.as_deref_mut() {
                    callback(&buf[..n]);
                }
                let remaining = max_bytes.saturating_sub(out.len());
                let append = n.min(remaining);
                if append > 0 {
                    out.extend_from_slice(&buf[..append]);
                }
                if append < n {
                    *truncated = true;
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(err) if err.kind() == ErrorKind::WouldBlock => return,
            Err(_) => {
                *stream = None;
                return;
            }
        }
    }
}

fn exit_code_from_status(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    if let Some(code) = status.code() {
        return code;
    }
    if let Some(signal) = status.signal() {
        return 128 + signal;
    }
    -1
}

fn wait_no_hang(child: &mut Child, exit_code: &mut i32) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                *exit_code = exit_code_from_status(status);
                return true;
            }
            Ok(None) => return false,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => {
                *exit_code = -1;
                return true;
            }
        }
    }
}

fn wait_blocking(child: &mut Child) -> i32 {
    match child.wait() {
        Ok(status) => exit_code_from_status(status),
        Err(_) => -1,
    }
}

fn terminate_and_wait(pid: i32, child: &mut Child, exit_code: &mut i32) {
    // SAFETY: `pid` is this child's pid, not yet reaped on this path; `-pid`
    // (its process group, since we set it as the group leader above) and
    // `pid` are both valid `kill(2)` targets.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
        libc::kill(pid, libc::SIGTERM);
    }

    let mut reaped = false;
    for _ in 0..10 {
        if wait_no_hang(child, exit_code) {
            reaped = true;
            break;
        }
        // SAFETY: null `fds`/`nfds == 0` poll(2) call used purely as a sleep,
        // matching the C++.
        unsafe {
            libc::poll(std::ptr::null_mut(), 0, 10);
        }
    }

    // SAFETY: as above.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
        libc::kill(pid, libc::SIGKILL);
    }
    if !reaped {
        *exit_code = wait_blocking(child);
    }
}

fn poll_timeout_ms(deadline: Option<Instant>) -> i32 {
    let Some(deadline) = deadline else {
        return POLL_INTERVAL.as_millis() as i32;
    };
    let now = Instant::now();
    if now >= deadline {
        return 0;
    }
    let remaining = deadline - now;
    let bounded = remaining.clamp(Duration::from_millis(1), POLL_INTERVAL);
    bounded.as_millis() as i32
}

fn trim_trailing_line_endings(value: &mut Vec<u8>) {
    while matches!(value.last(), Some(b'\n') | Some(b'\r')) {
        value.pop();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::Instant as StdInstant;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_args_returns_failure() {
        let result = run_sync(&[]);
        assert_eq!(result.exit_code, -1);
        assert!(!result.success());
    }

    #[test]
    fn empty_first_arg_returns_failure() {
        let result = run_sync(&args(&[""]));
        assert_eq!(result.exit_code, -1);
    }

    #[test]
    fn runs_true_and_reports_success() {
        let result = run_sync(&args(&["true"]));
        assert_eq!(result.exit_code, 0);
        assert!(result.success());
    }

    #[test]
    fn runs_false_and_reports_failure() {
        let result = run_sync(&args(&["false"]));
        assert_eq!(result.exit_code, 1);
        assert!(!result.success());
    }

    #[test]
    fn captures_stdout_and_stderr() {
        let result = run_sync(&args(&["/bin/sh", "-lc", "printf out; printf err >&2"]));
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.out, b"out");
        assert_eq!(result.err, b"err");
    }

    #[test]
    fn nonexistent_command_fails_without_hanging() {
        // Divergence from the C++ (which would report exit code 127 here, since
        // its hand-rolled execvp failure path falls through to `_exit(127)`
        // observed by the parent) — see the module doc comment.
        let result = run_sync(&args(&["/definitely/not/a/real/command-xyz"]));
        assert_eq!(result.exit_code, -1);
    }

    #[test]
    fn env_overrides_apply_to_child() {
        // SAFETY: cargo runs tests in this module concurrently, and sibling
        // tests do call `Command::spawn` on other threads while this one runs.
        // That's not a data race against *those*: std's `Command::spawn`
        // captures the environment via `env::vars_os()`, which (like
        // `set_var`/`remove_var`) takes std's process-wide `ENV_LOCK`
        // (verified in library/std/src/sys/env/unix.rs) before touching
        // `environ` — reads and writes are mutually exclusive regardless of
        // which thread issues them. It is *not* synchronized against
        // `process::detached`'s raw `libc::fork()`, which bypasses `ENV_LOCK`
        // entirely, so this test still takes the shared
        // `test_support::ENV_MUTATION_LOCK` to rule that pairing out within
        // this test binary (see `detached`'s module doc comment).
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        unsafe {
            std::env::set_var("NOCTALIA_PROCESS_UNSET_TEST", "parent");
        }

        let options = RunOptions {
            env: vec![
                EnvOverride {
                    name: "NOCTALIA_PROCESS_SET_TEST".to_string(),
                    value: Some("child".to_string()),
                },
                EnvOverride {
                    name: "NOCTALIA_PROCESS_UNSET_TEST".to_string(),
                    value: None,
                },
            ],
            ..RunOptions::default()
        };

        let result = run_sync_with_options(
            &args(&[
                "/bin/sh",
                "-lc",
                r#"printf '%s/%s' "$NOCTALIA_PROCESS_SET_TEST" "${NOCTALIA_PROCESS_UNSET_TEST-unset}""#,
            ]),
            options,
        );

        // SAFETY: as above.
        unsafe {
            std::env::remove_var("NOCTALIA_PROCESS_UNSET_TEST");
        }

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.out, b"child/unset");
    }

    #[test]
    fn timeout_terminates_a_long_running_command() {
        let options = RunOptions {
            timeout: Some(Duration::from_millis(100)),
            ..RunOptions::default()
        };
        let start = StdInstant::now();
        let result = run_sync_with_options(&args(&["/bin/sh", "-lc", "sleep 5"]), options);
        assert!(result.timed_out);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "should have terminated near the timeout, not run to completion"
        );
    }

    #[test]
    fn cancel_flag_terminates_a_running_command() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_setter = Arc::clone(&cancel);
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancel_setter.store(true, Ordering::Relaxed);
        });

        let options = RunOptions {
            cancel: Some(cancel),
            ..RunOptions::default()
        };
        let start = StdInstant::now();
        let result = run_sync_with_options(&args(&["/bin/sh", "-lc", "sleep 5"]), options);
        setter.join().expect("cancel-setting thread panicked");

        assert!(
            result.timed_out,
            "cancellation reuses the timed_out flag, matching the C++"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn output_byte_limit_truncates_but_still_streams_full_chunks_to_callbacks() {
        let options = RunOptions {
            max_output_bytes: 3,
            ..RunOptions::default()
        };

        let mut streamed_out = Vec::new();
        let mut streamed_err = Vec::new();
        let mut stdout_cb = |chunk: &[u8]| streamed_out.extend_from_slice(chunk);
        let mut stderr_cb = |chunk: &[u8]| streamed_err.extend_from_slice(chunk);

        let result = run_sync_process(
            &args(&["/bin/sh", "-lc", "printf abcdef; printf uvwxyz >&2"]),
            &options,
            Some(&mut stdout_cb),
            Some(&mut stderr_cb),
        );

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.out, b"abc");
        assert!(result.out_truncated);
        assert_eq!(result.err, b"uvw");
        assert!(result.err_truncated);
        assert_eq!(
            streamed_out, b"abcdef",
            "the callback should see the full, untruncated stream"
        );
        assert_eq!(
            streamed_err, b"uvwxyz",
            "the callback should see the full, untruncated stream"
        );
    }
}
