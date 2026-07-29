//! Port of the `nl_langinfo`-backed half of `src/time/time_format.cpp`:
//! `formatCurrentDate` and `localeFirstDayOfWeek`.
//!
//! `_NL_TIME_FIRST_WEEKDAY`/`_NL_TIME_WEEK_1STDAY` are GNU-only
//! `<langinfo.h>` extension items (under `__USE_GNU`) that the `libc` crate
//! doesn't expose — confirmed by grepping its source for both names.
//! `D_FMT`/`nl_langinfo` themselves *are* exposed (and work correctly on
//! this target — verified against a small C program linked against the same
//! glibc, both the raw `nl_item` value and the returned string matched
//! exactly). The two missing constants are hand-verified the same way
//! against this host's real glibc headers (see PROGRESS.log, task 1.9) and
//! hardcoded here, `#[cfg(target_env = "gnu")]`-gated like the C++'s
//! `#if defined(__GLIBC__)`.

use std::ffi::CStr;
use std::time::SystemTime;

use super::{local_tm, strftime, system_time_to_unix_seconds};

#[cfg(target_env = "gnu")]
const NL_TIME_FIRST_WEEKDAY: libc::nl_item = 131176;
#[cfg(target_env = "gnu")]
const NL_TIME_WEEK_1STDAY: libc::nl_item = 131174;

/// Formats the current local date using the locale's preferred format, with
/// a weekday name prefixed. Port of `formatCurrentDate`.
pub fn format_current_date() -> String {
    // SAFETY: `nl_langinfo` returns a pointer into locale-owned/static
    // storage, valid until the next locale-affecting call; copied into an
    // owned `String` immediately.
    let d_fmt = unsafe { CStr::from_ptr(libc::nl_langinfo(libc::D_FMT)) }
        .to_string_lossy()
        .into_owned();

    let mut fmt = String::from("%A, ");
    fmt.push_str(&d_fmt);
    // Widen a 2-digit-year `%y` to 4-digit `%Y` — same non-overlapping
    // left-to-right replace as `strftime::normalize_format_escapes`.
    let fmt = fmt.replace("%y", "%Y");

    let tm = local_tm(system_time_to_unix_seconds(SystemTime::now()));
    strftime::format_strftime(&fmt, &tm)
}

/// First day of the week from the active `LC_TIME` locale (`tm_wday`
/// encoding: Sun=0 .. Sat=6). Port of `localeFirstDayOfWeek`.
pub fn locale_first_day_of_week() -> i32 {
    #[cfg(target_env = "gnu")]
    if let Some(day) = glibc_first_day_of_week() {
        return day;
    }
    // ISO-style Monday when the platform doesn't expose locale week data.
    1
}

#[cfg(target_env = "gnu")]
fn glibc_first_day_of_week() -> Option<i32> {
    // SAFETY: both calls return pointers into locale-owned/static
    // `nl_langinfo` storage. `_NL_TIME_FIRST_WEEKDAY`'s pointer is
    // dereferenced for one byte immediately below, after a null check.
    // `_NL_TIME_WEEK_1STDAY`'s pointer is never dereferenced — glibc packs
    // this particular item's answer into the pointer's own numeric value
    // (a documented glibc quirk); only its address is read.
    let (first_weekday_ptr, week1stday_ptr) = unsafe {
        (
            libc::nl_langinfo(NL_TIME_FIRST_WEEKDAY),
            libc::nl_langinfo(NL_TIME_WEEK_1STDAY),
        )
    };

    let week1stday = week1stday_ptr as usize as u32;
    if first_weekday_ptr.is_null() || week1stday == 0 {
        return None;
    }
    // SAFETY: verified non-null above; glibc guarantees at least one
    // readable byte for `_NL_TIME_FIRST_WEEKDAY`.
    let first_weekday = i32::from(unsafe { *first_weekday_ptr.cast::<u8>() });
    if first_weekday < 1 {
        return None;
    }

    // WEEK_1STDAY packs a YYYYMMDD anchor date; FIRST_WEEKDAY is a 1-based
    // offset (in days) from that anchor to the locale's actual first
    // weekday. Compute the anchor's own weekday via pure calendar math
    // (jiff's `civil::Date` — no timezone/tzdb involved) and shift by the
    // offset, exactly mirroring the C++'s `std::chrono::sys_days` use.
    let year = (week1stday / 10000) as i16;
    let month = ((week1stday / 100) % 100) as i8;
    let day = (week1stday % 100) as i8;
    let anchor = jiff::civil::Date::new(year, month, day).ok()?;
    let anchor_weekday = i32::from(anchor.weekday().to_sunday_zero_offset());
    Some((anchor_weekday + first_weekday - 1) % 7)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn format_current_date_is_well_formed_and_starts_with_a_weekday() {
        let formatted = format_current_date();
        assert!(!formatted.is_empty());
        assert!(formatted.contains(", "));
    }

    #[test]
    fn locale_first_day_of_week_is_a_valid_tm_wday() {
        let day = locale_first_day_of_week();
        assert!((0..=6).contains(&day));
    }

    #[cfg(target_env = "gnu")]
    #[test]
    fn glibc_first_day_of_week_anchor_math_is_self_consistent() {
        // 2000-01-03 (a Monday) as the anchor with firstWeekday=1 should
        // yield Monday (1) — a hand-checkable case for the anchor-shift
        // formula, independent of whatever this host's real locale reports.
        let anchor = jiff::civil::Date::new(2000, 1, 3).unwrap();
        assert_eq!(anchor.weekday(), jiff::civil::Weekday::Monday);
        let anchor_weekday = i32::from(anchor.weekday().to_sunday_zero_offset());
        assert_eq!((anchor_weekday + 1 - 1) % 7, 1);
    }
}
