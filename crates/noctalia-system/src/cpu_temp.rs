//! Port of `src/system/cpu_temp_sensor.{h,cpp}`.
//!
//! `read_small_text_file` is a minimal pull-forward of `FileUtils::readSmallTextFile`
//! (`src/util/file_utils.h`) — not the whole header (no owning task yet); same "minimal shared
//! piece" pattern as task 1.6.5's local `generate_uuid_v4`. `pub(crate)` since `intel_gpu.rs`
//! (task 5.6.3) is now a second real consumer — same promotion precedent as `brightness::c_trim`/
//! `icon_resolver::unquote`.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub temp_c: f64,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeResult {
    pub reading: Option<Reading>,
    pub error: String,
}

struct Sensor {
    hwmon_name: String,
    label: String,
    input_path: PathBuf,
    temp_c: f64,
    driver_priority: i32,
    sensor_priority: i32,
}

/// Port of `FileUtils::readSmallTextFile`: first line of the file, trailing `\n`/`\r`/space/tab
/// trimmed. `None` if the file can't be opened or its first line is empty even before trimming
/// (matches the C++'s `text.empty()` check running *before* the trim loop — a whitespace-only
/// line is `Some("")`, not `None`, since the trim runs after that check).
pub(crate) fn read_small_text_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let mut text = content.split('\n').next().unwrap_or("").to_string();
    if text.is_empty() {
        return None;
    }
    while text.ends_with(['\n', '\r', ' ', '\t']) {
        text.pop();
    }
    Some(text)
}

/// Port of `file >> raw` on a `long long`: skips leading whitespace, reads an optional sign then
/// a run of digits, and stops at the first non-digit rather than requiring the whole token to be
/// numeric (so trailing garbage after the number, e.g. a stray newline artifact, doesn't reject
/// an otherwise-valid reading the way a strict `str::parse` on the whole token would).
fn read_input_celsius(path: &Path) -> Option<f64> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim_start();
    let sign_len = usize::from(trimmed.starts_with('-') || trimmed.starts_with('+'));
    let digits_len = trimmed[sign_len..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits_len == 0 {
        return None;
    }
    let raw: i64 = trimmed[..sign_len + digits_len].parse().ok()?;
    if raw < 0 {
        return None;
    }
    if raw >= 1000 {
        Some(raw as f64 / 1000.0)
    } else {
        Some(raw as f64)
    }
}

fn is_temp_input_file_name(file_name: &str) -> bool {
    let Some(number) = file_name
        .strip_prefix("temp")
        .and_then(|s| s.strip_suffix("_input"))
    else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
}

fn temp_input_index(file_name: &str) -> i32 {
    if !is_temp_input_file_name(file_name) {
        return 0;
    }
    file_name
        .strip_prefix("temp")
        .and_then(|s| s.strip_suffix("_input"))
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0)
}

fn file_name_str(path: &Path) -> &str {
    path.file_name().and_then(|s| s.to_str()).unwrap_or("")
}

fn label_for_input_path(input_path: &Path) -> String {
    let file_name = file_name_str(input_path);
    let index = temp_input_index(file_name);
    let base = file_name.strip_suffix("_input").unwrap_or(file_name);
    let label_path = input_path.with_file_name(format!("{base}_label"));
    read_small_text_file(&label_path).unwrap_or_else(|| format!("temp{index}"))
}

fn format_hwmon_source(hwmon_name: &str, label: &str, input_path: &Path) -> String {
    let name = if hwmon_name.is_empty() {
        "unknown"
    } else {
        hwmon_name
    };
    if label.is_empty() {
        format!("hwmon:{name} {}", input_path.display())
    } else {
        format!("hwmon:{name} label=\"{label}\" {}", input_path.display())
    }
}

fn format_thermal_source(zone_type: &str, input_path: &Path) -> String {
    let zone_type = if zone_type.is_empty() {
        "unknown"
    } else {
        zone_type
    };
    format!("thermal_zone:{zone_type} {}", input_path.display())
}

fn known_driver_priority(hwmon_name: &str) -> i32 {
    match hwmon_name.to_ascii_lowercase().as_str() {
        "k10temp" => 0,
        "zenpower" => 1,
        "coretemp" => 2,
        "ibmpowernv" => 3,
        _ => -1,
    }
}

