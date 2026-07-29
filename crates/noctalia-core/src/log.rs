//! Port of `src/core/log.{cpp,h}`.
//!
//! `log_message` is the single sink: console output (ANSI, time-only, filtered by the
//! current level) and file output (plain, full date, unfiltered, size-capped with
//! rotation) are both derived from it, exactly mirroring the C++ implementation. A
//! [`tracing_subscriber::Layer`] bridges ordinary `tracing::{debug,info,warn,error}!`
//! calls from anywhere in the process into the same sink, using the event's target as
//! the section — that satisfies the "on tracing" architecture decision for modules
//! ported later without forcing [`Logger`]'s per-instance section (a runtime value)
//! through tracing's static-callsite `target` machinery, which requires a
//! const-evaluable expression and so cannot accept it.

use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::{Event, Level as TracingLevel, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt as _;

const LOG_LEVEL_ENV: &str = "NOCTALIA_LOG_LEVEL";
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const MAX_LOG_LINE_BYTES: usize = 8 * 1024;
const BUFFERED_FILE_LOG_FLUSH_LINES: usize = 64;
const BUFFERED_FILE_LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const SHORT_TRUNCATION_SUFFIX: &str = " ... [truncated]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

fn decode_level(value: u8) -> LogLevel {
    match value {
        0 => LogLevel::Debug,
        1 => LogLevel::Info,
        2 => LogLevel::Warn,
        _ => LogLevel::Error,
    }
}

pub fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

pub fn parse_log_level(value: &str) -> Option<LogLevel> {
    match value {
        "debug" => Some(LogLevel::Debug),
        "info" => Some(LogLevel::Info),
        "warn" => Some(LogLevel::Warn),
        "error" => Some(LogLevel::Error),
        _ => None,
    }
}

// Divergence from C++: the C++ serializes level changes against log calls under
// the same `scoped_lock` that guards the log file/state. Here `MIN_LEVEL` is a
// bare atomic outside that lock, so a `set_log_level` racing a log call can
// observe the pre- or post-change threshold inconsistently. Benign in practice
// (worst case: one log line right at the boundary uses the "wrong" threshold)
// and not worth a lock for a value only ever read/written atomically.
static MIN_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

pub fn current_log_level() -> LogLevel {
    decode_level(MIN_LEVEL.load(Ordering::Relaxed))
}

pub fn set_log_level(level: LogLevel) {
    MIN_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn init_log_level_from_environment() {
    let Ok(value) = env::var(LOG_LEVEL_ENV) else {
        return;
    };

    match parse_log_level(&value) {
        Some(level) => {
            set_log_level(level);
            log_info(format_args!("log level set to {}", log_level_name(level)));
        }
        None => {
            log_warn(format_args!(
                "invalid {LOG_LEVEL_ENV} '{value}'; expected debug, info, warn, or error"
            ));
        }
    }
}

// ── Timestamp capture (mirrors `timespec_get` + `localtime_r`) ────────────────

struct Timestamp {
    tm: libc::tm,
    millis: i64,
}

fn capture_timestamp() -> Timestamp {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = since_epoch.as_secs() as libc::time_t;
    let millis = i64::from(since_epoch.subsec_millis());

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `secs` and `tm` are both valid, live for the call, and `localtime_r`
    // only ever reads `secs` and writes into `tm`.
    unsafe {
        libc::localtime_r(&secs, &mut tm);
    }
    Timestamp { tm, millis }
}

// ── Formatting ──────────────────────────────────────────────────────────────

fn level_tag_ansi(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "\x1b[36mDBG\x1b[0m",
        LogLevel::Info => "\x1b[32mINF\x1b[0m",
        LogLevel::Warn => "\x1b[33mWRN\x1b[0m",
        LogLevel::Error => "\x1b[31mERR\x1b[0m",
    }
}

fn level_tag_plain(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "DBG",
        LogLevel::Info => "INF",
        LogLevel::Warn => "WRN",
        LogLevel::Error => "ERR",
    }
}

fn console_prefix(ts: &Timestamp, level: LogLevel, section: Option<&str>) -> String {
    let mut prefix = format!(
        "{:02}:{:02}:{:02}.{:03} [{}]",
        ts.tm.tm_hour,
        ts.tm.tm_min,
        ts.tm.tm_sec,
        ts.millis,
        level_tag_ansi(level)
    );
    if let Some(section) = section.filter(|s| !s.is_empty()) {
        prefix.push_str(" [\x1b[34m");
        prefix.push_str(section);
        prefix.push_str("\x1b[0m]");
    }
    prefix.push(' ');
    prefix
}

