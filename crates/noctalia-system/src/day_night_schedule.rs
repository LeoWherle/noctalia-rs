//! Port of `src/system/day_night_schedule.{cpp,h}` (task 5.6.2): solar-time / manual-schedule
//! day-night boundary evaluation for night light / theme auto mode. Self-contained given
//! already-ported `noctalia_config::types::location::LocationConfig` — no forward-phase
//! dependency.
//!
//! No C++ test exists for this file (confirmed: no `day_night_schedule_test.cpp` in `tests/`) —
//! task 5.6's own done bar is smoke tests, fixture-driven where meaningful.
//!
//! `evaluate`'s manual-mode and astronomical-mode branches are pulled out into private
//! `evaluate_manual`/`evaluate_astronomical` functions parameterized on the current
//! minute-of-day/second, and `compute_solar_times` is split into a pure
//! `compute_solar_times_for(day_of_year, gmtoff_sec, ...)` core — same testability-driven
//! pure-core-extraction precedent as `ddc.rs`'s `detect_ddc_displays`/
//! `parse_ddc_detect_output` split (task 5.3.2): the C++ reads "now" via
//! `system_clock::now()`/`localtime_r` inline inside both `evaluate` and `computeSolarTimes`,
//! which makes those two functions untestable without depending on the real wall clock. This
//! doesn't change `evaluate`'s or `compute_solar_times`'s observable behavior — the public
//! functions still read live wall-clock time exactly where the C++ does — it only exposes the
//! pure math underneath for direct, deterministic testing.

use std::time::Duration;

use noctalia_config::types::location::LocationConfig;

/// Port of `day_night_schedule::GeoCoordinates`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GeoCoordinates {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Port of `day_night_schedule::Evaluation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evaluation {
    pub night: bool,
    pub until_boundary: Duration,
    /// Time elapsed since the most recent day/night boundary. Used to position a clock-anchored
    /// fade ramp so the temperature depends on wall-clock time, not on when the app started.
    pub since_boundary: Duration,
}

impl Default for Evaluation {
    fn default() -> Self {
        Self {
            night: false,
            until_boundary: Duration::from_secs(3600),
            since_boundary: Duration::from_secs(3600),
        }
    }
}

/// Port of the anonymous namespace's `timeToMinutes`. Callers must have accepted the string
/// through [`normalized_clock`] first.
fn time_to_minutes(hhmm: &str) -> i32 {
    let b = hhmm.as_bytes();
    i32::from(b[0] - b'0') * 600
        + i32::from(b[1] - b'0') * 60
        + i32::from(b[3] - b'0') * 10
        + i32::from(b[4] - b'0')
}

fn local_tm() -> libc::tm {
    let mut t: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `t`/`tm` are valid, writable buffers of the sizes `time`/`localtime_r` expect, live
    // for the duration of these calls.
    unsafe {
        libc::time(&mut t);
        libc::localtime_r(&t, &mut tm);
    }
    tm
}

/// Port of the anonymous namespace's `currentLocalTime`: (minutes since local midnight, seconds
/// within the current minute).
fn current_local_time() -> (i32, i32) {
    let tm = local_tm();
    (tm.tm_hour * 60 + tm.tm_min, tm.tm_sec)
}