fn sensor_priority_for_driver(hwmon_name: &str, label: &str, input_index: i32) -> i32 {
    let name = hwmon_name.to_ascii_lowercase();
    let lower_label = label.to_ascii_lowercase();

    if name == "k10temp" || name == "zenpower" {
        if label.starts_with("Tctl") {
            return 0;
        }
        if input_index == 1 {
            return 1;
        }
        if label.starts_with("Tdie") {
            return 2;
        }
        if label.starts_with("Package id") {
            return 3;
        }
        if label.starts_with("SoC Temperature") {
            return 4;
        }
        if label.starts_with("Core") || label.starts_with("Tccd") {
            return 5;
        }
        return 20;
    }

    if name == "coretemp" {
        if label.starts_with("Package id") {
            return 0;
        }
        if input_index == 1 {
            return 1;
        }
        if label.starts_with("Core") {
            return 2;
        }
        return 20;
    }

    if name == "ibmpowernv" {
        if lower_label.contains("core") {
            return 0;
        }
        if input_index == 1 {
            return 1;
        }
        return 20;
    }

    100
}

/// Follows symlinks when deciding "is a directory" (matches `std::filesystem::directory_entry::
/// is_directory()`'s default symlink-following behavior) — load-bearing on a real system, since
/// `/sys/class/hwmon/hwmonN` entries are themselves symlinks into `/sys/devices/...`.
fn sorted_directories(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false))
        .collect();
    paths.sort();
    paths
}

fn read_known_hwmon_sensors(hwmon_root: &Path) -> Vec<Sensor> {
    let mut sensors = Vec::new();
    for hwmon_path in sorted_directories(hwmon_root) {
        let hwmon_name = read_small_text_file(&hwmon_path.join("name"))
            .unwrap_or_else(|| file_name_str(&hwmon_path).to_string());
        let driver_priority = known_driver_priority(&hwmon_name);
        if driver_priority < 0 {
            continue;
        }

        let mut input_paths: Vec<PathBuf> = fs::read_dir(&hwmon_path)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| is_temp_input_file_name(file_name_str(p)))
            .collect();
        input_paths.sort();

        for input_path in input_paths {
            let Some(temp_c) = read_input_celsius(&input_path) else {
                continue;
            };
            let input_index = temp_input_index(file_name_str(&input_path));
            let label = label_for_input_path(&input_path);
            sensors.push(Sensor {
                sensor_priority: sensor_priority_for_driver(&hwmon_name, &label, input_index),
                hwmon_name: hwmon_name.clone(),
                label,
                input_path,
                temp_c,
                driver_priority,
            });
        }
    }
    sensors
}

fn choose_known_hwmon_sensor(hwmon_root: &Path) -> Option<Reading> {
    let mut sensors = read_known_hwmon_sensors(hwmon_root);
    if sensors.is_empty() {
        return None;
    }

    let has_non_zero = sensors.iter().any(|s| s.temp_c > 0.0);
    if has_non_zero {
        sensors.retain(|s| s.temp_c > 0.0);
    }

    // input_path tie-break compares as strings (`Path`'s component-wise `Ord` can disagree with
    // string order across path separators), matching the C++'s `lhs.inputPath.string() <
    // rhs.inputPath.string()` exactly.
    let best = sensors.iter().min_by(|a, b| {
        a.driver_priority
            .cmp(&b.driver_priority)
            .then_with(|| a.sensor_priority.cmp(&b.sensor_priority))
            .then_with(|| {
                a.input_path
                    .to_string_lossy()
                    .cmp(&b.input_path.to_string_lossy())
            })
    })?;
    Some(Reading {
        temp_c: best.temp_c,
        source: format_hwmon_source(&best.hwmon_name, &best.label, &best.input_path),
    })
}

fn known_thermal_zone_priority(zone_type: &str) -> i32 {
    match zone_type {
        "cpu-thermal" => 0,
        "x86_pkg_temp" => 1,
        "acpitz" => 2,
        _ => -1,
    }
}

struct ThermalSensor {
    zone_type: String,
    input_path: PathBuf,
    temp_c: f64,
    priority: i32,
}