fn file_prefix(ts: &Timestamp, level: LogLevel, section: Option<&str>) -> String {
    let mut prefix = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{}]",
        ts.tm.tm_year + 1900,
        ts.tm.tm_mon + 1,
        ts.tm.tm_mday,
        ts.tm.tm_hour,
        ts.tm.tm_min,
        ts.tm.tm_sec,
        ts.millis,
        level_tag_plain(level)
    );
    if let Some(section) = section.filter(|s| !s.is_empty()) {
        prefix.push_str(" [");
        prefix.push_str(section);
        prefix.push(']');
    }
    prefix.push(' ');
    prefix
}

fn truncation_suffix(original_bytes: usize) -> String {
    format!(" ... [truncated, original={original_bytes} bytes]")
}

struct CappedMessage<'a> {
    storage: String,
    original: &'a str,
    capped: bool,
}

impl CappedMessage<'_> {
    fn text(&self) -> &str {
        if self.capped {
            &self.storage
        } else {
            self.original
        }
    }
}

fn cap_message_for_line(msg: &str, prefix_bytes: usize) -> CappedMessage<'_> {
    if prefix_bytes + 1 >= MAX_LOG_LINE_BYTES {
        return CappedMessage {
            storage: String::new(),
            original: msg,
            capped: true,
        };
    }

    let max_message_bytes = MAX_LOG_LINE_BYTES - prefix_bytes - 1;
    if msg.len() <= max_message_bytes {
        return CappedMessage {
            storage: String::new(),
            original: msg,
            capped: false,
        };
    }

    let suffix = truncation_suffix(msg.len());
    if suffix.len() > max_message_bytes {
        let take = SHORT_TRUNCATION_SUFFIX.len().min(max_message_bytes);
        return CappedMessage {
            storage: SHORT_TRUNCATION_SUFFIX[..take].to_string(),
            original: msg,
            capped: true,
        };
    }

    let mut body_bytes = max_message_bytes - suffix.len();
    while body_bytes > 0 && !msg.is_char_boundary(body_bytes) {
        body_bytes -= 1;
    }

    let mut storage = String::with_capacity(body_bytes + suffix.len());
    storage.push_str(&msg[..body_bytes]);
    storage.push_str(&suffix);
    CappedMessage {
        storage,
        original: msg,
        capped: true,
    }
}

fn write_line(writer: &mut impl std::io::Write, prefix: &str, msg: &str) -> u64 {
    let mut bytes = 0u64;
    if !prefix.is_empty() && writer.write_all(prefix.as_bytes()).is_ok() {
        bytes += prefix.len() as u64;
    }
    if !msg.is_empty() && writer.write_all(msg.as_bytes()).is_ok() {
        bytes += msg.len() as u64;
    }
    if writer.write_all(b"\n").is_ok() {
        bytes += 1;
    }
    bytes
}

// ── File sink: rotation, capping, buffered flush ───────────────────────────

struct FileState {
    log_path: Option<PathBuf>,
    backup_path: Option<PathBuf>,
    file: Option<File>,
    size_bytes: u64,
    buffered_lines: usize,
    last_flush_at: Instant,
}

impl FileState {
    fn new() -> Self {
        Self {
            log_path: None,
            backup_path: None,
            file: None,
            size_bytes: 0,
            buffered_lines: 0,
            last_flush_at: Instant::now(),
        }
    }

    fn open(&mut self) {
        let Some(log_path) = self.log_path.as_ref() else {
            return;
        };
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .ok();
        self.size_bytes = match &self.file {
            Some(_) => fs::metadata(log_path).map(|m| m.len()).unwrap_or(0),
            None => 0,
        };
        self.buffered_lines = 0;
        self.last_flush_at = Instant::now();
    }

    fn close(&mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
    }

    fn rotate(&mut self) {
        self.close();
        let (Some(log_path), Some(backup_path)) = (self.log_path.clone(), self.backup_path.clone())
        else {
            return;
        };
        let _ = fs::remove_file(&backup_path);
        let _ = fs::rename(&log_path, &backup_path);
        self.open();
    }

    fn flush(&mut self) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
        self.buffered_lines = 0;
        self.last_flush_at = Instant::now();
    }

    fn should_flush(&mut self, level: LogLevel) -> bool {
        if level >= LogLevel::Warn {
            return true;
        }
        self.buffered_lines += 1;
        self.buffered_lines >= BUFFERED_FILE_LOG_FLUSH_LINES
            || self.last_flush_at.elapsed() >= BUFFERED_FILE_LOG_FLUSH_INTERVAL
    }
}

