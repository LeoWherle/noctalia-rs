//! Port of `src/time/*` — task 1.9.
//!
//! Five C++ source files map to four submodules plus this shared root:
//! `time_format.{cpp,h}` splits across [`strftime`] (the raw `libc::strftime`
//! engine plus the `{: ... }`-brace compat bridge that every other function
//! sits on top of), [`locale`] (`nl_langinfo`-backed `formatCurrentDate`/
//! `localeFirstDayOfWeek`), [`zone`] (the two functions that genuinely need
//! an IANA timezone database: `isValidTimezone`/`formatTimezoneTime`), and
//! [`relative`] (`formatTimeAgo`/`formatElapsedSince`/`formatDuration`/
//! `formatClockTime`); `time_service.{cpp,h}` → [`service`]. `time_poll_source.h`
//! has no port here, for the same reason `FileWatchPollSource` had none in
//! task 1.2: it's pure `PollSource` glue with zero independent logic
//! (`pollTimeoutMs()`/`dispatch()` just forward to `TimeService`) — it
//! belongs with the calloop main-loop wiring, whenever that's assembled.
//!
//! **Design departure, load-bearing enough to record here rather than in a
//! per-function comment:** the C++ genuinely runs two independent formatting
//! engines side by side — a raw `strftime`-compatible path
//! (`formatStrftimeCompat`/`formatStrftimeRaw`, used by every real call
//! site, confirmed by grepping every caller in `src/`) and a `std::vformat`
//! chrono-formatter fallback that's only reachable for a format string with
//! no `%` in it at all (unreachable by every real caller). This port only
//! implements the first engine, built on `libc::strftime` directly rather
//! than `jiff`'s own `strtime` formatter: real call sites depend on
//! locale-aware weekday/month names (e.g. `widget_config.cpp`'s default
//! clock format is `"{:%a %d %b}"`), which `libc::strftime` resolves from
//! the process's `LC_TIME` locale exactly like the C++ does — `jiff`'s
//! `strtime` module always renders English names, which would silently
//! diverge for any non-English locale. `jiff` is reserved for [`zone`], the
//! one place this crate genuinely needs a real IANA timezone database
//! (arbitrary-zone-name lookup with correct historical DST rules) rather
//! than pure calendar math. A format string with no `%` at all now falls
//! back to being returned unchanged instead of C++'s full locale-default
//! chrono rendering. Every hardcoded format string at a real call site in
//! `src/` contains `%` (verified by grep), so this is unreached there; it's
//! reachable in principle through a *user-configured* format (e.g. the
//! clock widget's `format` setting) with no `%` and no unescaped `{`
//! either — that narrow case degrades the same way in both engines anyway
//! (both just return the string unchanged), so it isn't a real divergence.
//!
//! `%s` (Unix epoch seconds) is still resolved by the same manual
//! chunk-and-substitute scan the C++ uses (`formatStrftimeWithUnixSeconds`)
//! rather than trusting `libc::strftime`'s own `%s` (whose correctness
//! depends on `tm_gmtoff` being set correctly, which the C++ doesn't trust
//! either) — a faithful port, not a new mechanism.

mod locale;
mod relative;
mod service;
mod strftime;
mod zone;

use std::time::{SystemTime, UNIX_EPOCH};

pub use locale::{format_current_date, locale_first_day_of_week};
pub use relative::{format_clock_time, format_duration, format_elapsed_since, format_time_ago};
pub use service::TimeService;
pub use strftime::format_strftime;
pub use zone::{format_timezone_time, is_valid_timezone};

/// Formats current local time with a `libc::strftime`-family pattern (bare
/// `"%H:%M"` or brace-wrapped `"{:%H:%M}"`). Port of `formatLocalTime`.
pub fn format_local_time(fmt: &str) -> String {
    let normalized = strftime::normalize_format_escapes(fmt);
    let now = SystemTime::now();
    let unix_seconds = system_time_to_unix_seconds(now);
    let tm = local_tm(unix_seconds);
    strftime::format_strftime_compat(&normalized, &tm, Some(unix_seconds)).unwrap_or(normalized)
}

/// Formats a Unix timestamp in local time. Port of `formatLocalUnixTime`.
pub fn format_local_unix_time(unix_seconds: i64, fmt: &str) -> String {
    let normalized = strftime::normalize_format_escapes(fmt);
    let tm = local_tm(unix_seconds);
    strftime::format_strftime_compat(&normalized, &tm, Some(unix_seconds)).unwrap_or(normalized)
}