fn choose_thermal_zone_sensor(thermal_root: &Path) -> Option<Reading> {
    let mut sensors = Vec::new();
    for zone_path in sorted_directories(thermal_root) {
        let zone_type = read_small_text_file(&zone_path.join("type")).unwrap_or_default();
        let priority = known_thermal_zone_priority(&zone_type);
        if priority < 0 {
            continue;
        }

        let temp_path = zone_path.join("temp");
        let Some(temp_c) = read_input_celsius(&temp_path) else {
            continue;
        };
        sensors.push(ThermalSensor {
            zone_type,
            input_path: temp_path,
            temp_c,
            priority,
        });
    }

    if sensors.is_empty() {
        return None;
    }

    let has_non_zero = sensors.iter().any(|s| s.temp_c > 0.0);
    if has_non_zero {
        sensors.retain(|s| s.temp_c > 0.0);
    }

    let best = sensors.iter().min_by(|a, b| {
        a.priority.cmp(&b.priority).then_with(|| {
            a.input_path
                .to_string_lossy()
                .cmp(&b.input_path.to_string_lossy())
        })
    })?;
    Some(Reading {
        temp_c: best.temp_c,
        source: format_thermal_source(&best.zone_type, &best.input_path),
    })
}

fn read_configured_sensor(configured_path: &Path) -> ProbeResult {
    let file_name = file_name_str(configured_path);
    if !is_temp_input_file_name(file_name) {
        return ProbeResult {
            reading: None,
            error: format!(
                "configured CPU temperature sensor is not a temp*_input file: {}",
                configured_path.display()
            ),
        };
    }

    if !configured_path.exists() {
        return ProbeResult {
            reading: None,
            error: format!(
                "configured CPU temperature sensor does not exist: {}",
                configured_path.display()
            ),
        };
    }
    let is_regular_file = fs::metadata(configured_path)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !is_regular_file {
        return ProbeResult {
            reading: None,
            error: format!(
                "configured CPU temperature sensor is not readable as a regular file: {}",
                configured_path.display()
            ),
        };
    }

    let Some(temp_c) = read_input_celsius(configured_path) else {
        return ProbeResult {
            reading: None,
            error: format!(
                "configured CPU temperature sensor could not be parsed: {}",
                configured_path.display()
            ),
        };
    };

    let label = label_for_input_path(configured_path);
    let hwmon_name =
        read_small_text_file(&configured_path.with_file_name("name")).unwrap_or_default();
    ProbeResult {
        reading: Some(Reading {
            temp_c,
            source: format!(
                "configured {}",
                format_hwmon_source(&hwmon_name, &label, configured_path)
            ),
        }),
        error: String::new(),
    }
}