static FILE_STATE: LazyLock<Mutex<FileState>> = LazyLock::new(|| Mutex::new(FileState::new()));
static REGISTERED_EXIT_FLUSH: AtomicBool = AtomicBool::new(false);

extern "C" fn flush_log_file_at_exit() {
    let mut state = FILE_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.flush();
}

/// Opens (or rotates into) the on-disk log file under `$XDG_CACHE_HOME/noctalia` (or
/// `$HOME/.cache/noctalia`). A no-op if neither environment variable points anywhere
/// writable.
pub fn init_log_file() {
    let cache_home = env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty());
    let home = env::var_os("HOME").filter(|v| !v.is_empty());

    let dir = if let Some(cache_home) = cache_home {
        PathBuf::from(cache_home).join("noctalia")
    } else if let Some(home) = home {
        PathBuf::from(home).join(".cache").join("noctalia")
    } else {
        return;
    };

    if fs::create_dir_all(&dir).is_err() {
        return;
    }

    let log_path = dir.join("noctalia.log");
    let backup_path = dir.join("noctalia.log.1");

    let mut state = FILE_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.close();
    state.log_path = Some(log_path.clone());
    state.backup_path = Some(backup_path);

    let needs_rotate = fs::metadata(&log_path)
        .map(|m| m.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if needs_rotate {
        state.rotate();
    } else {
        state.open();
    }

    if state.file.is_some() && !REGISTERED_EXIT_FLUSH.swap(true, Ordering::SeqCst) {
        // SAFETY: `flush_log_file_at_exit` captures nothing and only touches the
        // process-global `FILE_STATE`; `libc::atexit` requires a valid `extern "C"
        // fn()` pointer, which this is.
        unsafe {
            libc::atexit(flush_log_file_at_exit);
        }
    }
}

// ── The sink itself ─────────────────────────────────────────────────────────

pub fn log_message(level: LogLevel, section: Option<&str>, msg: &str) {
    let ts = capture_timestamp();

    // One lock guards the whole call, console write included, matching the C++
    // `detail::logMessage`'s single `scoped_lock` — otherwise concurrent callers'
    // console prefix/message/newline writes (three separate `write_all`s) can
    // interleave and garble stderr, and console/file ordering can diverge across
    // threads.
    let mut state = FILE_STATE.lock().unwrap_or_else(PoisonError::into_inner);

    if level >= current_log_level() {
        let prefix = console_prefix(&ts, level, section);
        let capped = cap_message_for_line(msg, prefix.len());
        let mut stderr = std::io::stderr();
        write_line(&mut stderr, &prefix, capped.text());
    }

    if state.file.is_some() {
        let prefix = file_prefix(&ts, level, section);
        let capped = cap_message_for_line(msg, prefix.len());
        let capped_text = capped.text();
        let line_bytes = (prefix.len() + capped_text.len() + 1) as u64;
        if state.size_bytes > 0 && state.size_bytes + line_bytes > MAX_LOG_BYTES {
            state.rotate();
        }
        if let Some(file) = state.file.as_mut() {
            let written = write_line(file, &prefix, capped_text);
            state.size_bytes += written;
        }
        if state.should_flush(level) {
            state.flush();
        }
    }
}

// ── Global free functions (no section tag) ─────────────────────────────────

pub fn log_debug(args: fmt::Arguments<'_>) {
    log_message(LogLevel::Debug, None, &args.to_string());
}

pub fn log_info(args: fmt::Arguments<'_>) {
    log_message(LogLevel::Info, None, &args.to_string());
}

pub fn log_warn(args: fmt::Arguments<'_>) {
    log_message(LogLevel::Warn, None, &args.to_string());
}

