//! Port of `tests/time_format_test.cpp`, plus new coverage (no C++ test
//! exists for these) of the `time::*` functions that resolve text through
//! `i18n::tr`/`trp`: `format_duration`, `format_time_ago`,
//! `format_elapsed_since`, and `format_file_time(None)`'s "unknown" text.
//!
//! Kept as its own integration-test binary rather than unit tests inside
//! `time::*`, for the same reason as `i18n_supported_languages_test.rs`:
//! `i18n::init` writes a process-global `Mutex<Service>`
//! (`i18n::service`'s module doc comment) shared with every other test in
//! `noctalia-core`'s single unit-test binary, and this needs a loaded
//! catalog before any of the assertions below mean anything. A separate
//! integration-test binary can call `i18n::init("en")` once up front,
//! matching the C++ test's own `i18n::Service::instance().init("en")`,
//! without racing anything else.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use jiff::tz::TimeZone;

use noctalia_core::i18n;
use noctalia_core::time::{
    format_duration, format_elapsed_since, format_file_time, format_local_unix_time,
    format_time_ago, format_timezone_time, is_valid_timezone,
};

fn expected_zone_label(now: Timestamp, tz: &TimeZone) -> String {
    let info = tz.to_offset_info(now);
    let total_minutes = info.offset().seconds() / 60;
    let hours = total_minutes / 60;
    let minutes = (total_minutes % 60).abs();
    format!("{hours:+03}{minutes:02}|{}", info.abbreviation())
}

fn utc_day_of_year(now: Timestamp) -> String {
    format!("{:03}", now.to_zoned(TimeZone::UTC).day_of_year())
}

#[test]
fn ported_time_format_test_cpp() {
    i18n::init("en");

    assert_eq!(format_local_unix_time(1_700_000_000, "%s"), "1700000000");
    assert_eq!(
        format_local_unix_time(1_700_000_000, "recording_%s"),
        "recording_1700000000"
    );
    assert_eq!(
        format_local_unix_time(1_700_000_000, "%%s_%s"),
        "%s_1700000000"
    );

    assert!(is_valid_timezone(""));
    assert!(is_valid_timezone("UTC"));
    assert!(!is_valid_timezone("Europe/Berln"));

    let kiritimati =
        TimeZone::get("Pacific/Kiritimati").expect("Pacific/Kiritimati must exist in tzdb");
    let before_timezone_format = Timestamp::now();
    assert_eq!(
        format_timezone_time("%z|%Z", "Pacific/Kiritimati"),
        expected_zone_label(before_timezone_format, &kiritimati)
    );

    let formatted_utc_day = format_timezone_time("%j", "UTC");
    let after_timezone_format = Timestamp::now();
    let utc_day_matches = formatted_utc_day == utc_day_of_year(before_timezone_format)
        || formatted_utc_day == utc_day_of_year(after_timezone_format);
    assert!(utc_day_matches, "formatted={formatted_utc_day}");

    assert_eq!(format_duration(Duration::from_secs(59)), "<1m");
    assert_eq!(format_duration(Duration::from_secs(60)), "1 minute");
    assert_eq!(
        format_duration(Duration::from_secs(2 * 3600 + 60)),
        "2 hours 1 minute"
    );
    assert_eq!(
        format_duration(Duration::from_secs(24 * 3600 + 3600 + 60)),
        "1 day 1 hour 1 minute"
    );
}

#[test]
fn format_duration_covers_every_branch() {
    i18n::init("en");
    assert_eq!(format_duration(Duration::from_secs(0)), "<1m");
    assert_eq!(format_duration(Duration::from_secs(3600)), "1 hour");
    assert_eq!(format_duration(Duration::from_secs(24 * 3600)), "1 day");
    assert_eq!(
        format_duration(Duration::from_secs(24 * 3600 + 3600)),
        "1 day 1 hour"
    );
    assert_eq!(
        format_duration(Duration::from_secs(24 * 3600 + 60)),
        "1 day 1 minute"
    );
}

#[test]
fn format_time_ago_reports_just_now_and_minutes_ago() {
    i18n::init("en");
    let now = SystemTime::now();
    assert_eq!(format_time_ago(now), "just now");
    assert_eq!(format_time_ago(now - Duration::from_secs(120)), "2 min ago");
}

#[test]
fn format_elapsed_since_reports_just_now_and_hours_ago() {
    i18n::init("en");
    let now = Instant::now();
    assert_eq!(format_elapsed_since(now), "just now");
    let earlier = now
        .checked_sub(Duration::from_secs(2 * 3600))
        .expect("instant underflow");
    assert_eq!(format_elapsed_since(earlier), "2 hr ago");
}

#[test]
fn format_file_time_none_reports_translated_unknown() {
    i18n::init("en");
    assert_eq!(format_file_time(None), "Unknown");
}
