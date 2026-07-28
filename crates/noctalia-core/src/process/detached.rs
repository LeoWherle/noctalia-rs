//! Port of double-fork detached spawning from `src/core/process/process.cpp`
//! (task 1.6.3): `doubleForkExecDetached`, the plain callback-less
//! `runAsync(args, activationToken, workingDir)` / `runAsync(command)`
//! overloads built on it, `launchDetachedTracked`/`terminateTracked`,
//! `launchFirstAvailable`, and `commandExists`/`resolvePrivilegeEscalator`
//! (PATH search). This is a genuinely different execution strategy from
//! 1.6.2's worker-thread `run_async*` family — a real double-fork + `setsid`
//! so the exec'd process is reparented away from us entirely, matching
//! launcher app-activation semantics — not a wrapper around
//! `run_sync_process`, so it needs its own raw `fork`/`exec` machinery built
//! directly on `libc`.
//!
//! Two things worth recording up front, both inherited from the C++
//! unchanged (not fixed here, per "match C++ behavior over improving it"):
//!
//! - Forking a multithreaded process is inherently fragile: only the calling
//!   thread survives into the child, so if another thread held a lock (libc's
//!   allocator, a mutex protecting some shared state) at the instant of
//!   `fork()`, the child inherits that lock in a permanently-held state and
//!   can deadlock the moment it tries to acquire it — before it gets to
//!   `execvp`. The C++ has exactly the same exposure (it forks from the same
//!   process, which is not guaranteed single-threaded either); this isn't a
//!   Rust-specific hazard, and there's no fix here beyond what the C++
//!   already does (keep the fork-to-exec window doing only async-signal-safe
//!   work).
//! - Neither implementation takes any lock before calling `fork()`. On this
//!   crate's own test binary specifically, that matters for one concrete
//!   pairing: Rust's `std::env::set_var`/`remove_var` (used by this module's
//!   own `detached_async_inherits_launch_environment` test and by
//!   `core_exec`'s `env_overrides_apply_to_child`) go through std's internal
//!   `ENV_LOCK`, but a raw `libc::fork()` here does not — unlike
//!   `std::process::Command::spawn`, which takes `ENV_LOCK`'s read side
//!   before forking (see `core_exec`'s doc comment on that). A `set_var`
//!   write racing with this module's raw `fork()` on another thread could
//!   observe (or leave the child with) a torn `environ` array. Both of this
//!   crate's env-mutating tests take a shared `test_support::ENV_MUTATION_LOCK`
//!   for their duration to rule that out within our own test binary; nothing
//!   about production code changes.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Port of `commandExists`: absolute/relative paths (containing `/`) are
/// checked directly; bare names are searched over `$PATH` (or the same
/// hard-coded fallback the C++ uses when `$PATH` is unset/empty), each
/// candidate required to be executable *and* a regular file (so a directory
/// on `$PATH` with the right name, or one passed in directly, is rejected).
pub fn command_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.contains('/') {
        return is_executable_regular_file(Path::new(name));
    }

    let path_env = std::env::var_os("PATH");
    let path_bytes: &[u8] = match &path_env {
        Some(p) if !p.is_empty() => p.as_bytes(),
        _ => b"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    };

    let mut start = 0usize;
    loop {
        if start > path_bytes.len() {
            break;
        }
        let end = path_bytes[start..]
            .iter()
            .position(|&b| b == b':')
            .map(|i| start + i);
        let dir = match end {
            Some(e) => &path_bytes[start..e],
            None => &path_bytes[start..],
        };

        let mut candidate = Vec::with_capacity(dir.len() + 1 + name.len());
        if !dir.is_empty() {
            candidate.extend_from_slice(dir);
            candidate.push(b'/');
        }
        candidate.extend_from_slice(name.as_bytes());
        let candidate_path = Path::new(std::ffi::OsStr::from_bytes(&candidate));
        if is_executable_regular_file(candidate_path) {
            return true;
        }

        match end {
            Some(e) => start = e + 1,
            None => break,
        }
    }

    false
}

fn is_executable_regular_file(path: &Path) -> bool {
    let Ok(cpath) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `cpath` is a valid, NUL-terminated C string live for this call.
    let executable = unsafe { libc::access(cpath.as_ptr(), libc::X_OK) } == 0;
    executable
        && std::fs::metadata(path)
            .map(|m| m.is_file())
            .unwrap_or(false)
}