/// Formats an ISO 8601 prefix (`"YYYY-MM-DDTHH:MM"`) using the given format.
/// Port of `formatIsoTime`.
pub fn format_iso_time(iso_time: &str, fmt: &str) -> String {
    let Some((year, month, day, hour, minute)) = parse_iso_prefix(iso_time) else {
        return iso_time.to_string();
    };

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month - 1;
    tm.tm_mday = day;
    tm.tm_hour = hour;
    tm.tm_min = minute;
    tm.tm_isdst = -1;
    // SAFETY: `tm` is a valid, live `libc::tm`; `mktime` only reads/writes
    // through this pointer. `mktime` also normalizes `tm`'s other fields
    // (wday/yday/isdst) in place — the C++ relies on this same mutation
    // before handing the (now-normalized) `tm` to the formatter below.
    let unix_seconds = unsafe { libc::mktime(&mut tm) };
    let unix_seconds = (unix_seconds != -1).then_some(unix_seconds);

    let normalized = strftime::normalize_format_escapes(fmt);
    strftime::format_strftime_compat(&normalized, &tm, unix_seconds).unwrap_or(normalized)
}

/// Formats a `SystemTime` as UTC (gmtime) with `strftime` semantics, e.g.
/// `"%Y-%m-%dT%H:%M:%SZ"`. Port of `formatUtcTime`.
pub fn format_utc_time(tp: SystemTime, fmt: &str) -> String {
    let unix_seconds = system_time_to_unix_seconds(tp);
    let secs: libc::time_t = unix_seconds as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `secs` and `tm` are both valid and live for the call;
    // `gmtime_r` only reads `secs` and writes into `tm`.
    unsafe { libc::gmtime_r(&secs, &mut tm) };
    strftime::format_strftime_with_unix_seconds(fmt, &tm, Some(unix_seconds))
}

/// Formats a file modification time as `"YYYY-MM-DD HH:MM"`, or a
/// translated "unknown" string. Port of `formatFileTime`.
///
/// Diverges from the C++ signature (`const std::filesystem::file_time_type&`,
/// "unknown" triggered by comparing against a default-constructed sentinel)
/// by taking `Option<SystemTime>` instead: `std::fs::Metadata::modified()`
/// is fallible (`io::Result`), and callers naturally have an `Option`/`Result`
/// already rather than a sentinel value to compare against. This also drops
/// the C++'s `file_time_type` → `system_clock` clock-domain conversion
/// dance entirely — `SystemTime` already *is* wall-clock time, so there's no
/// second incompatible clock to convert from.
pub fn format_file_time(tp: Option<SystemTime>) -> String {
    use crate::i18n;

    let Some(tp) = tp else {
        return i18n::tr("time.file.unknown", &[]);
    };
    let unix_seconds = system_time_to_unix_seconds(tp);
    let tm = local_tm(unix_seconds);
    let formatted = strftime::format_strftime_raw("%Y-%m-%d %H:%M", &tm);
    if formatted.is_empty() {
        i18n::tr("time.file.unknown", &[])
    } else {
        formatted
    }
}

