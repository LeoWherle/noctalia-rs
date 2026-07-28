//! Port of `src/core/process/process_fds.{cpp,h}` (task 1.6.6): `raiseOpenFileLimit`
//! (raises the soft `RLIMIT_NOFILE` toward the hard limit — the default 1024 soft
//! cap is far too low for a long-running GPU client, since the NVIDIA EGL/Wayland
//! driver accumulates internal `sync_file` fences over a session and exhausting the
//! soft limit makes the Wayland connection fail fatally) and
//! `describeOpenFileDescriptors` (a `/proc/self/fd` census, bucketed by target kind,
//! for diagnosing exactly that kind of exhaustion when it happens).
//!
//! No C++ test exists for either function (confirmed: zero references to
//! `ProcessFds`/`raiseOpenFileLimit`/`describeOpenFileDescriptors` in any
//! `tests/*_test.cpp`) — the task's own bar is unit tests for the FD-target
//! bucketing plus a smoke test that `raise_open_file_limit` never lowers the
//! soft limit and `describe_open_file_descriptors`'s output is well-formed,
//! which is what the test module here does.
//!
//! Divergences from the C++, recorded per rule 6 (even minor/unreachable ones):
//!
//! - Error messages use `std::io::Error`'s `Display` (e.g. `"No such file or
//!   directory (os error 2)"`) rather than bare `strerror(errno)` (`"No such
//!   file or directory"`). Both are diagnostic-only strings with no C++ test
//!   asserting their exact contents; the Rust form is a strict superset of
//!   information (same message, plus the numeric errno), not a behavior
//!   change.
//! - `bucket_target`'s long-path truncation clamps to the nearest UTF-8 char
//!   boundary at or before 117 bytes, not exactly byte 117: the C++'s
//!   `std::string` is a raw byte buffer that can truncate mid-multi-byte
//!   sequence with no validity requirement, but a Rust `String` must stay
//!   valid UTF-8. `/proc/self/fd/*` targets are real filesystem paths or
//!   kernel-synthesized pseudo-paths (`socket:[...]`, `pipe:[...]`, etc.),
//!   overwhelmingly ASCII in practice, so this is not reachable in practice —
//!   and even when it is, the result is at most 2 bytes shorter than the
//!   C++'s, not a different bucket or a corrupted count.
//! - `read_fd_target` lossily converts the raw `readlink` bytes to UTF-8
//!   (`String::from_utf8_lossy`) instead of storing them as opaque bytes like
//!   the C++'s `std::string`. Unlike `process::matching`'s `/proc/<pid>/cmdline`
//!   scanning (where byte fidelity is load-bearing because needles are
//!   substring-matched against raw bytes), this output is display-only
//!   diagnostic text (logged, or appended to an error message) — never
//!   compared or matched against anything — so a lossy display conversion
//!   changes nothing observable for the well-formed-real-path case, and only
//!   substitutes U+FFFD for genuinely invalid bytes in the pathological case.

use std::collections::HashMap;
use std::ffi::CStr;

use crate::log::Logger;

static LOG: Logger = Logger::new("fdlimit");

/// Default `maxTargets` from the C++'s `describeOpenFileDescriptors(std::size_t
/// maxTargets = 8)` — Rust has no default arguments, so `describe_open_file_descriptors`
/// below is the zero-arg convenience call and `describe_open_file_descriptors_with_max_targets`
/// is the explicit-parameter form, mirroring the `run_sync`/`run_sync_with_options`
/// naming convention already used elsewhere in `process::*`.
const DEFAULT_MAX_FD_TARGETS: usize = 8;

/// Longest single readlink target buffer this will grow to before giving up
/// (matches the C++'s `buffer.size() > 8192U` cap).
const MAX_FD_TARGET_BUFFER: usize = 8192;

fn is_fd_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

fn rlimit_value(value: libc::rlim_t) -> String {
    if value == libc::RLIM_INFINITY {
        "infinity".to_string()
    } else {
        value.to_string()
    }
}

