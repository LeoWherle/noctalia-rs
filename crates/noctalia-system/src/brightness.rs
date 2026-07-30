//! Port of the dependency-free slice of `src/system/brightness_service.cpp` (task 5.3.1): sysfs
//! raw-value reading/mapping and backlight-candidate ranking.
//!
//! The rest of `BrightnessService` — DDC/CI (`ddcutil`) support (task 5.3.2), and the
//! Wayland-connector-attributed enumeration, worker thread, logind `SetBrightness`, and IPC/poll
//! wiring (task 5.3.3, blocked on Phase 6/9/10 landing) — is not ported here. See
//! MIGRATION_PLAN.md task 5.3's split for the full breakdown.

use std::fs;
use std::path::{Path, PathBuf};

/// Characters `std::isspace` (the "C" locale, used by the C++'s `StringUtils::trim`) treats as
/// whitespace: space, tab, newline, vertical tab, form feed, carriage return. `char::is_ascii_
/// whitespace` is close but omits vertical tab, so this is spelled out explicitly rather than
/// reused.
const C_ISSPACE: [char; 6] = [' ', '\t', '\n', '\x0B', '\x0C', '\r'];

fn is_c_isspace_byte(b: u8) -> bool {
    C_ISSPACE.iter().any(|&c| c as u32 == u32::from(b))
}

/// Port of `readSysfsInt`: `file >> value` on an `int`, pre-initialized to `-1`. `operator>>`'s
/// `sentry` only touches `value` once it finds a non-whitespace byte before EOF; a file that's
/// empty, unopenable, or holds only whitespace all leave the `-1` pre-init untouched. Once a
/// non-whitespace byte is found, `num_get::get` either parses a valid integer or — for anything
/// else (a bare sign, non-numeric text) — sets `value` to `0`. Verified against real
/// `istream`/`ifstream` behavior (empty file, whitespace-only file, garbage, bare sign, vertical
/// tab before a digit), not just the general rule. Out-of-range values clamp to `i32::MIN`/`MAX`,
/// matching `operator>>`'s overflow behavior (sets the value to the type's min/max and fails).
fn read_sysfs_int(path: &Path) -> i32 {
    let Ok(bytes) = fs::read(path) else {
        return -1;
    };

    let mut i = 0;
    while i < bytes.len() && is_c_isspace_byte(bytes[i]) {
        i += 1;
    }
    if i == bytes.len() {
        // No non-whitespace byte before EOF: the sentry fails without touching `value`.
        return -1;
    }

    let negative = bytes[i] == b'-';
    if negative || bytes[i] == b'+' {
        i += 1;
    }

    let mut value: i64 = 0;
    let mut saw_digit = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        saw_digit = true;
        // Sysfs integer files are tiny; clamping happens once at the end rather than mid-loop,
        // so this can't meaningfully overflow `i64` before the final clamp runs.
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(bytes[i] - b'0'));
        i += 1;
    }
    if !saw_digit {
        // A non-whitespace byte was found but no valid integer followed (e.g. a bare sign, or
        // non-numeric text): num_get::get sets the target to 0.
        return 0;
    }

    let value = if negative { -value } else { value };
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Port of the free function `normalizedBrightness`: `[0.0, 1.0]`, `0.0` for a negative/missing
/// raw reading or a non-positive `max_raw`.
pub fn normalized_brightness(current_raw: i32, max_raw: i32) -> f32 {
    if current_raw < 0 || max_raw <= 0 {
        return 0.0;
    }
    (current_raw as f32 / max_raw as f32).clamp(0.0, 1.0)
}

/// Port of `readBacklightBrightness`.
fn read_backlight_brightness(sysfs_path: &Path, max_raw: i32) -> f32 {
    let requested = read_sysfs_int(&sysfs_path.join("brightness"));
    normalized_brightness(requested, max_raw)
}

/// Port of `readBacklightType`: the sysfs device's `type` file, first line, trimmed and
/// lowercased. Empty when the file can't be opened.
fn read_backlight_type(sysfs_path: &Path) -> String {
    let Ok(content) = fs::read_to_string(sysfs_path.join("type")) else {
        return String::new();
    };
    // `std::getline` reads only the first line (stopping at `\n`, which it discards).
    let first_line = content.split('\n').next().unwrap_or("");
    first_line
        .trim_matches(C_ISSPACE.as_slice())
        .to_ascii_lowercase()
}

