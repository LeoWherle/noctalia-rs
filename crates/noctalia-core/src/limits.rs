//! Port of `config/config_limits.h` (task 2.1.1): config-level clamp
//! constants for the clipboard history size. Unrelated to `color` in
//! content, but the whole C++ header is these four constants and 2.1.1's
//! plan text claims it wholesale; placed at the crate root here (not under
//! `color`) since nothing else in that header is color-related. Consumed by
//! later tasks (2.x's clipboard config struct, 10.5's clipboard service,
//! 14.4's clipboard manager UI) which don't exist yet — pulled forward now
//! so those tasks don't need to re-derive them from the C++ separately.

/// Port of `kClipboardHistoryMinEntries` (config_limits.h:7).
pub const CLIPBOARD_HISTORY_MIN_ENTRIES: i64 = 10;
/// Port of `kClipboardHistoryDefaultEntries` (config_limits.h:8).
pub const CLIPBOARD_HISTORY_DEFAULT_ENTRIES: i64 = 100;
/// Port of `kClipboardHistoryMaxEntries` (config_limits.h:9).
pub const CLIPBOARD_HISTORY_MAX_ENTRIES: i64 = 10000;
/// Port of `kClipboardHistoryStepEntries` (config_limits.h:10).
pub const CLIPBOARD_HISTORY_STEP_ENTRIES: i64 = 10;

// These relationships hold for fixed constants, so check them at compile time
// rather than as a `#[test]` (which clippy flags as an assertion on
// constants).
const _: () = assert!(CLIPBOARD_HISTORY_DEFAULT_ENTRIES >= CLIPBOARD_HISTORY_MIN_ENTRIES);
const _: () = assert!(CLIPBOARD_HISTORY_DEFAULT_ENTRIES <= CLIPBOARD_HISTORY_MAX_ENTRIES);
const _: () = assert!(CLIPBOARD_HISTORY_DEFAULT_ENTRIES % CLIPBOARD_HISTORY_STEP_ENTRIES == 0);