/// Port of `resolvePrivilegeEscalator`: prefer `run0` over `pkexec`, since
/// `pkexec` often stays on `PATH` without its setuid wrapper (e.g. on NixOS).
pub fn resolve_privilege_escalator() -> Option<String> {
    if command_exists("run0") {
        return Some("run0".to_string());
    }
    if command_exists("pkexec") {
        return Some("pkexec".to_string());
    }
    None
}

/// Port of `runAsync(const std::vector<std::string>&, const std::string&, const std::string&)`:
/// double-fork + `setsid` detached spawn, discarding the grandchild's pid.
/// When `activation_token` is non-empty, the grandchild sets
/// `XDG_ACTIVATION_TOKEN`/`DESKTOP_STARTUP_ID` (launcher activation).
pub fn launch_detached(args: &[String], activation_token: &str, working_dir: &str) -> bool {
    if args.is_empty() || args[0].is_empty() {
        return false;
    }
    double_fork_exec_detached(args, activation_token, working_dir, false).is_ok()
}

/// Port of `runAsync(const std::string&)`: shell-string composition over
/// [`launch_detached`], matching the C++'s `/bin/sh -lc` wrapping.
pub fn launch_detached_shell(command: &str) -> bool {
    if command.is_empty() {
        return false;
    }
    launch_detached(&shell_command(command), "", "")
}

/// Port of `launchDetachedTracked`: like [`launch_detached`], but reports the
/// grandchild's pid back for [`terminate_tracked`].
pub fn launch_detached_tracked(args: &[String]) -> Option<i32> {
    if args.is_empty() || args[0].is_empty() {
        return None;
    }
    double_fork_exec_detached(args, "", "", true).ok().flatten()
}

/// Port of `terminateTracked`: SIGTERM, then SIGKILL only if a non-blocking
/// reap doesn't immediately find it exited.
pub fn terminate_tracked(pid: i32) {
    if pid <= 0 {
        return;
    }
    let p = pid as libc::pid_t;
    // SAFETY: `p` is a pid previously reported by `launch_detached_tracked`;
    // signaling a pid that has already exited and been reaped is a documented
    // ESRCH no-op, not undefined behavior.
    unsafe {
        libc::kill(p, libc::SIGTERM);
    }
    let mut status = 0;
    // SAFETY: `status` is a valid, live `c_int` for the duration of this call.
    let reaped = unsafe { libc::waitpid(p, &mut status, libc::WNOHANG) };
    if reaped != p {
        // SAFETY: as above.
        unsafe {
            libc::kill(p, libc::SIGKILL);
            libc::waitpid(p, &mut status, 0);
        }
    }
}

/// Port of `launchFirstAvailable`: try each command variant in order, launching
/// (detached, no activation token/working dir) the first whose executable name
/// is found on `PATH`.
pub fn launch_first_available(command_variants: &[&[&str]]) -> bool {
    for variant in command_variants {
        if variant.is_empty() {
            continue;
        }
        if !command_exists(variant[0]) {
            continue;
        }
        let args: Vec<String> = variant.iter().map(|s| s.to_string()).collect();
        if launch_detached(&args, "", "") {
            return true;
        }
    }
    false
}

fn shell_command(command: &str) -> Vec<String> {
    vec![
        "/bin/sh".to_string(),
        "-lc".to_string(),
        command.to_string(),
    ]
}