fn rlimit_summary() -> String {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, correctly-sized `rlimit` for `getrlimit` to
    // populate.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return "rlimit_nofile=unavailable".to_string();
    }
    format!(
        "rlimit_nofile={}/{}",
        rlimit_value(limit.rlim_cur),
        rlimit_value(limit.rlim_max)
    )
}

/// Clamps `s` to at most `max_bytes` bytes, moving down to the nearest UTF-8
/// char boundary if `max_bytes` itself would split a multi-byte sequence. See
/// the module doc comment's truncation divergence note for why this can't
/// just be `String::truncate` at an arbitrary byte offset like the C++'s
/// `std::string::resize`.
fn truncate_to_char_boundary(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Port of `bucketTarget`.
fn bucket_target(mut target: String) -> String {
    if target.starts_with("socket:") {
        return "socket".to_string();
    }
    if target.starts_with("pipe:") {
        return "pipe".to_string();
    }
    if target.starts_with("memfd:") || target.starts_with("/memfd:") {
        return "memfd".to_string();
    }
    if target.starts_with("anon_inode:") {
        return target;
    }
    if target.len() > 120 {
        truncate_to_char_boundary(&mut target, 117);
        target.push_str("...");
    }
    target
}

/// Port of `readFdTarget`: resolves `/proc/self/fd/<fd_name>` via `readlink`,
/// growing the buffer and retrying while the target might have been
/// truncated, matching the C++'s `buffer.resize(buffer.size() * 2U)` loop and
/// its `> 8192` giveup cap.
fn read_fd_target(fd_name: &str) -> String {
    let path = format!("/proc/self/fd/{fd_name}");
    let Ok(c_path) = std::ffi::CString::new(path) else {
        // Unreachable in practice: `fd_name` is validated by `is_fd_name` to
        // be non-empty ASCII digits before this is ever called, so it can
        // never contain an interior NUL.
        return "readlink failed: invalid path".to_string();
    };

    let mut buffer_len: usize = 512;
    loop {
        let mut buffer = vec![0u8; buffer_len];
        // SAFETY: `c_path` is a valid, NUL-terminated C string for the
        // duration of this call; `buffer` is a valid, writable buffer of at
        // least `buffer_len - 1` bytes, matching the C++'s
        // `buffer.size() - 1` (which likewise always leaves room for a
        // trailing NUL the C++ appends itself — here we simply truncate
        // instead of NUL-terminating, since we return an owned `String`).
        let n = unsafe {
            libc::readlink(
                c_path.as_ptr(),
                buffer.as_mut_ptr().cast::<libc::c_char>(),
                buffer_len - 1,
            )
        };
        if n < 0 {
            return format!("readlink failed: {}", std::io::Error::last_os_error());
        }
        let n = n as usize;
        if n < buffer_len - 1 {
            buffer.truncate(n);
            return String::from_utf8_lossy(&buffer).into_owned();
        }
        buffer_len = buffer_len.saturating_mul(2);
        if buffer_len > MAX_FD_TARGET_BUFFER {
            return "<fd target too long>".to_string();
        }
    }
}

/// Port of `ProcessFds::raiseOpenFileLimit`: raises the soft `RLIMIT_NOFILE`
/// toward the hard limit, logging the outcome (never the reverse — a no-op
/// once already at the hard limit, and left untouched on any `getrlimit`/
/// `setrlimit` failure).
pub fn raise_open_file_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, correctly-sized `rlimit` for `getrlimit` to
    // populate.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        LOG.error(format_args!(
            "RLIMIT_NOFILE getrlimit failed: {}",
            std::io::Error::last_os_error()
        ));
        return;
    }

    let previous = limit.rlim_cur;
    if limit.rlim_cur >= limit.rlim_max {
        LOG.info(format_args!(
            "RLIMIT_NOFILE already at hard limit ({})",
            rlimit_value(previous)
        ));
        return;
    }

    limit.rlim_cur = limit.rlim_max;
    // SAFETY: `limit` holds a valid `rlimit` with `rlim_cur` raised to
    // `rlim_max`; raising one's own soft limit to the existing hard limit
    // never requires elevated privilege.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
        LOG.error(format_args!(
            "RLIMIT_NOFILE setrlimit to {} failed: {} (soft limit stays {})",
            rlimit_value(limit.rlim_max),
            std::io::Error::last_os_error(),
            rlimit_value(previous)
        ));
        return;
    }

    LOG.info(format_args!(
        "RLIMIT_NOFILE soft limit raised {} -> {}",
        rlimit_value(previous),
        rlimit_value(limit.rlim_max)
    ));
}

