//! Port of the relative/duration half of `src/time/time_format.cpp`:
//! `formatTimeAgo`, `formatElapsedSince`, `formatDuration`, `formatClockTime`,
//! and the private `formatAgeSeconds`/`formatDurationUnit`/
//! `formatDurationParts` helpers they share.

use std::time::{Duration, Instant, SystemTime};

use crate::i18n;

use super::{local_tm, strftime, system_time_to_unix_seconds};

/// Port of `formatAgeSeconds`. `calendar_after_six_days`, when present, is
/// the *original* instant (not "now") formatted in local time as `"Jan  5"`
/// once the age crosses a week — matches `formatTimeAgo` passing `tp`
/// itself through, and `formatElapsedSince` passing nothing (`steady_clock`
/// has no calendar mapping).
fn format_age_seconds(secs: i64, calendar_after_six_days: Option<SystemTime>) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        return i18n::tr("time.relative.just-now", &[]);
    }
    if secs < 3600 {
        return i18n::trp("time.relative.minutes-ago", secs / 60, &[]);
    }
    if secs < 86400 {
        return i18n::trp("time.relative.hours-ago", secs / 3600, &[]);
    }
    if secs < 7 * 86400 {
        return i18n::trp("time.relative.days-ago", secs / 86400, &[]);
    }
    if let Some(tp) = calendar_after_six_days {
        let tm = local_tm(system_time_to_unix_seconds(tp));
        let date = strftime::format_strftime_raw("%b %e", &tm);
        if !date.is_empty() {
            return date;
        }
    }
    i18n::trp("time.relative.days-ago", secs / 86400, &[])
}

/// Same wording as `format_time_ago`, but duration is computed from a
/// monotonic clock (e.g. `Notification::receivedTime`). Port of
/// `formatElapsedSince`.
pub fn format_elapsed_since(since: Instant) -> String {
    let secs = Instant::now().saturating_duration_since(since).as_secs() as i64;
    format_age_seconds(secs, None)
}

/// Port of `formatTimeAgo`.
pub fn format_time_ago(tp: SystemTime) -> String {
    let secs = match SystemTime::now().duration_since(tp) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0, // `tp` is in the future; clamps to 0 exactly like the C++'s `std::max`.
    };
    format_age_seconds(secs, Some(tp))
}

fn format_duration_unit(key: &str, count: u64) -> String {
    i18n::trp(key, count as i64, &[])
}

fn format_duration_parts2(first: &str, second: &str) -> String {
    i18n::tr(
        "time.duration.two-parts",
        &[("first", first.to_string()), ("second", second.to_string())],
    )
}

fn format_duration_parts3(first: &str, second: &str, third: &str) -> String {
    i18n::tr(
        "time.duration.three-parts",
        &[
            ("first", first.to_string()),
            ("second", second.to_string()),
            ("third", third.to_string()),
        ],
    )
}

/// Formats a duration using translated day/hour/minute units. Port of
/// `formatDuration`.
///
/// Takes `std::time::Duration` (unsigned) rather than the C++'s signed
/// `std::chrono::seconds`: every real call site already passes a
/// non-negative duration (uptime, playback position, battery estimate), and
/// `Duration` is the natural Rust type for callers to hold one in.
pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let days = total_seconds / 86400;
    let rem = total_seconds % 86400;
    let hours = rem / 3600;
    let minutes = (rem % 3600) / 60;

    if days > 0 {
        let day_text = format_duration_unit("time.units.day", days);
        if hours > 0 && minutes > 0 {
            return format_duration_parts3(
                &day_text,
                &format_duration_unit("time.units.hour", hours),
                &format_duration_unit("time.units.minute", minutes),
            );
        }
        if hours > 0 {
            return format_duration_parts2(
                &day_text,
                &format_duration_unit("time.units.hour", hours),
            );
        }
        if minutes > 0 {
            return format_duration_parts2(
                &day_text,
                &format_duration_unit("time.units.minute", minutes),
            );
        }
        return day_text;
    }
    if hours > 0 {
        let hour_text = format_duration_unit("time.units.hour", hours);
        if minutes > 0 {
            return format_duration_parts2(
                &hour_text,
                &format_duration_unit("time.units.minute", minutes),
            );
        }
        return hour_text;
    }
    if minutes > 0 {
        return format_duration_unit("time.units.minute", minutes);
    }
    i18n::tr("time.duration.less-than-minute", &[])
}

/// Formats seconds as clock-style `"M:SS"` or `"H:MM:SS"`. Returns `"0:00"`
/// for `<= 0`. Port of `formatClockTime`.
pub fn format_clock_time(seconds: i64) -> String {
    if seconds <= 0 {
        return "0:00".to_string();
    }
    let total_minutes = seconds / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn format_clock_time_pads_seconds_and_minutes() {
        assert_eq!(format_clock_time(0), "0:00");
        assert_eq!(format_clock_time(-5), "0:00");
        assert_eq!(format_clock_time(65), "1:05");
        assert_eq!(format_clock_time(3661), "1:01:01");
    }
}