/// Core double-fork + `setsid` engine behind [`launch_detached`] and
/// [`launch_detached_tracked`]. `Ok(pid)` reports the grandchild's pid when
/// `report_pid` is set (always `None` otherwise); `Err(())` covers every
/// failure the C++ collapses to a plain `false` (a failed `pipe`/`fork`, the
/// intermediate exiting non-zero, or — when tracked — a malformed/missing pid
/// report).
fn double_fork_exec_detached(
    args: &[String],
    activation_token: &str,
    working_dir: &str,
    report_pid: bool,
) -> Result<Option<i32>, ()> {
    let mut report_pipe = [-1i32, -1i32];
    if report_pid {
        // SAFETY: `report_pipe` is a valid, writable `[i32; 2]`.
        if unsafe { libc::pipe(report_pipe.as_mut_ptr()) } != 0 {
            return Err(());
        }
    }

    // SAFETY: `fork()` is always safe to call; the returned value is checked
    // immediately below before anything else happens in either process.
    let intermediate = unsafe { libc::fork() };
    if intermediate < 0 {
        if report_pid {
            close_pipe(&mut report_pipe);
        }
        return Err(());
    }

    if intermediate > 0 {
        // Parent: read the grandchild's pid first (the intermediate process
        // may exit before the grandchild gets a chance to write it).
        if report_pid {
            // SAFETY: `report_pipe[1]` is our own write end; no one else holds it.
            unsafe {
                libc::close(report_pipe[1]);
            }
            let mut reported: libc::pid_t = -1;
            let reported_ptr = std::ptr::addr_of_mut!(reported).cast::<libc::c_void>();
            // SAFETY: `reported_ptr` points to a valid, writable `pid_t`-sized
            // buffer for the duration of this call.
            let n = unsafe { libc::read(report_pipe[0], reported_ptr, size_of::<libc::pid_t>()) };
            // SAFETY: our own read end, not used again after this.
            unsafe {
                libc::close(report_pipe[0]);
            }
            let status = wait_for_exit(intermediate);
            let ok = status.success() && n == size_of::<libc::pid_t>() as isize && reported > 0;
            return if ok {
                Ok(Some(reported as i32))
            } else {
                Err(())
            };
        }

        let status = wait_for_exit(intermediate);
        return if status.success() { Ok(None) } else { Err(()) };
    }

    // Intermediate child: start a new session, then fork again so the
    // grandchild reparents away from us once we exit.
    if report_pid {
        // SAFETY: our own read end, unused in this branch.
        unsafe {
            libc::close(report_pipe[0]);
        }
    }

    // SAFETY: `setsid()` takes no arguments and is always safe to call.
    if unsafe { libc::setsid() } < 0 {
        if report_pid {
            write_pipe_or_ignore(report_pipe[1], &(-1i32).to_ne_bytes());
            // SAFETY: our own write end.
            unsafe {
                libc::close(report_pipe[1]);
            }
        }
        // SAFETY: `_exit` never returns; nothing after it runs.
        unsafe { libc::_exit(1) };
    }

    // SAFETY: as above.
    let worker = unsafe { libc::fork() };
    if worker < 0 {
        if report_pid {
            write_pipe_or_ignore(report_pipe[1], &(-1i32).to_ne_bytes());
            // SAFETY: our own write end.
            unsafe {
                libc::close(report_pipe[1]);
            }
        }
        // SAFETY: as above.
        unsafe { libc::_exit(1) };
    }
    if worker > 0 {
        // Still the intermediate/session-leader process: done, exit so the
        // grandchild (below) gets reparented away from us.
        if report_pid {
            // SAFETY: our own write end.
            unsafe {
                libc::close(report_pipe[1]);
            }
        }
        // SAFETY: as above.
        unsafe { libc::_exit(0) };
    }

    // Grandchild: this is the process that actually execs the target command.
    if report_pid {
        // SAFETY: `getpid()` takes no arguments and is always safe to call.
        let own_pid = unsafe { libc::getpid() };
        write_pipe_or_ignore(report_pipe[1], &own_pid.to_ne_bytes());
        // SAFETY: our own write end.
        unsafe {
            libc::close(report_pipe[1]);
        }
    }

    if !working_dir.is_empty() {
        match CString::new(working_dir) {
            // SAFETY: `c_dir` is a valid, NUL-terminated C string live for this call.
            Ok(c_dir) if unsafe { libc::chdir(c_dir.as_ptr()) } == 0 => {}
            // SAFETY: as above.
            _ => unsafe { libc::_exit(126) },
        }
    }

    if !activation_token.is_empty()
        && let Ok(c_token) = CString::new(activation_token)
    {
        // SAFETY: `c_token`/the literal env-name strings are valid,
        // NUL-terminated C strings live for the duration of these calls.
        unsafe {
            libc::setenv(c"XDG_ACTIVATION_TOKEN".as_ptr(), c_token.as_ptr(), 1);
            libc::setenv(c"DESKTOP_STARTUP_ID".as_ptr(), c_token.as_ptr(), 1);
        }
    }

    attach_stdio_to_dev_null();
    exec_or_exit(args);
}

/// Never returns: replaces this process image via `execvp`, falling through
/// to `_exit(127)` on failure — matching the C++'s
/// `::execvp(argv[0], argv.data()); ::_exit(127);` exactly, including the
/// exit-code-127 convention for "couldn't exec" (shell convention for
/// command-not-found).
fn exec_or_exit(args: &[String]) -> ! {
    let mut cstrings = Vec::with_capacity(args.len());
    for arg in args {
        match CString::new(arg.as_str()) {
            Ok(c) => cstrings.push(c),
            // An embedded NUL can't be represented as a C string / exec argument
            // at all — same as the C++, which would silently truncate at the
            // NUL via `c_str()`, but failing outright is closer to correct.
            // SAFETY: `_exit` never returns.
            Err(_) => unsafe { libc::_exit(127) },
        }
    }
    let mut argv: Vec<*const libc::c_char> = cstrings.iter().map(|c| c.as_ptr()).collect();
    argv.push(std::ptr::null());

    // SAFETY: `argv` is a valid, NUL-terminated array of valid, live C strings
    // (owned by `cstrings`, which outlives this call). `execvp` only returns
    // on failure, per POSIX; the subsequent `_exit` never returns either.
    unsafe {
        libc::execvp(argv[0], argv.as_ptr());
        libc::_exit(127);
    }
}