pub fn log_error(args: fmt::Arguments<'_>) {
    log_message(LogLevel::Error, None, &args.to_string());
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::log::log_debug(::std::format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log::log_info(::std::format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log::log_warn(::std::format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log::log_error(::std::format_args!($($arg)*)) };
}

// ── Logger — per-module logger with a section tag ──────────────────────────

pub struct Logger {
    section: &'static str,
}

impl Logger {
    pub const fn new(section: &'static str) -> Self {
        Self { section }
    }

    pub fn debug(&self, args: fmt::Arguments<'_>) {
        log_message(LogLevel::Debug, Some(self.section), &args.to_string());
    }

    pub fn info(&self, args: fmt::Arguments<'_>) {
        log_message(LogLevel::Info, Some(self.section), &args.to_string());
    }

    pub fn warn(&self, args: fmt::Arguments<'_>) {
        log_message(LogLevel::Warn, Some(self.section), &args.to_string());
    }

    pub fn error(&self, args: fmt::Arguments<'_>) {
        log_message(LogLevel::Error, Some(self.section), &args.to_string());
    }
}

// ── tracing bridge ──────────────────────────────────────────────────────────
//
// Lets modules ported later use plain `tracing::{debug,info,warn,error}!` and land
// in the same sink, with the event's target (module path, by default) as the
// section. This is the "on tracing" half of the architecture decision; `Logger`
// and the free `log_*` functions above bypass tracing entirely because their
// section is a runtime value and tracing's macros require `target` to be
// const-evaluable (it seeds a `static` callsite).

struct BridgeLayer;

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

impl<S> Layer<S> for BridgeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = match *event.metadata().level() {
            TracingLevel::ERROR => LogLevel::Error,
            TracingLevel::WARN => LogLevel::Warn,
            TracingLevel::INFO => LogLevel::Info,
            TracingLevel::DEBUG | TracingLevel::TRACE => LogLevel::Debug,
        };
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        log_message(level, Some(event.metadata().target()), &visitor.message);
    }
}

/// Installs the global `tracing` subscriber that bridges into this module's sink.
/// A no-op if a global subscriber is already set — tests that exercise
/// `log_message` directly don't need this.
pub fn install_tracing_bridge() {
    let _ = tracing_subscriber::registry()
        .with(LevelFilter::TRACE)
        .with(BridgeLayer)
        .try_init();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_names_log_levels() {
        assert_eq!(parse_log_level("debug"), Some(LogLevel::Debug));
        assert_eq!(parse_log_level("info"), Some(LogLevel::Info));
        assert_eq!(parse_log_level("warn"), Some(LogLevel::Warn));
        assert_eq!(parse_log_level("error"), Some(LogLevel::Error));
        assert_eq!(parse_log_level("WRN"), None);
        assert_eq!(log_level_name(LogLevel::Warn), "warn");
    }

    // 2026-07-28 10:23:11.123 local time.
    fn fixed_timestamp() -> Timestamp {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = 126;
        tm.tm_mon = 6;
        tm.tm_mday = 28;
        tm.tm_hour = 10;
        tm.tm_min = 23;
        tm.tm_sec = 11;
        Timestamp { tm, millis: 123 }
    }

    #[test]
    fn console_prefix_matches_cpp_format() {
        let ts = fixed_timestamp();

        let prefix = console_prefix(&ts, LogLevel::Info, None);
        assert_eq!(prefix, "10:23:11.123 [\x1b[32mINF\x1b[0m] ");

        let prefix = console_prefix(&ts, LogLevel::Warn, Some("compositor"));
        assert_eq!(
            prefix,
            "10:23:11.123 [\x1b[33mWRN\x1b[0m] [\x1b[34mcompositor\x1b[0m] "
        );
    }

    #[test]
    fn file_prefix_matches_cpp_format() {
        let ts = fixed_timestamp();

        let prefix = file_prefix(&ts, LogLevel::Error, None);
        assert_eq!(prefix, "2026-07-28 10:23:11.123 [ERR] ");

        let prefix = file_prefix(&ts, LogLevel::Debug, Some("audio"));
        assert_eq!(prefix, "2026-07-28 10:23:11.123 [DBG] [audio] ");
    }

    #[test]
    fn caps_long_lines_with_truncation_marker() {
        let msg = "x".repeat(10_000);
        let capped = cap_message_for_line(&msg, 20);
        assert!(capped.capped);
        assert!(capped.text().len() <= MAX_LOG_LINE_BYTES - 20);
        assert!(capped.text().contains("truncated, original=10000 bytes"));
    }

    #[test]
    fn caps_respect_utf8_boundaries() {
        // 2 bytes/char: naively truncating at a byte offset would split a codepoint.
        let msg = "é".repeat(5_000);
        let capped = cap_message_for_line(&msg, 20);
        assert!(capped.capped);
        assert!(std::str::from_utf8(capped.text().as_bytes()).is_ok());
    }

    #[test]
    fn short_message_is_not_capped() {
        let capped = cap_message_for_line("hello", 20);
        assert!(!capped.capped);
        assert_eq!(capped.text(), "hello");
    }
}