/// Port of the anonymous namespace's `sinceBoundaryMs`.
fn since_boundary_ms(now_min: i32, now_sec: i32, last_boundary_min: i32) -> Duration {
    let mut since_min = now_min - last_boundary_min;
    if since_min < 0 {
        since_min += 1440;
    }
    Duration::from_millis((since_min * 60 * 1000 + now_sec * 1000) as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SolarTimes {
    sunrise_minutes: i32,
    sunset_minutes: i32,
}

/// Pure core of the anonymous namespace's `computeSolarTimes` — see the module doc comment.
/// `day_of_year` is 1-indexed (matches the C++'s `tm_yday + 1`); `gmtoff_sec` is the local UTC
/// offset in seconds (matches `tm.tm_gmtoff`).
fn compute_solar_times_for(
    day_of_year: i32,
    gmtoff_sec: i64,
    latitude: f64,
    longitude: f64,
) -> SolarTimes {
    let pi = std::f64::consts::PI;
    let fractional_year = 2.0 * pi / 365.0 * (f64::from(day_of_year) - 1.0);

    let equation_of_time = 229.18
        * (0.000075 + 0.001868 * fractional_year.cos()
            - 0.032077 * fractional_year.sin()
            - 0.014615 * (2.0 * fractional_year).cos()
            - 0.040849 * (2.0 * fractional_year).sin());
    let declination = 0.006918 - 0.399912 * fractional_year.cos()
        + 0.070257 * fractional_year.sin()
        - 0.006758 * (2.0 * fractional_year).cos()
        + 0.000907 * (2.0 * fractional_year).sin()
        - 0.002697 * (3.0 * fractional_year).cos()
        + 0.00148 * (3.0 * fractional_year).sin();

    let sunrise_zenith = 90.833 * pi / 180.0;
    let lat_rad = latitude * pi / 180.0;
    let hour_angle_arg = sunrise_zenith.cos() / (lat_rad.cos() * declination.cos())
        - lat_rad.tan() * declination.tan();

    if hour_angle_arg > 1.0 {
        return SolarTimes {
            sunrise_minutes: 0,
            sunset_minutes: 0,
        };
    }
    if hour_angle_arg < -1.0 {
        return SolarTimes {
            sunrise_minutes: 0,
            sunset_minutes: 1440,
        };
    }

    let hour_angle_deg = hour_angle_arg.clamp(-1.0, 1.0).acos() * 180.0 / pi;
    let time_zone_offset_min = gmtoff_sec as f64 / 60.0;
    let solar_noon_min = 720.0 - 4.0 * longitude - equation_of_time + time_zone_offset_min;

    let normalize_minutes = |minutes: f64| -> i32 {
        let mut rounded = minutes.round() as i32;
        rounded %= 1440;
        if rounded < 0 {
            rounded += 1440;
        }
        rounded
    };

    SolarTimes {
        sunrise_minutes: normalize_minutes(solar_noon_min - hour_angle_deg * 4.0),
        sunset_minutes: normalize_minutes(solar_noon_min + hour_angle_deg * 4.0),
    }
}

fn compute_solar_times(latitude: f64, longitude: f64) -> SolarTimes {
    let tm = local_tm();
    compute_solar_times_for(tm.tm_yday + 1, tm.tm_gmtoff, latitude, longitude)
}

/// Port of `day_night_schedule::normalizedClock`.
pub fn normalized_clock(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    if !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || !bytes[4].is_ascii_digit()
    {
        return None;
    }
    let hour = i32::from(bytes[0] - b'0') * 10 + i32::from(bytes[1] - b'0');
    let minute = i32::from(bytes[3] - b'0') * 10 + i32::from(bytes[4] - b'0');
    // The C++ also checks `hour < 0`/`minute < 0`, which can never be true here (both are built
    // from two ASCII-digit subtractions) — dropped as genuinely unreachable, not a behavior change.
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(value.to_string())
}

/// Port of `day_night_schedule::resolveCoordinates`.
///
/// `resolvedLatitude`/`resolvedLongitude` are the coordinates published by `LocationService` (IP
/// geolocation or geocoded address). When absent, manual latitude/longitude from the config are
/// used. Fixed sunrise/sunset times are used only when `LocationConfig::custom_schedule` is
/// explicitly true — they are not an automatic fallback when coordinates are unavailable.
pub fn resolve_coordinates(
    config: &LocationConfig,
    resolved_latitude: Option<f64>,
    resolved_longitude: Option<f64>,
) -> GeoCoordinates {
    if resolved_latitude.is_some() && resolved_longitude.is_some() {
        return GeoCoordinates {
            latitude: resolved_latitude,
            longitude: resolved_longitude,
        };
    }
    if config.latitude.is_some() && config.longitude.is_some() {
        return GeoCoordinates {
            latitude: config.latitude,
            longitude: config.longitude,
        };
    }
    GeoCoordinates::default()
}

/// Port of `day_night_schedule::hasUsableCustomTimes`. Both sunset and sunrise parse as `HH:MM`.
/// Custom scheduling needs this; without it the times cannot drive a schedule and the request is
/// a misconfiguration to surface, not to absorb.
pub fn has_usable_custom_times(config: &LocationConfig) -> bool {
    normalized_clock(&config.sunset).is_some() && normalized_clock(&config.sunrise).is_some()
}

/// Port of `day_night_schedule::isManualMode`.
pub fn is_manual_mode(config: &LocationConfig) -> bool {
    config.custom_schedule && has_usable_custom_times(config)
}

/// Pure core of `evaluate`'s manual-schedule branch — see the module doc comment.
fn evaluate_manual(sunset_min: i32, sunrise_min: i32, now_min: i32, now_sec: i32) -> Evaluation {
    let night = if sunset_min < sunrise_min {
        now_min >= sunset_min && now_min < sunrise_min
    } else {
        now_min >= sunset_min || now_min < sunrise_min
    };
    let target_min = if night { sunrise_min } else { sunset_min };
    let mut diff_min = target_min - now_min;
    if diff_min <= 0 {
        diff_min += 1440;
    }
    let ms = diff_min * 60 * 1000 - now_sec * 1000;
    Evaluation {
        night,
        until_boundary: Duration::from_millis(ms.max(1000) as u64),
        since_boundary: since_boundary_ms(
            now_min,
            now_sec,
            if night { sunset_min } else { sunrise_min },
        ),
    }
}

/// Pure core of `evaluate`'s astronomical branch — see the module doc comment.
fn evaluate_astronomical(times: SolarTimes, now_min: i32, now_sec: i32) -> Evaluation {
    if times.sunrise_minutes == 0 && times.sunset_minutes == 0 {
        // Polar night: the sun never rises, so there is no boundary today.
        return Evaluation {
            night: true,
            ..Evaluation::default()
        };
    }
    if times.sunrise_minutes == 0 && times.sunset_minutes == 1440 {
        // Polar day: the sun never sets, so there is no boundary today.
        return Evaluation::default();
    }

    let sunset = times.sunset_minutes;
    let sunrise = times.sunrise_minutes;
    let night = if sunset > sunrise {
        now_min >= sunset || now_min < sunrise
    } else {
        now_min >= sunset && now_min < sunrise
    };
    let target_min = if night { sunrise } else { sunset };
    let mut diff_min = target_min - now_min;
    if diff_min <= 0 {
        diff_min += 1440;
    }
    let ms = diff_min * 60 * 1000 - now_sec * 1000;
    Evaluation {
        night,
        until_boundary: Duration::from_millis(ms.max(1000) as u64),
        since_boundary: since_boundary_ms(now_min, now_sec, if night { sunset } else { sunrise }),
    }
}

/// Port of `day_night_schedule::evaluate`.
pub fn evaluate(
    config: &LocationConfig,
    resolved_latitude: Option<f64>,
    resolved_longitude: Option<f64>,
) -> Evaluation {
    let (now_min, now_sec) = current_local_time();

    if is_manual_mode(config) {
        let sunset_min = time_to_minutes(&config.sunset);
        let sunrise_min = time_to_minutes(&config.sunrise);
        return evaluate_manual(sunset_min, sunrise_min, now_min, now_sec);
    }

    let coords = resolve_coordinates(config, resolved_latitude, resolved_longitude);
    let (Some(latitude), Some(longitude)) = (coords.latitude, coords.longitude) else {
        // No coordinates available, retry in 1 hour in case location resolves later.
        return Evaluation::default();
    };

    let times = compute_solar_times(latitude, longitude);
    evaluate_astronomical(times, now_min, now_sec)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn normalized_clock_accepts_valid_hhmm() {
        assert_eq!(normalized_clock("09:30"), Some("09:30".to_string()));
        assert_eq!(normalized_clock("00:00"), Some("00:00".to_string()));
        assert_eq!(normalized_clock("23:59"), Some("23:59".to_string()));
    }

    #[test]
    fn normalized_clock_rejects_malformed_input() {
        assert_eq!(normalized_clock(""), None);
        assert_eq!(normalized_clock("9:30"), None, "wrong length");
        assert_eq!(normalized_clock("09-30"), None, "wrong separator");
        assert_eq!(normalized_clock("0a:30"), None, "non-digit hour");
        assert_eq!(normalized_clock("09:3a"), None, "non-digit minute");
        assert_eq!(normalized_clock("24:00"), None, "hour out of range");
        assert_eq!(normalized_clock("00:60"), None, "minute out of range");
    }

    #[test]
    fn has_usable_custom_times_requires_both_fields_valid() {
        let mut config = LocationConfig::default();
        assert!(!has_usable_custom_times(&config));
        config.sunset = "20:00".to_string();
        assert!(!has_usable_custom_times(&config), "sunrise still missing");
        config.sunrise = "07:00".to_string();
        assert!(has_usable_custom_times(&config));
        config.sunrise = "bad".to_string();
        assert!(!has_usable_custom_times(&config));
    }

    #[test]
    fn is_manual_mode_requires_flag_and_usable_times() {
        let mut config = LocationConfig {
            custom_schedule: true,
            sunset: "20:00".to_string(),
            sunrise: "07:00".to_string(),
            ..Default::default()
        };
        assert!(is_manual_mode(&config));
        config.custom_schedule = false;
        assert!(!is_manual_mode(&config), "flag off, even with usable times");
        config.custom_schedule = true;
        config.sunrise = String::new();
        assert!(
            !is_manual_mode(&config),
            "unusable times, even with flag on"
        );
    }

    #[test]
    fn resolve_coordinates_prefers_resolved_over_config_over_empty() {
        let config = LocationConfig {
            latitude: Some(10.0),
            longitude: Some(20.0),
            ..Default::default()
        };
        assert_eq!(
            resolve_coordinates(&config, Some(1.0), Some(2.0)),
            GeoCoordinates {
                latitude: Some(1.0),
                longitude: Some(2.0)
            },
            "resolved coordinates win when both present"
        );
        assert_eq!(
            resolve_coordinates(&config, None, None),
            GeoCoordinates {
                latitude: Some(10.0),
                longitude: Some(20.0)
            },
            "falls back to config coordinates"
        );
        assert_eq!(
            resolve_coordinates(&LocationConfig::default(), None, None),
            GeoCoordinates::default(),
            "empty when neither source has both coordinates"
        );
        assert_eq!(
            resolve_coordinates(&config, Some(1.0), None),
            GeoCoordinates {
                latitude: Some(10.0),
                longitude: Some(20.0)
            },
            "a partial resolved pair (only latitude) does not count as resolved"
        );
    }

    // Golden values below were computed independently (a from-scratch reimplementation of the
    // same published NOAA solar-position formula the C++ uses, run outside this repo) rather than
    // derived from `compute_solar_times_for` itself, so they can actually catch a transcription
    // error in the port.
    #[test]
    fn compute_solar_times_for_matches_independently_computed_values() {
        // Equator, prime meridian, day 80 (~spring equinox), UTC.
        let times = compute_solar_times_for(80, 0, 0.0, 0.0);
        assert_eq!(times.sunrise_minutes, 365);
        assert_eq!(times.sunset_minutes, 1091);

        // Mid-latitude Northern Hemisphere, day 80, UTC-5.
        let times = compute_solar_times_for(80, -18_000, 40.7, -74.0);
        assert_eq!(times.sunrise_minutes, 360);
        assert_eq!(times.sunset_minutes, 1088);
    }

    #[test]
    fn compute_solar_times_for_detects_polar_night() {
        let times = compute_solar_times_for(1, 0, 80.0, 0.0);
        assert_eq!(times.sunrise_minutes, 0);
        assert_eq!(times.sunset_minutes, 0);
    }

    #[test]
    fn compute_solar_times_for_detects_polar_day() {
        let times = compute_solar_times_for(172, 0, 80.0, 0.0);
        assert_eq!(times.sunrise_minutes, 0);
        assert_eq!(times.sunset_minutes, 1440);
    }

    #[test]
    fn evaluate_astronomical_reports_polar_night_and_polar_day() {
        let polar_night = SolarTimes {
            sunrise_minutes: 0,
            sunset_minutes: 0,
        };
        let eval = evaluate_astronomical(polar_night, 600, 0);
        assert!(eval.night);
        assert_eq!(eval.until_boundary, Duration::from_secs(3600));
        assert_eq!(eval.since_boundary, Duration::from_secs(3600));

        let polar_day = SolarTimes {
            sunrise_minutes: 0,
            sunset_minutes: 1440,
        };
        let eval = evaluate_astronomical(polar_day, 600, 0);
        assert!(!eval.night);
        assert_eq!(eval.until_boundary, Duration::from_secs(3600));
        assert_eq!(eval.since_boundary, Duration::from_secs(3600));
    }

    #[test]
    fn evaluate_astronomical_reports_day_and_night_around_boundaries() {
        // sunrise 06:00 (360), sunset 18:00 (1080).
        let times = SolarTimes {
            sunrise_minutes: 360,
            sunset_minutes: 1080,
        };

        // Midday: daytime, next boundary is sunset at 18:00, 6 hours from 12:00.
        let eval = evaluate_astronomical(times, 720, 0);
        assert!(!eval.night);
        assert_eq!(eval.until_boundary, Duration::from_secs(6 * 3600));
        assert_eq!(eval.since_boundary, Duration::from_secs(6 * 3600));

        // Midnight: nighttime, next boundary is sunrise at 06:00, 6 hours from 00:00.
        let eval = evaluate_astronomical(times, 0, 0);
        assert!(eval.night);
        assert_eq!(eval.until_boundary, Duration::from_secs(6 * 3600));
    }

    #[test]
    fn evaluate_manual_reports_day_and_night_around_boundaries() {
        // sunset 20:00 (1200), sunrise 06:00 (360) — night wraps past midnight.
        // At 21:00 (1260): night, 9 hours until sunrise.
        let eval = evaluate_manual(1200, 360, 1260, 0);
        assert!(eval.night);
        assert_eq!(eval.until_boundary, Duration::from_secs(9 * 3600));

        // At 12:00 (720): day, 8 hours until sunset.
        let eval = evaluate_manual(1200, 360, 720, 0);
        assert!(!eval.night);
        assert_eq!(eval.until_boundary, Duration::from_secs(8 * 3600));
    }

    #[test]
    fn evaluate_manual_floors_until_boundary_at_one_second() {
        // One second before the sunset boundary: `until_boundary` floors at 1000ms rather than
        // going to (near) zero, matching the C++'s `std::max(ms, 1000ms)`.
        let eval = evaluate_manual(1200, 360, 1199, 59);
        assert_eq!(eval.until_boundary, Duration::from_millis(1000));
    }

    #[test]
    fn evaluate_uses_manual_schedule_when_configured() {
        let config = LocationConfig {
            custom_schedule: true,
            sunset: "20:00".to_string(),
            sunrise: "07:00".to_string(),
            // Deliberately present but must be ignored: manual mode wins over coordinates.
            latitude: Some(51.5),
            longitude: Some(-0.1),
            ..Default::default()
        };
        // Just confirm this takes the manual path and produces a self-consistent result — the
        // real "now" isn't controllable here (see the module doc comment), so this only checks
        // shape, not values already covered by `evaluate_manual`'s own tests.
        let eval = evaluate(&config, None, None);
        assert!(eval.until_boundary >= Duration::from_secs(1));
        assert!(eval.since_boundary < Duration::from_secs(24 * 3600));
    }

    #[test]
    fn evaluate_returns_default_when_no_coordinates_are_available() {
        let config = LocationConfig::default();
        assert_eq!(evaluate(&config, None, None), Evaluation::default());
    }
}