/// Port of `attachStdioToDevNull`.
fn attach_stdio_to_dev_null() {
    // SAFETY: `/dev/null` always exists; `open`/`dup2`/`close` are
    // async-signal-safe and valid to call in a freshly-forked child before
    // `exec`.
    unsafe {
        let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if devnull >= 0 {
            libc::dup2(devnull, libc::STDIN_FILENO);
            libc::dup2(devnull, libc::STDOUT_FILENO);
            libc::dup2(devnull, libc::STDERR_FILENO);
            if devnull > libc::STDERR_FILENO {
                libc::close(devnull);
            }
        }
    }
}

/// Port of `writePipeOrIgnore`.
fn write_pipe_or_ignore(fd: i32, data: &[u8]) {
    let mut remaining = data;
    while !remaining.is_empty() {
        // SAFETY: `fd` is our own pipe write end, valid for this call;
        // `remaining` is a live slice of at least `remaining.len()` bytes.
        let n = unsafe { libc::write(fd, remaining.as_ptr().cast(), remaining.len()) };
        if n > 0 {
            remaining = &remaining[n as usize..];
        } else if n == 0
            || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            // A short write of 0 bytes, or any error other than EINTR (matches
            // the C++'s two separate `return;` arms for the same conditions).
            return;
        }
    }
}

fn close_pipe(pipe: &mut [i32; 2]) {
    // SAFETY: both fds were just opened by us via `libc::pipe` and aren't used again.
    unsafe {
        libc::close(pipe[0]);
        libc::close(pipe[1]);
    }
}