pub fn read(hwmon_root: &Path, thermal_root: &Path, configured_sensor_path: &str) -> ProbeResult {
    if !configured_sensor_path.is_empty() {
        return read_configured_sensor(Path::new(configured_sensor_path));
    }

    if let Some(reading) = choose_known_hwmon_sensor(hwmon_root) {
        return ProbeResult {
            reading: Some(reading),
            error: String::new(),
        };
    }

    if let Some(reading) = choose_thermal_zone_sensor(thermal_root) {
        return ProbeResult {
            reading: Some(reading),
            error: String::new(),
        };
    }

    ProbeResult {
        reading: None,
        error: "no CPU temperature sensor found".to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-cpu-temp-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_text(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("has parent")).expect("create parent");
        fs::write(path, text).expect("write fixture");
    }

    struct SensorSpec {
        index: i32,
        label: &'static str,
        raw: i64,
    }

    fn add_hwmon(hwmon_root: &Path, dir_name: &str, name: &str, sensors: &[SensorSpec]) -> PathBuf {
        let hwmon = hwmon_root.join(dir_name);
        write_text(&hwmon.join("name"), name);
        for sensor in sensors {
            let base = format!("temp{}", sensor.index);
            write_text(
                &hwmon.join(format!("{base}_input")),
                &sensor.raw.to_string(),
            );
            if !sensor.label.is_empty() {
                write_text(&hwmon.join(format!("{base}_label")), sensor.label);
            }
        }
        hwmon
    }

    fn add_thermal_zone(thermal_root: &Path, dir_name: &str, zone_type: &str, raw: i64) {
        let zone = thermal_root.join(dir_name);
        write_text(&zone.join("type"), zone_type);
        write_text(&zone.join("temp"), &raw.to_string());
    }

    fn read_fixture(root: &Path, configured_path: &str) -> ProbeResult {
        read(&root.join("hwmon"), &root.join("thermal"), configured_path)
    }

    fn expect_temp(result: &ProbeResult, expected: f64) {
        let reading = result
            .reading
            .as_ref()
            .unwrap_or_else(|| panic!("no reading; error={}", result.error));
        assert!(
            (reading.temp_c - expected).abs() < 0.001,
            "expected {expected}, got {}",
            reading.temp_c
        );
    }

    #[test]
    fn amd_prefers_tctl() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon3",
            "k10temp",
            &[
                SensorSpec {
                    index: 1,
                    label: "Tctl",
                    raw: 58000,
                },
                SensorSpec {
                    index: 3,
                    label: "Tccd1",
                    raw: 48000,
                },
                SensorSpec {
                    index: 4,
                    label: "Tccd2",
                    raw: 46000,
                },
            ],
        );

        let result = read_fixture(&root, "");
        expect_temp(&result, 58.0);
        assert!(result.reading.unwrap().source.contains("Tctl"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn motherboard_cpu_label_is_ignored() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon0",
            "nct6798",
            &[SensorSpec {
                index: 11,
                label: "PCH_CPU_TEMP",
                raw: 0,
            }],
        );
        add_hwmon(
            &root.join("hwmon"),
            "hwmon2",
            "k10temp",
            &[
                SensorSpec {
                    index: 1,
                    label: "Tctl",
                    raw: 58000,
                },
                SensorSpec {
                    index: 3,
                    label: "Tccd1",
                    raw: 48000,
                },
            ],
        );

        let result = read_fixture(&root, "");
        expect_temp(&result, 58.0);
        assert!(result.reading.unwrap().source.contains("k10temp"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn intel_package_wins() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon1",
            "coretemp",
            &[
                SensorSpec {
                    index: 1,
                    label: "Package id 0",
                    raw: 66000,
                },
                SensorSpec {
                    index: 2,
                    label: "Core 0",
                    raw: 55000,
                },
            ],
        );

        let result = read_fixture(&root, "");
        expect_temp(&result, 66.0);
        assert!(result.reading.unwrap().source.contains("Package id 0"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn manual_path_wins() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon2",
            "k10temp",
            &[SensorSpec {
                index: 1,
                label: "Tctl",
                raw: 58000,
            }],
        );
        let manual = add_hwmon(
            &root.join("hwmon"),
            "hwmon5",
            "custom",
            &[SensorSpec {
                index: 2,
                label: "Manual Sensor",
                raw: 42000,
            }],
        );

        let configured = manual.join("temp2_input");
        let result = read_fixture(&root, configured.to_str().expect("utf8 path"));
        expect_temp(&result, 42.0);
        assert!(result.reading.unwrap().source.contains("configured"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn manual_invalid_does_not_fall_back() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon2",
            "k10temp",
            &[SensorSpec {
                index: 1,
                label: "Tctl",
                raw: 58000,
            }],
        );

        let missing = root.join("hwmon").join("hwmon2").join("missing_input");
        let result = read_fixture(&root, missing.to_str().expect("utf8 path"));
        assert!(result.reading.is_none());
        assert!(result.error.contains("temp*_input"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zero_auto_candidate_is_skipped() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon2",
            "k10temp",
            &[
                SensorSpec {
                    index: 1,
                    label: "Tctl",
                    raw: 0,
                },
                SensorSpec {
                    index: 3,
                    label: "Tccd1",
                    raw: 48000,
                },
            ],
        );

        let result = read_fixture(&root, "");
        expect_temp(&result, 48.0);
        assert!(result.reading.unwrap().source.contains("Tccd1"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn thermal_fallback_when_no_known_hwmon() {
        let root = make_temp_dir();
        add_hwmon(
            &root.join("hwmon"),
            "hwmon0",
            "nct6798",
            &[SensorSpec {
                index: 11,
                label: "PCH_CPU_TEMP",
                raw: 0,
            }],
        );
        add_thermal_zone(
            &root.join("thermal"),
            "thermal_zone0",
            "x86_pkg_temp",
            61000,
        );

        let result = read_fixture(&root, "");
        expect_temp(&result, 61.0);
        assert!(result.reading.unwrap().source.contains("x86_pkg_temp"));
        let _ = fs::remove_dir_all(&root);
    }
}