/// Port of `ProcessFds::describeOpenFileDescriptors(maxTargets = 8)`.
pub fn describe_open_file_descriptors() -> String {
    describe_open_file_descriptors_with_max_targets(DEFAULT_MAX_FD_TARGETS)
}

/// Port of `ProcessFds::describeOpenFileDescriptors(maxTargets)`: counts open
/// file descriptors under `/proc/self/fd`, bucketed by target kind, and
/// reports the `max_targets` most common buckets alongside the process's
/// `RLIMIT_NOFILE`.
pub fn describe_open_file_descriptors_with_max_targets(max_targets: usize) -> String {
    let limit = rlimit_summary();

    // SAFETY: `c"/proc/self/fd"` is a valid, static, NUL-terminated C string.
    let dir = unsafe { libc::opendir(c"/proc/self/fd".as_ptr()) };
    if dir.is_null() {
        return format!(
            "open_fds=unavailable (opendir /proc/self/fd failed: {}), {limit}",
            std::io::Error::last_os_error()
        );
    }

    // SAFETY: `dir` was just confirmed non-null and opened successfully above.
    let directory_fd = unsafe { libc::dirfd(dir) };

    let mut count: usize = 0;
    let mut target_counts: HashMap<String, usize> = HashMap::new();
    loop {
        // SAFETY: `dir` is a live, valid `DIR*` for the duration of this loop,
        // not concurrently accessed by any other thread.
        let entry = unsafe { libc::readdir(dir) };
        if entry.is_null() {
            break;
        }
        // SAFETY: `entry` was just confirmed non-null; `d_name` is a
        // NUL-terminated array populated by `readdir` for the lifetime of
        // this iteration (before the next `readdir`/`closedir` call).
        let name_cstr = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let Ok(name) = name_cstr.to_str() else {
            continue;
        };
        if !is_fd_name(name) {
            continue;
        }
        if let Ok(fd_number) = name.parse::<i32>()
            && fd_number == directory_fd
        {
            continue;
        }
        count += 1;
        let bucket = bucket_target(read_fd_target(name));
        *target_counts.entry(bucket).or_insert(0) += 1;
    }
    // SAFETY: `dir` was successfully opened above and hasn't been closed yet.
    unsafe {
        libc::closedir(dir);
    }

    let mut targets: Vec<(String, usize)> = target_counts.into_iter().collect();
    targets.sort_by(|lhs, rhs| rhs.1.cmp(&lhs.1).then_with(|| lhs.0.cmp(&rhs.0)));

    let mut out = format!("open_fds={count}, {limit}");
    if !targets.is_empty() && max_targets > 0 {
        out.push_str(", top_fd_targets=[");
        for (i, (target, target_count)) in targets.iter().take(max_targets).enumerate() {
            if i != 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{target}={target_count}"));
        }
        out.push(']');
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn is_fd_name_accepts_only_nonempty_digits() {
        assert!(is_fd_name("0"));
        assert!(is_fd_name("42"));
        assert!(!is_fd_name(""));
        assert!(!is_fd_name("4a"));
        assert!(!is_fd_name("-1"));
    }

    #[test]
    fn rlimit_value_formats_infinity_and_finite() {
        assert_eq!(rlimit_value(libc::RLIM_INFINITY), "infinity");
        assert_eq!(rlimit_value(1024), "1024");
    }

    #[test]
    fn bucket_target_recognizes_known_prefixes() {
        assert_eq!(bucket_target("socket:[12345]".to_string()), "socket");
        assert_eq!(bucket_target("pipe:[12345]".to_string()), "pipe");
        assert_eq!(bucket_target("memfd:foo".to_string()), "memfd");
        assert_eq!(bucket_target("/memfd:foo (deleted)".to_string()), "memfd");
        assert_eq!(
            bucket_target("anon_inode:[eventfd]".to_string()),
            "anon_inode:[eventfd]"
        );
    }

    #[test]
    fn bucket_target_passes_through_short_real_paths_unchanged() {
        assert_eq!(
            bucket_target("/usr/lib/libfoo.so.1".to_string()),
            "/usr/lib/libfoo.so.1"
        );
    }

    #[test]
    fn bucket_target_truncates_long_paths() {
        let long_path = format!("/{}", "a".repeat(200));
        let bucketed = bucket_target(long_path);
        assert_eq!(bucketed.len(), 120, "117 kept bytes + \"...\"");
        assert!(bucketed.ends_with("..."));
    }

    #[test]
    fn truncate_to_char_boundary_never_panics_on_multibyte_input() {
        // A run of 2-byte UTF-8 characters (U+00C9 LATIN CAPITAL LETTER E WITH
        // ACUTE) whose char boundaries only ever fall on even byte offsets —
        // since 117 is odd, byte 117 is guaranteed to split a character,
        // genuinely exercising the boundary-search decrement loop (unlike a
        // 3-byte character, whose width evenly divides 117 and would make
        // byte 117 already a boundary without ever decrementing).
        let mut s = "É".repeat(60);
        let original_len = s.len();
        assert!(
            !s.is_char_boundary(117),
            "test setup should put a multi-byte char across byte 117"
        );
        truncate_to_char_boundary(&mut s, 117);
        assert!(s.len() <= 117);
        assert!(s.len() < original_len);
        assert!(s.is_char_boundary(s.len()));
    }

    #[test]
    fn raise_open_file_limit_never_lowers_the_soft_limit() {
        let before = {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: `limit` is a valid, correctly-sized `rlimit` for
            // `getrlimit` to populate.
            let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
            assert_eq!(rc, 0, "getrlimit should succeed in a test process");
            limit.rlim_cur
        };

        raise_open_file_limit();

        let after = {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: as above.
            let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
            assert_eq!(rc, 0, "getrlimit should succeed in a test process");
            limit.rlim_cur
        };

        assert!(
            after >= before,
            "soft RLIMIT_NOFILE should never be lowered: {before} -> {after}"
        );
    }

    #[test]
    fn describe_open_file_descriptors_output_is_well_formed() {
        let described = describe_open_file_descriptors();
        assert!(described.starts_with("open_fds="), "{described}");
        assert!(described.contains("rlimit_nofile="), "{described}");
    }

    #[test]
    fn describe_open_file_descriptors_with_max_targets_zero_omits_the_section() {
        let described = describe_open_file_descriptors_with_max_targets(0);
        assert!(!described.contains("top_fd_targets"), "{described}");
    }

    #[test]
    fn describe_open_file_descriptors_counts_a_freshly_opened_pipe() {
        let mut fds: [i32; 2] = [0, 0];
        // SAFETY: `fds` is a valid, writable `[i32; 2]` for `pipe` to
        // populate with a fresh read/write fd pair.
        let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert_eq!(rc, 0, "failed to open a test pipe");

        let described = describe_open_file_descriptors_with_max_targets(20);

        // SAFETY: both fds were just opened above and aren't used again.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }

        assert!(
            described.contains("pipe="),
            "expected the freshly opened pipe to show up in the pipe bucket: {described}"
        );
    }
}
