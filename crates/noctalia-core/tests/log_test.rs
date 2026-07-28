//! Port of `tests/log_test.cpp`. Kept as a single test (rather than one-per-case)
//! because the checks mutate process-global state — env vars and the log sink's
//! shared level/file — exactly as the C++ test's single-threaded `main()` does; the
//! sequencing between cases matters.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use noctalia_core::log::{
    LogLevel, current_log_level, init_log_file, init_log_level_from_environment,
    install_tracing_bridge, log_level_name, log_warn, parse_log_level, set_log_level,
};

const MAX_LOG_BYTES: u64 = 1024 * 1024;
const MAX_LOG_LINE_BYTES: u64 = 8 * 1024;

#[test]
fn log_behaves_like_cpp_reference() {
    set_log_level(LogLevel::Error);

    parses_log_levels();
    reads_log_level_environment();
    writes_capped_log_lines();
    rotates_while_running();
    tracing_bridge_forwards_events_to_the_same_sink();
}

fn parses_log_levels() {
    assert_eq!(parse_log_level("debug"), Some(LogLevel::Debug));
    assert_eq!(parse_log_level("info"), Some(LogLevel::Info));
    assert_eq!(parse_log_level("warn"), Some(LogLevel::Warn));
    assert_eq!(parse_log_level("error"), Some(LogLevel::Error));
    assert_eq!(
        parse_log_level("WRN"),
        None,
        "non-canonical log level was accepted"
    );
    assert_eq!(log_level_name(LogLevel::Warn), "warn");
}

fn reads_log_level_environment() {
    set_log_level(LogLevel::Info);
    set_env("NOCTALIA_LOG_LEVEL", "warn");
    init_log_level_from_environment();
    assert_eq!(
        current_log_level(),
        LogLevel::Warn,
        "environment log level was not applied"
    );

    set_log_level(LogLevel::Error);
    set_env("NOCTALIA_LOG_LEVEL", "WRN");
    init_log_level_from_environment();
    assert_eq!(
        current_log_level(),
        LogLevel::Error,
        "invalid environment log level changed current level"
    );
    remove_env("NOCTALIA_LOG_LEVEL");
}

fn writes_capped_log_lines() {
    let cache_root = make_temp_root("noctalia-log-cap");
    use_cache_home(&cache_root);

    init_log_file();
    log_warn(format_args!("{}", "x".repeat(10_000)));

    let log_path = cache_root.join("noctalia").join("noctalia.log");
    let size = fs::metadata(&log_path)
        .expect("failed to stat capped log file")
        .len();
    assert!(size <= MAX_LOG_LINE_BYTES, "capped log line exceeded 8 KiB");

    let log = fs::read_to_string(&log_path).expect("failed to read capped log file");
    assert!(
        log.contains("truncated, original=10000 bytes"),
        "missing truncation marker"
    );

    let _ = fs::remove_dir_all(&cache_root);
}

fn rotates_while_running() {
    let cache_root = make_temp_root("noctalia-log-rotate");
    use_cache_home(&cache_root);

    let log_dir = cache_root.join("noctalia");
    fs::create_dir_all(&log_dir).expect("failed to create log dir");

    let log_path = log_dir.join("noctalia.log");
    let initial_size = MAX_LOG_BYTES - 8;
    {
        let chunk = "a".repeat(4096);
        let mut contents = String::with_capacity(initial_size as usize);
        let mut written = 0u64;
        while written + chunk.len() as u64 <= initial_size {
            contents.push_str(&chunk);
            written += chunk.len() as u64;
        }
        contents.push_str(&"a".repeat((initial_size - written) as usize));
        fs::write(&log_path, &contents).expect("failed to seed log file");
    }

    init_log_file();
    log_warn(format_args!("rotate-now"));

    let backup_path = log_dir.join("noctalia.log.1");
    let backup_size = fs::metadata(&backup_path)
        .expect("missing rotated backup")
        .len();
    assert_eq!(
        backup_size, initial_size,
        "rotated backup size did not match original log"
    );

    let current_log = fs::read_to_string(&log_path).expect("failed to read post-rotation log");
    assert!(
        current_log.contains("rotate-now"),
        "new log did not receive post-rotation line"
    );
    assert!(
        current_log.len() as u64 <= MAX_LOG_LINE_BYTES,
        "post-rotation log line exceeded 8 KiB"
    );

    let _ = fs::remove_dir_all(&cache_root);
}

fn tracing_bridge_forwards_events_to_the_same_sink() {
    let cache_root = make_temp_root("noctalia-log-bridge");
    use_cache_home(&cache_root);

    init_log_file();
    install_tracing_bridge();
    tracing::error!(target: "bridge-section", "hello from bridge");

    let log_path = cache_root.join("noctalia").join("noctalia.log");
    let log = fs::read_to_string(&log_path).expect("failed to read bridged log file");
    assert!(
        log.contains("[bridge-section]"),
        "bridged event missing its section tag"
    );
    assert!(
        log.contains("hello from bridge"),
        "bridged event message missing"
    );

    let _ = fs::remove_dir_all(&cache_root);
}

fn use_cache_home(path: &Path) {
    set_env(
        "XDG_CACHE_HOME",
        path.to_str().expect("temp path was not valid UTF-8"),
    );
}

fn set_env(key: &str, value: &str) {
    // SAFETY: this test binary is single-threaded (one #[test] fn, run sequentially);
    // no other thread observes the environment concurrently.
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_env(key: &str) {
    // SAFETY: see `set_env` above.
    unsafe {
        std::env::remove_var(key);
    }
}

fn make_temp_root(label: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("failed to create temp root");
    path
}
