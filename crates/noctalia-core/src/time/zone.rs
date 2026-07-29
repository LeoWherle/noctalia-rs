//! Port of the two `src/time/time_format.cpp` functions that genuinely need
//! an IANA timezone database — `isValidTimezone` and `formatTimezoneTime` —
//! the one place this crate uses `jiff` (architecture decision 5). Every
//! other function in `time::*` only ever needs the system-local zone
//! (`libc::localtime_r`, no database lookup) or pure calendar math
//! ([`super::locale`]'s anchor-weekday computation).
//!
//! `formatTimezoneTime` uses `jiff` only to compute the correct
//! offset/abbreviation/DST/wall-clock fields for an arbitrary zone name at a
//! given instant (replacing `std::chrono::time_zone::get_info`), then hands
//! those fields to a `libc::tm` and renders through the same
//! `libc::strftime`-based engine as everything else in this module — see
//! [`super`]'s module doc comment for why rendering stays on `libc::strftime`
//! rather than `jiff`'s own formatter (locale-aware weekday/month names).

use std::ffi::CString;
use std::time::SystemTime;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::strftime;
use super::system_time_to_unix_seconds;

/// Empty selects system-local time; a non-empty value must name a zone in
/// the active timezone database. Port of `isValidTimezone`.
pub fn is_valid_timezone(tz_name: &str) -> bool {
    tz_name.is_empty() || TimeZone::get(tz_name).is_ok()
}

/// Formats current time for a specific timezone. Falls back to local time
/// if the timezone is invalid or empty. Port of `formatTimezoneTime`.
pub fn format_timezone_time(fmt: &str, tz_name: &str) -> String {
    if tz_name.is_empty() {
        return super::format_local_time(fmt);
    }
    let Ok(tz) = TimeZone::get(tz_name) else {
        return super::format_local_time(fmt);
    };

    let unix_seconds = system_time_to_unix_seconds(SystemTime::now());
    let Ok(ts) = Timestamp::from_second(unix_seconds) else {
        return super::format_local_time(fmt);
    };
    let zdt = ts.to_zoned(tz.clone());
    let info = tz.to_offset_info(ts);

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = i32::from(zdt.year()) - 1900;
    tm.tm_mon = i32::from(zdt.month()) - 1;
    tm.tm_mday = i32::from(zdt.day());
    tm.tm_hour = i32::from(zdt.hour());
    tm.tm_min = i32::from(zdt.minute());
    tm.tm_sec = i32::from(zdt.second());
    tm.tm_wday = i32::from(zdt.weekday().to_sunday_zero_offset());
    tm.tm_yday = i32::from(zdt.day_of_year()) - 1; // tm_yday is 0-based; jiff's is 1-based.
    tm.tm_isdst = i32::from(info.dst().is_dst());
    tm.tm_gmtoff = libc::c_long::from(zdt.offset().seconds());
    // `abbreviation()` is tzdata-sourced ASCII (e.g. "CET", "+14") and never
    // contains a NUL in practice; `unwrap_or_default` degrades to an empty
    // `%Z` rather than panicking on the unreachable case.
    let abbrev = CString::new(info.abbreviation()).unwrap_or_default();
    // SAFETY-relevant, not `unsafe` itself: `abbrev` must outlive every use
    // of `tm` below (the `format_strftime_compat` call, which may read
    // `tm_zone` while rendering `%Z`). It's a local that isn't dropped until
    // this function returns, so that holds.
    tm.tm_zone = abbrev.as_ptr();

    let normalized = strftime::normalize_format_escapes(fmt);
    strftime::format_strftime_compat(&normalized, &tm, Some(unix_seconds)).unwrap_or(normalized)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_timezone_accepts_empty_and_known_zones() {
        assert!(is_valid_timezone(""));
        assert!(is_valid_timezone("UTC"));
        assert!(is_valid_timezone("Pacific/Kiritimati"));
    }

    #[test]
    fn is_valid_timezone_rejects_unknown_zones() {
        assert!(!is_valid_timezone("Europe/Berln"));
        assert!(!is_valid_timezone("Not/AZone"));
    }

    #[test]
    fn format_timezone_time_falls_back_to_local_for_empty_or_invalid_zone() {
        assert_eq!(
            format_timezone_time("%H:%M", ""),
            super::super::format_local_time("%H:%M")
        );
        assert_eq!(
            format_timezone_time("%H:%M", "Not/AZone"),
            super::super::format_local_time("%H:%M")
        );
    }

    #[test]
    fn format_timezone_time_renders_offset_and_abbreviation() {
        // Pacific/Kiritimati has had a fixed +14:00 offset (no DST) since
        // its 1995 date-line move, so this is stable across "now".
        assert_eq!(
            format_timezone_time("%z|%Z", "Pacific/Kiritimati"),
            "+1400|+14"
        );
    }

    #[test]
    fn format_timezone_time_day_of_year_matches_utc_day() {
        use jiff::{Timestamp, tz::TimeZone};

        let before = system_time_to_unix_seconds(SystemTime::now());
        let formatted = format_timezone_time("%j", "UTC");
        let after = system_time_to_unix_seconds(SystemTime::now());

        let expected_for = |secs: i64| -> String {
            let ts = Timestamp::from_second(secs).unwrap();
            let zdt = ts.to_zoned(TimeZone::UTC);
            format!("{:03}", zdt.day_of_year())
        };
        assert!(
            formatted == expected_for(before) || formatted == expected_for(after),
            "formatted={formatted}, expected one of before/after day-of-year"
        );
    }
}
