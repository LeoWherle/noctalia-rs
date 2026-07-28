//! `umask` is process-global state, so both checks below run sequentially inside
//! one #[test] fn in their own test binary — otherwise cargo's default
//! within-binary parallelism could run them concurrently and race on the mask,
//! same reasoning as the env var tests in `tests/log_test.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use noctalia_core::atomic_file::write_text_file_atomic;

#[test]
fn requested_mode_bypasses_umask_but_default_mode_does_not() {
    let dir = make_temp_dir("noctalia-atomic-file-umask");

    // SAFETY: single-threaded test binary, one #[test] fn; umask is restored
    // before returning.
    let previous = unsafe { libc::umask(0o077) };

    let explicit_target = dir.join("secret.toml");
    let explicit_result = write_text_file_atomic(&explicit_target, "secret", Some(0o600));

    let default_target = dir.join("plain.toml");
    let default_result = write_text_file_atomic(&default_target, "plain", None);

    // SAFETY: restoring the umask this process/thread had before the test.
    unsafe {
        libc::umask(previous);
    }

    explicit_result.expect("explicit-mode write failed");
    default_result.expect("default-mode write failed");

    let explicit_mode = fs::metadata(&explicit_target)
        .expect("failed to stat file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        explicit_mode, 0o600,
        "explicit mode should be forced past umask via fchmod, matching C++"
    );

    let default_mode = fs::metadata(&default_target)
        .expect("failed to stat file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        default_mode, 0o600,
        "with no explicit mode requested, the C++ never fchmods, so umask still applies"
    );

    let _ = fs::remove_dir_all(&dir);
}

fn make_temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("failed to create temp dir");
    path
}