/// Converts a `SystemTime` to Unix seconds, handling both directions of
/// `duration_since` (the C++'s `system_clock::to_time_t` accepts pre-1970
/// instants natively since `time_t` is signed; this restores the same range
/// instead of panicking/saturating at the epoch).
fn system_time_to_unix_seconds(tp: SystemTime) -> i64 {
    match tp.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Builds a system-local `libc::tm` for the given Unix timestamp via
/// `localtime_r`, shared by every function that formats in local time.
fn local_tm(unix_seconds: i64) -> libc::tm {
    let secs: libc::time_t = unix_seconds as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `secs` and `tm` are both valid and live for the call;
    // `localtime_r` only reads `secs` and writes into `tm`.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm
}

/// Parses the `"%d-%d-%dT%d:%d"` prefix `formatIsoTime` needs (year, month,
/// day, hour, minute), mirroring `std::sscanf`'s per-field leniency (no
/// zero-padding required, stops at the first non-digit) without pulling in
/// a parsing crate for five integers. Returns `None` on the first field or
/// literal-character mismatch, matching the C++'s `< 5`-fields-parsed check.
fn parse_iso_prefix(iso_time: &str) -> Option<(i32, i32, i32, i32, i32)> {
    let (year, rest) = take_int(iso_time)?;
    let rest = expect_literal(rest, '-')?;
    let (month, rest) = take_int(rest)?;
    let rest = expect_literal(rest, '-')?;
    let (day, rest) = take_int(rest)?;
    let rest = expect_literal(rest, 'T')?;
    let (hour, rest) = take_int(rest)?;
    let rest = expect_literal(rest, ':')?;
    let (minute, _rest) = take_int(rest)?;
    Some((
        year as i32,
        month as i32,
        day as i32,
        hour as i32,
        minute as i32,
    ))
}

/// Consumes a leading `-?[0-9]+` integer, returning it and the unconsumed
/// remainder. `None` if there's no digit at all (an `sscanf` `%d` failure).
fn take_int(s: &str) -> Option<(i64, &str)> {
    let bytes = s.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    let value = s[..i].parse::<i64>().ok()?;
    Some((value, &s[i..]))
}

/// Consumes a single expected literal character, or `None` if it's not
/// there (an `sscanf` literal-match failure).
fn expect_literal(s: &str, lit: char) -> Option<&str> {
    let mut chars = s.chars();
    (chars.next() == Some(lit)).then_some(chars.as_str())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso_prefix_accepts_well_formed_input() {
        assert_eq!(
            parse_iso_prefix("2026-05-09T06:23"),
            Some((2026, 5, 9, 6, 23))
        );
    }

    #[test]
    fn parse_iso_prefix_accepts_non_padded_fields_like_sscanf() {
        assert_eq!(parse_iso_prefix("2026-5-9T6:23"), Some((2026, 5, 9, 6, 23)));
    }

    #[test]
    fn parse_iso_prefix_ignores_trailing_content() {
        assert_eq!(
            parse_iso_prefix("2026-05-09T06:23:45.500Z"),
            Some((2026, 5, 9, 6, 23))
        );
    }

    #[test]
    fn parse_iso_prefix_rejects_missing_fields() {
        assert_eq!(parse_iso_prefix("2026-05-09"), None);
        assert_eq!(parse_iso_prefix("not-a-date"), None);
        assert_eq!(parse_iso_prefix(""), None);
    }

    #[test]
    fn format_iso_time_returns_input_unchanged_on_parse_failure() {
        assert_eq!(format_iso_time("garbage", "%H:%M"), "garbage");
    }

    #[test]
    fn format_iso_time_formats_a_valid_prefix() {
        assert_eq!(format_iso_time("2026-05-09T06:23", "%H:%M"), "06:23");
    }

    #[test]
    fn system_time_to_unix_seconds_round_trips_a_known_instant() {
        let tp = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(system_time_to_unix_seconds(tp), 1_700_000_000);
    }

    #[test]
    fn system_time_to_unix_seconds_handles_pre_epoch_instants() {
        let tp = UNIX_EPOCH - std::time::Duration::from_secs(100);
        assert_eq!(system_time_to_unix_seconds(tp), -100);
    }

    #[test]
    fn format_local_unix_time_formats_the_unix_epoch_token() {
        assert_eq!(format_local_unix_time(1_700_000_000, "%s"), "1700000000");
        assert_eq!(
            format_local_unix_time(1_700_000_000, "recording_%s"),
            "recording_1700000000"
        );
        assert_eq!(
            format_local_unix_time(1_700_000_000, "%%s_%s"),
            "%s_1700000000"
        );
    }

    #[test]
    fn format_utc_time_formats_a_known_instant() {
        let tp = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(
            format_utc_time(tp, "%Y-%m-%dT%H:%M:%SZ"),
            "2023-11-14T22:13:20Z"
        );
    }

    // `format_file_time(None)`'s "unknown" text goes through `i18n::tr`,
    // which needs a loaded catalog (`i18n::init`) — a process-global
    // `Mutex<Service>` shared with every other test in this crate's single
    // unit-test binary (see `i18n::service`'s own module doc comment). Tested
    // in `tests/time_format_test.rs` instead, a separate binary that calls
    // `i18n::init("en")` once up front, matching `time_format_test.cpp`.

    #[test]
    fn format_file_time_formats_a_known_instant() {
        let tp = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let formatted = format_file_time(Some(tp));
        assert_eq!(formatted.len(), "YYYY-MM-DD HH:MM".len());
    }
}