/// Port of the `while (::waitpid(pid, &status, 0) < 0 && errno == EINTR) {}`
/// reap loop, wrapped in `std::process::ExitStatus` so the exited/exit-code
/// check reuses std's own `WIFEXITED`/`WEXITSTATUS` logic instead of
/// reimplementing those macros by hand.
fn wait_for_exit(pid: libc::pid_t) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    let mut status = 0;
    loop {
        // SAFETY: `status` is a valid, live `c_int` for the duration of this call.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r >= 0 {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            break;
        }
    }
    std::process::ExitStatus::from_raw(status)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn command_exists_rejects_directories() {
        assert!(!command_exists("/usr/bin"));
        assert!(!command_exists("/"));
        assert!(command_exists("true"));
        assert!(!command_exists(""));
        assert!(!command_exists("/nonexistent"));
    }

    #[test]
    fn detached_async_inherits_launch_environment() {
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let out_path =
            std::env::temp_dir().join(format!("noctalia_process_env_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&out_path);

        // SAFETY: guarded by ENV_MUTATION_LOCK above; see the module doc
        // comment for why a raw fork() (unlike Command::spawn) needs this.
        unsafe {
            std::env::set_var(
                "NOCTALIA_WALLPAPER_PATH",
                "/tmp/noctalia test/wallpaper.png",
            );
            std::env::set_var("NOCTALIA_WALLPAPER_CONNECTOR", "DP-1");
        }

        let command = format!(
            r#"printf '%s\n%s' "$NOCTALIA_WALLPAPER_PATH" "$NOCTALIA_WALLPAPER_CONNECTOR" > {}"#,
            shell_quote(out_path.to_str().expect("temp path must be UTF-8"))
        );
        let launched = launch_detached_shell(&command);

        // SAFETY: as above.
        unsafe {
            std::env::remove_var("NOCTALIA_WALLPAPER_PATH");
            std::env::remove_var("NOCTALIA_WALLPAPER_CONNECTOR");
        }

        assert!(launched, "detached async env command did not launch");

        let expected = "/tmp/noctalia test/wallpaper.png\nDP-1";
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut contents = String::new();
        while Instant::now() < deadline {
            if let Ok(read) = std::fs::read_to_string(&out_path) {
                contents = read;
                if contents == expected {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let _ = std::fs::remove_file(&out_path);
        assert_eq!(contents, expected);
    }

    #[test]
    fn launch_detached_tracked_reports_a_terminable_pid() {
        // Not an env mutation itself, but this test does trigger a raw
        // `fork()`, which must stay serialized against any concurrent
        // `set_var`/`remove_var` elsewhere in this binary — see the module
        // doc comment.
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pid = launch_detached_tracked(&["sleep".to_string(), "5".to_string()])
            .expect("expected a tracked pid");
        assert!(pid > 0);

        terminate_tracked(pid);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut gone = false;
        while Instant::now() < deadline {
            // SAFETY: signal 0 sends nothing; it only probes whether `pid`
            // still exists and is signalable, matching the standard
            // liveness-check idiom.
            let probe = unsafe { libc::kill(pid as libc::pid_t, 0) };
            if probe != 0 {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(gone, "terminate_tracked did not terminate the process");
    }

    #[test]
    fn launch_detached_tracked_rejects_empty_args() {
        assert!(launch_detached_tracked(&[]).is_none());
        assert!(launch_detached_tracked(&[String::new()]).is_none());
    }

    #[test]
    fn terminate_tracked_ignores_non_positive_pids() {
        terminate_tracked(0);
        terminate_tracked(-1);
    }

    #[test]
    fn launch_first_available_picks_the_first_existing_command() {
        // This test's second variant actually runs and forks; see the
        // module doc comment for why that needs excluding concurrent env
        // mutation elsewhere in this binary.
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let out_path = std::env::temp_dir().join(format!(
            "noctalia_first_available_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_path);
        let out_arg = out_path.to_str().expect("temp path must be UTF-8");

        let launched =
            launch_first_available(&[&["definitely-not-a-real-command-xyz"], &["touch", out_arg]]);
        assert!(launched);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut exists = false;
        while Instant::now() < deadline {
            if out_path.exists() {
                exists = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&out_path);
        assert!(
            exists,
            "launch_first_available did not run the second variant"
        );
    }

    #[test]
    fn launch_first_available_returns_false_when_nothing_matches() {
        assert!(!launch_first_available(&[&[
            "definitely-not-a-real-command-xyz"
        ]]));
        assert!(!launch_first_available(&[&[]]));
    }

    #[test]
    fn resolve_privilege_escalator_prefers_run0_over_pkexec() {
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile_dir("noctalia_escalator_test");
        write_fake_executable(&dir, "run0");
        write_fake_executable(&dir, "pkexec");

        // Replace PATH entirely rather than prepending: this host's real
        // PATH already has a genuine `run0` (systemd 256+, shipped on this
        // NixOS), which would make the fallback test below pass for the
        // wrong reason (finding the real `run0`) if the real PATH stayed
        // reachable.
        let restored = replace_path(&dir);
        let result = resolve_privilege_escalator();
        restore_path(restored);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(result.as_deref(), Some("run0"));
    }

    #[test]
    fn resolve_privilege_escalator_falls_back_to_pkexec() {
        let _env_guard = crate::process::test_support::ENV_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile_dir("noctalia_escalator_fallback_test");
        write_fake_executable(&dir, "pkexec");

        // See the comment in the test above: must not leave the real PATH
        // (and its real `run0`) reachable, or this would spuriously resolve
        // to the real `run0` instead of exercising the pkexec fallback.
        let restored = replace_path(&dir);
        let result = resolve_privilege_escalator();
        restore_path(restored);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(result.as_deref(), Some("pkexec"));
    }

    fn tempfile_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn write_fake_executable(dir: &Path, name: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("failed to write fake executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("failed to chmod fake executable");
    }

    /// Mutates process-global `PATH`; callers must hold `ENV_MUTATION_LOCK`
    /// for the duration (both call sites do).
    fn replace_path(dir: &Path) -> Option<std::ffi::OsString> {
        let original = std::env::var_os("PATH");
        // SAFETY: guarded by ENV_MUTATION_LOCK in the calling test.
        unsafe {
            std::env::set_var("PATH", dir.as_os_str());
        }
        original
    }

    /// Same locking requirement as [`replace_path`].
    fn restore_path(original: Option<std::ffi::OsString>) {
        // SAFETY: as above.
        unsafe {
            match original {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    fn shell_quote(value: &str) -> String {
        let mut quoted = String::from("'");
        for ch in value.chars() {
            if ch == '\'' {
                quoted.push_str("'\\''");
            } else {
                quoted.push(ch);
            }
        }
        quoted.push('\'');
        quoted
    }
}