/// Port of `extractBacklightDeviceName`: the sysfs device name from either a bare name
/// (`"intel_backlight"`) or a path, taking everything after the last `/`.
pub fn extract_backlight_device_name(device_spec: &str) -> &str {
    match device_spec.rfind('/') {
        Some(last_slash) => &device_spec[last_slash + 1..],
        None => device_spec,
    }
}

/// Port of `backlightTypeRank`: lower ranks are preferred (`"raw"` first, everything unrecognized
/// last).
pub fn backlight_type_rank(kind: &str) -> i32 {
    match kind {
        "raw" => 0,
        "platform" => 1,
        "firmware" => 2,
        _ => 3,
    }
}

/// Port of `backlightNamePenalty`: a same-ranked device named like a GPU-vendor or ACPI fallback
/// backlight is penalized relative to a plain platform one.
pub fn backlight_name_penalty(name: &str) -> i32 {
    if name.starts_with("nvidia") {
        2
    } else if name.starts_with("acpi_video") {
        1
    } else {
        0
    }
}

/// Minimal stand-in for the C++'s `BacklightCandidate` (which wraps a full `DisplayInternal`):
/// only the fields `isBetterBacklightCandidate` actually reads. The Wayland-connector-attributed
/// candidate this will wrap once enumeration is connector-aware lands in task 5.3.3.
#[derive(Debug, Clone, PartialEq)]
pub struct BacklightCandidate {
    pub exact_drm_match: bool,
    pub kind: String,
    pub backlight_name: String,
    pub max_raw: i32,
}

/// Port of `isBetterBacklightCandidate`: `true` when `next` should replace `current` as the
/// chosen backlight for a connector. Preference order: an exact DRM-connector match beats a
/// fallback match, then lower `backlight_type_rank`, then lower `backlight_name_penalty`, then
/// higher `max_raw` (finer-grained device), then — as a final, arbitrary but deterministic
/// tie-break — the *lexicographically smaller* device name (`next.name < current.name`, matching
/// the C++'s `return next.display.backlightName < current.display.backlightName;` exactly).
pub fn is_better_backlight_candidate(
    current: &BacklightCandidate,
    next: &BacklightCandidate,
) -> bool {
    if current.exact_drm_match != next.exact_drm_match {
        return next.exact_drm_match;
    }

    let current_type_rank = backlight_type_rank(&current.kind);
    let next_type_rank = backlight_type_rank(&next.kind);
    if current_type_rank != next_type_rank {
        return next_type_rank < current_type_rank;
    }

    let current_penalty = backlight_name_penalty(&current.backlight_name);
    let next_penalty = backlight_name_penalty(&next.backlight_name);
    if current_penalty != next_penalty {
        return next_penalty < current_penalty;
    }

    if current.max_raw != next.max_raw {
        return next.max_raw > current.max_raw;
    }

    next.backlight_name < current.backlight_name
}

/// A backlight device found under `/sys/class/backlight`, before any Wayland-connector
/// attribution (task 5.3.3 adds that layer on top).
#[derive(Debug, Clone, PartialEq)]
pub struct BacklightDevice {
    pub name: String,
    pub sysfs_path: PathBuf,
    pub max_raw: i32,
    pub brightness: f32,
    pub kind: String,
}

/// Port of the sysfs-only half of `enumerateBacklights` — everything up to (not including) the
/// Wayland-connector matching that decides whether a device is kept. Devices reporting a
/// non-positive `max_brightness` are skipped, matching the C++'s `if (maxBrightness <= 0)
/// continue;` guard. Empty (not an error) when `backlight_dir` doesn't exist or holds no
/// candidates, matching the C++'s `opendir` failure returning early with just a debug log.
pub fn scan_backlight_devices(backlight_dir: &Path) -> Vec<BacklightDevice> {
    let Ok(entries) = fs::read_dir(backlight_dir) else {
        return Vec::new();
    };

    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let sysfs_path = entry.path();

        let max_raw = read_sysfs_int(&sysfs_path.join("max_brightness"));
        if max_raw <= 0 {
            continue;
        }

        devices.push(BacklightDevice {
            brightness: read_backlight_brightness(&sysfs_path, max_raw),
            kind: read_backlight_type(&sysfs_path),
            name,
            sysfs_path,
            max_raw,
        });
    }

    devices
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-brightness-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_backlight_device(
        root: &Path,
        name: &str,
        max_brightness: &str,
        brightness: &str,
        kind: Option<&str>,
    ) {
        let dev_dir = root.join(name);
        fs::create_dir_all(&dev_dir).expect("create device dir");
        let mut max_file =
            fs::File::create(dev_dir.join("max_brightness")).expect("create max_brightness");
        max_file
            .write_all(max_brightness.as_bytes())
            .expect("write max_brightness");
        let mut cur_file = fs::File::create(dev_dir.join("brightness")).expect("create brightness");
        cur_file
            .write_all(brightness.as_bytes())
            .expect("write brightness");
        if let Some(kind) = kind {
            let mut type_file = fs::File::create(dev_dir.join("type")).expect("create type");
            type_file.write_all(kind.as_bytes()).expect("write type");
        }
    }

    #[test]
    fn read_sysfs_int_distinguishes_missing_file_from_unparseable_content() {
        let dir = tempfile_dir();
        assert_eq!(
            read_sysfs_int(&dir.join("does-not-exist")),
            -1,
            "a file that can't be opened at all should read as -1"
        );

        let empty = dir.join("empty");
        fs::File::create(&empty).expect("create empty file");
        assert_eq!(
            read_sysfs_int(&empty),
            -1,
            "an empty file has no non-whitespace byte: the sentry fails without touching value, \
             leaving the -1 pre-init untouched (verified against real istream behavior)"
        );

        let whitespace_only = dir.join("whitespace-only");
        fs::write(&whitespace_only, b"  \n\t").expect("write whitespace-only");
        assert_eq!(
            read_sysfs_int(&whitespace_only),
            -1,
            "a whitespace-only file is the same sentry-failure case as an empty file"
        );

        let garbage = dir.join("garbage");
        fs::write(&garbage, b"not-a-number\n").expect("write garbage");
        assert_eq!(
            read_sysfs_int(&garbage),
            0,
            "a non-whitespace byte was found (so the sentry succeeds) but no integer parses: 0"
        );

        let bare_sign = dir.join("bare-sign");
        fs::write(&bare_sign, b"-\n").expect("write bare sign");
        assert_eq!(
            read_sysfs_int(&bare_sign),
            0,
            "a lone sign with no digits should read as 0"
        );

        let value = dir.join("value");
        fs::write(&value, b"  255\n").expect("write value");
        assert_eq!(
            read_sysfs_int(&value),
            255,
            "leading whitespace should be skipped like operator>>"
        );

        let vertical_tab = dir.join("vertical-tab");
        fs::write(&vertical_tab, b"\x0B42").expect("write vertical-tab-prefixed value");
        assert_eq!(
            read_sysfs_int(&vertical_tab),
            42,
            "a leading vertical tab is C isspace and should be skipped like operator>>, even \
             though it isn't ASCII whitespace by Rust's definition"
        );

        let negative = dir.join("negative");
        fs::write(&negative, b"-5").expect("write negative");
        assert_eq!(read_sysfs_int(&negative), -5);

        let trailing_garbage = dir.join("trailing_garbage");
        fs::write(&trailing_garbage, b"42abc").expect("write trailing garbage");
        assert_eq!(
            read_sysfs_int(&trailing_garbage),
            42,
            "operator>> stops at the first non-digit rather than rejecting the whole token"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalized_brightness_clamps_and_handles_missing_values() {
        assert_eq!(normalized_brightness(50, 100), 0.5);
        assert_eq!(
            normalized_brightness(-1, 100),
            0.0,
            "a negative raw reading should be 0"
        );
        assert_eq!(
            normalized_brightness(50, 0),
            0.0,
            "a non-positive max should be 0"
        );
        assert_eq!(
            normalized_brightness(150, 100),
            1.0,
            "over-max should clamp to 1.0"
        );
    }

    #[test]
    fn extract_backlight_device_name_handles_bare_names_and_paths() {
        assert_eq!(
            extract_backlight_device_name("intel_backlight"),
            "intel_backlight"
        );
        assert_eq!(
            extract_backlight_device_name("/sys/class/backlight/intel_backlight"),
            "intel_backlight"
        );
    }

    #[test]
    fn backlight_type_rank_orders_raw_before_platform_before_firmware_before_unknown() {
        assert!(backlight_type_rank("raw") < backlight_type_rank("platform"));
        assert!(backlight_type_rank("platform") < backlight_type_rank("firmware"));
        assert!(backlight_type_rank("firmware") < backlight_type_rank("something-else"));
    }

    #[test]
    fn backlight_name_penalty_flags_nvidia_and_acpi_video() {
        assert_eq!(backlight_name_penalty("nvidia_0"), 2);
        assert_eq!(backlight_name_penalty("acpi_video0"), 1);
        assert_eq!(backlight_name_penalty("intel_backlight"), 0);
    }

    fn candidate(
        exact_drm_match: bool,
        kind: &str,
        name: &str,
        max_raw: i32,
    ) -> BacklightCandidate {
        BacklightCandidate {
            exact_drm_match,
            kind: kind.to_string(),
            backlight_name: name.to_string(),
            max_raw,
        }
    }

    #[test]
    fn is_better_backlight_candidate_prefers_exact_drm_match_first() {
        let current = candidate(false, "raw", "a", 100);
        let next = candidate(true, "firmware", "z", 1);
        assert!(
            is_better_backlight_candidate(&current, &next),
            "an exact DRM match should win even against a worse type/name/range"
        );
        assert!(!is_better_backlight_candidate(&next, &current));
    }

    #[test]
    fn is_better_backlight_candidate_then_prefers_lower_type_rank() {
        let current = candidate(true, "firmware", "a", 100);
        let next = candidate(true, "raw", "z", 1);
        assert!(is_better_backlight_candidate(&current, &next));
    }

    #[test]
    fn is_better_backlight_candidate_then_prefers_lower_name_penalty() {
        let current = candidate(true, "raw", "nvidia_0", 100);
        let next = candidate(true, "raw", "intel_backlight", 100);
        assert!(is_better_backlight_candidate(&current, &next));
    }

    #[test]
    fn is_better_backlight_candidate_then_prefers_higher_max_raw() {
        let current = candidate(true, "raw", "a", 100);
        let next = candidate(true, "raw", "b", 65535);
        assert!(is_better_backlight_candidate(&current, &next));
    }

    #[test]
    fn is_better_backlight_candidate_finally_tie_breaks_on_lexicographically_smaller_name() {
        let current = candidate(true, "raw", "intel_backlight", 100);
        let next = candidate(true, "raw", "acpi_backlight", 100);
        assert!(
            is_better_backlight_candidate(&current, &next),
            "\"acpi_backlight\" < \"intel_backlight\" lexicographically, so next should win"
        );
        assert!(!is_better_backlight_candidate(&next, &current));
    }

    #[test]
    fn scan_backlight_devices_skips_zero_max_and_reads_type() {
        let dir = tempfile_dir();
        write_backlight_device(&dir, "intel_backlight", "1000", "500", Some("raw\n"));
        write_backlight_device(&dir, "acpi_video0", "0", "0", Some("firmware\n"));
        write_backlight_device(&dir, "no_type", "255", "255", None);

        let mut devices = scan_backlight_devices(&dir);
        devices.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(devices.len(), 2, "the zero-max device should be skipped");
        assert_eq!(devices[0].name, "intel_backlight");
        assert_eq!(devices[0].max_raw, 1000);
        assert_eq!(devices[0].brightness, 0.5);
        assert_eq!(devices[0].kind, "raw");
        assert_eq!(devices[1].name, "no_type");
        assert_eq!(
            devices[1].kind, "",
            "a missing type file should read as empty"
        );
        assert_eq!(devices[1].brightness, 1.0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_backlight_devices_on_missing_directory_is_empty() {
        assert!(scan_backlight_devices(Path::new("/does/not/exist")).is_empty());
    }
}
