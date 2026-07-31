//! Port of the GPU sysfs/hwmon readers carved out of `system_monitor_service.{h,cpp}` (task
//! 5.6.5.2): AMD GPU discovery and stats via `/sys/class/drm/*/device` (no `dlopen`, unlike the
//! NVML/ROCm SMI readers in task 5.6.5.3), a vendor-agnostic `/sys/class/hwmon` temperature probe
//! that scores amdgpu/nvidia/i915/xe/nouveau sensors, and NVIDIA PCI display-device-state
//! detection via `/sys/bus/pci/devices`. Self-contained sysfs scanning, same precedent as
//! `intel_gpu.rs`/`rfkill_helper.rs`.
//!
//! Every scan root is a parameter (`drm_root`/`hwmon_root`/`pci_root`) rather than hardcoded, so
//! these are fixture-testable — same precedent as `intel_gpu::find_devices`'s `drm_root`
//! parameter. [`DEFAULT_DRM_ROOT`](crate::intel_gpu::DEFAULT_DRM_ROOT) is reused from
//! `intel_gpu.rs` for the real path; [`DEFAULT_HWMON_ROOT`]/[`DEFAULT_PCI_ROOT`] are this module's
//! own. `is_drm_card_name` is reused from `intel_gpu.rs` (promoted `pub(crate)` this session —
//! same "second real consumer" promotion precedent as `cpu_temp::read_small_text_file`).
//!
//! `merge_gpu_vram`/`has_usable_vram` are attributed to task 5.6.5.4 in MIGRATION_PLAN.md (the
//! cross-vendor VRAM-merging orchestration), but `read_amd_gpu_vram` below already needs
//! `merge_gpu_vram` to aggregate multiple AMD devices' VRAM into one reading, so they're
//! implemented here at their first real use and will be reused (not duplicated) once 5.6.5.4
//! needs them too.
//!
//! `read_temp_input_celsius`/`read_uint64_file` are local re-implementations of the same shape as
//! `cpu_temp::read_input_celsius`, not a shared promotion: the C++ itself has two independent
//! `readTempInputCelsius`s in separate anonymous namespaces (`cpu_temp_sensor.cpp` and
//! `system_monitor_service.cpp`) with different reject conditions (this file's rejects `raw <= 0`;
//! `cpu_temp_sensor`'s only rejects `raw < 0`), so porting them as two separate functions matches
//! the reference behavior instead of merging two things the C++ deliberately keeps distinct.
//!
//! No C++ test exists for these functions in isolation (confirmed: `system_monitor_service_test
//! .cpp`'s 2 assertions run with `gpuPollSeconds = 0`, exercising none of this) — task 5.6's own
//! done bar is fixture-driven tests. This dev host has no AMD or NVIDIA GPU (`lspci | grep -i
//! vga` shows only an Intel Meteor Lake Arc iGPU, confirmed same as task 5.6.3's precedent), so
//! these are fixture-only; no live smoke test is possible here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cpu_temp::read_small_text_file;
use crate::intel_gpu::is_drm_card_name;

/// The real `/sys/class/hwmon`, used by callers when no fixture root is given.
pub const DEFAULT_HWMON_ROOT: &str = "/sys/class/hwmon";
/// The real `/sys/bus/pci/devices`, used by callers when no fixture root is given.
pub const DEFAULT_PCI_ROOT: &str = "/sys/bus/pci/devices";

const AMD_PCI_VENDOR: &str = "0x1002";
const NVIDIA_PCI_VENDOR: &str = "0x10de";

/// Port of `TempSensorReading`.
#[derive(Debug, Clone, PartialEq)]
pub struct TempSensorReading {
    pub temp_c: f64,
    pub score: i32,
    pub source: String,
    pub is_nvidia: bool,
}

/// Port of `GpuHwmonProbe`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuHwmonProbe {
    pub reading: Option<TempSensorReading>,
    pub found_nvidia: bool,
}

/// Port of `GpuVramReading`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuVramReading {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub source: String,
    pub is_nvidia: bool,
}

/// Port of `SysfsGpuUsageReading`.
#[derive(Debug, Clone, PartialEq)]
pub struct SysfsGpuUsageReading {
    pub percent: f64,
    pub source: String,
}

/// Port of `AmdGpuSysfsDevice`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AmdGpuSysfsDevice {
    pub device_path: PathBuf,
    pub hwmon_path: PathBuf,
    pub has_busy: bool,
    pub has_temp: bool,
    pub has_vram: bool,
}

/// Port of `SystemMonitorService::NvidiaDisplayDeviceState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaDisplayDeviceState {
    None,
    InactiveOnly,
    Active,
}

/// Port of `hasUsableVram`.
pub fn has_usable_vram(reading: &GpuVramReading) -> bool {
    reading.total_bytes > 0 && reading.used_bytes <= reading.total_bytes
}

/// Port of `mergeGpuVram`.
pub fn merge_gpu_vram(target: &mut GpuVramReading, source: &GpuVramReading) {
    if !has_usable_vram(source) {
        return;
    }
    target.used_bytes += source.used_bytes;
    target.total_bytes += source.total_bytes;
    if target.source.is_empty() {
        target.source = source.source.clone();
    } else if !source.source.is_empty() {
        target.source.push_str(" + ");
        target.source.push_str(&source.source);
    }
    target.is_nvidia = target.is_nvidia || source.is_nvidia;
}

/// Port of `readTempInputCelsius` (the `system_monitor_service.cpp` anonymous-namespace copy,
/// which rejects `raw <= 0` — see the module doc comment for why this isn't shared with
/// `cpu_temp::read_input_celsius`).
fn read_temp_input_celsius(path: &Path) -> Option<f64> {
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
    if raw <= 0 {
        return None;
    }
    if raw >= 1000 {
        Some(raw as f64 / 1000.0)
    } else {
        Some(raw as f64)
    }
}

/// Port of `readUint64File`. Real sysfs counters are always non-negative decimal integers, so
/// (unlike `read_temp_input_celsius`) no sign is recognized — matching `file >> value` on an
/// unsigned type for the inputs this ever actually sees.
fn read_uint64_file(path: &Path) -> Option<u64> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim_start();
    let digits_len = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    trimmed[..digits_len].parse().ok()
}

fn format_hwmon_temp_source(hwmon_name: &str, label: &str, input_path: &Path) -> String {
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

/// Port of `isBetterHwmonSensor`.
pub fn is_better_hwmon_sensor(
    score: i32,
    temp_c: f64,
    best_score: i32,
    best_temp: Option<f64>,
) -> bool {
    score > best_score || (score == best_score && best_temp.is_none_or(|bt| temp_c > bt))
}

/// Port of `scoreGpuHwmonSensor`. `-1` means "not a recognized GPU hwmon driver name at all";
/// every recognized name's base score is non-negative, so a recognized driver never scores
/// negative regardless of label.
pub fn score_gpu_hwmon_sensor(hwmon_name: &str, label: &str) -> i32 {
    let name = hwmon_name.to_ascii_lowercase();
    let lbl = label.to_ascii_lowercase();

    let mut score = if name == "amdgpu"
        || name == "nvidia"
        || name.contains("nvidia")
        || name == "i915"
        || name == "xe"
    {
        20
    } else if name == "nouveau" {
        10
    } else {
        return -1;
    };

    if lbl.contains("junction") || lbl.contains("edge") {
        score += 30;
    } else if lbl.contains("gpu") || lbl.contains("mem") {
        score += 25;
    }

    score
}

/// Port of `isInactiveRuntimeStatus`.
fn is_inactive_runtime_status(status: &str) -> bool {
    let normalized = status.to_ascii_lowercase();
    normalized == "suspended" || normalized == "suspending"
}

/// Port of `isDeviceRuntimeSuspended`. A runtime-suspended GPU has no reading to give, and a
/// sysfs attribute that goes through the driver would resume it; `power/runtime_status` is served
/// by the PM core, so reading it never wakes the device. Devices without runtime PM report no
/// file at all and are treated as awake.
fn is_device_runtime_suspended(device_path: &Path) -> bool {
    read_small_text_file(&device_path.join("power").join("runtime_status"))
        .is_some_and(|status| is_inactive_runtime_status(&status))
}

/// Port of `isGpuHwmonAwake`.
fn is_gpu_hwmon_awake(hwmon_path: &Path) -> bool {
    let device_link = hwmon_path.join("device");
    if !device_link.exists() {
        return true;
    }
    !is_device_runtime_suspended(&device_link)
}

/// Port of `findAmdGpuHwmonPath`: the first subdirectory under `device_path/hwmon`, or an empty
/// path if there is none. Unsorted, matching the C++'s unsorted `directory_iterator` (a device
/// only ever has one hwmon child in practice, so ordering is moot).
fn find_amd_gpu_hwmon_path(device_path: &Path) -> PathBuf {
    let hwmon_dir = device_path.join("hwmon");
    let Ok(entries) = fs::read_dir(&hwmon_dir) else {
        return PathBuf::new();
    };
    for entry in entries.flatten() {
        if fs::metadata(entry.path())
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            return entry.path();
        }
    }
    PathBuf::new()
}

/// Port of `findAmdGpuSysfsDevices`.
pub fn find_amd_gpu_sysfs_devices(drm_root: &Path) -> Vec<AmdGpuSysfsDevice> {
    let Ok(entries) = fs::read_dir(drm_root) else {
        return Vec::new();
    };

    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = fs::metadata(entry.path())
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if !is_dir || !is_drm_card_name(&name) {
            continue;
        }

        let device_path = entry.path().join("device");
        if !device_path.exists() {
            continue;
        }

        if read_small_text_file(&device_path.join("vendor")).as_deref() != Some(AMD_PCI_VENDOR) {
            continue;
        }

        let Ok(driver_link) = fs::read_link(device_path.join("driver")) else {
            continue;
        };
        if driver_link.file_name().and_then(|n| n.to_str()) != Some("amdgpu") {
            continue;
        }

        // Skipping a suspended card leaves multi-GPU systems reading the one that is awake, which
        // is the one doing the rendering.
        if is_device_runtime_suspended(&device_path) {
            continue;
        }

        let hwmon_path = find_amd_gpu_hwmon_path(&device_path);
        let has_busy = device_path.join("gpu_busy_percent").exists();
        let has_vram = device_path.join("mem_info_vram_total").exists();
        let has_temp =
            !hwmon_path.as_os_str().is_empty() && hwmon_path.join("temp1_input").exists();

        if has_busy || has_vram || has_temp {
            devices.push(AmdGpuSysfsDevice {
                device_path,
                hwmon_path,
                has_busy,
                has_temp,
                has_vram,
            });
        }
    }

    devices
}

/// Port of `readAmdGpuSysfsUsage`.
pub fn read_amd_gpu_sysfs_usage(drm_root: &Path) -> Option<SysfsGpuUsageReading> {
    for device in find_amd_gpu_sysfs_devices(drm_root) {
        if !device.has_busy {
            continue;
        }
        let busy_path = device.device_path.join("gpu_busy_percent");
        let Some(value) = read_uint64_file(&busy_path) else {
            continue;
        };

        return Some(SysfsGpuUsageReading {
            percent: value.clamp(0, 100) as f64,
            source: format!("amdgpu sysfs:{}", busy_path.display()),
        });
    }

    None
}

/// Port of `readAmdGpuSysfsTempSensor`.
pub fn read_amd_gpu_sysfs_temp_sensor(drm_root: &Path) -> Option<TempSensorReading> {
    for device in find_amd_gpu_sysfs_devices(drm_root) {
        if !device.has_temp {
            continue;
        }

        let temp_path = device.hwmon_path.join("temp1_input");
        let Some(temp_c) = read_temp_input_celsius(&temp_path) else {
            continue;
        };

        return Some(TempSensorReading {
            temp_c,
            score: 0,
            source: format!("amdgpu sysfs:{}", temp_path.display()),
            is_nvidia: false,
        });
    }

    None
}

/// Port of `readAmdGpuVram`.
pub fn read_amd_gpu_vram(drm_root: &Path) -> Option<GpuVramReading> {
    let mut total = GpuVramReading::default();
    let mut device_count = 0u32;
    let mut first_source = String::new();

    for device in find_amd_gpu_sysfs_devices(drm_root) {
        if !device.has_vram {
            continue;
        }

        let used_path = device.device_path.join("mem_info_vram_used");
        let total_path = device.device_path.join("mem_info_vram_total");
        let Some(used) = read_uint64_file(&used_path) else {
            continue;
        };
        let Some(available) = read_uint64_file(&total_path) else {
            continue;
        };
        if available == 0 || used > available {
            continue;
        }

        device_count += 1;
        if first_source.is_empty() {
            first_source = used_path.display().to_string();
        }
        merge_gpu_vram(
            &mut total,
            &GpuVramReading {
                used_bytes: used,
                total_bytes: available,
                source: String::new(),
                is_nvidia: false,
            },
        );
    }

    if device_count == 0 || !has_usable_vram(&total) {
        return None;
    }

    total.source = if device_count == 1 {
        format!("amdgpu:{first_source}")
    } else {
        format!("amdgpu sysfs ({device_count} devices)")
    };
    Some(total)
}

/// Port of `readGpuHwmonTempSensor`.
pub fn read_gpu_hwmon_temp_sensor(hwmon_root: &Path) -> GpuHwmonProbe {
    let mut probe = GpuHwmonProbe::default();
    let Ok(hwmon_entries) = fs::read_dir(hwmon_root) else {
        return probe;
    };

    let mut best_score = -1;
    for hwmon_entry in hwmon_entries.flatten() {
        let hwmon_path = hwmon_entry.path();
        if !fs::metadata(&hwmon_path)
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            continue;
        }

        let hwmon_name = read_small_text_file(&hwmon_path.join("name")).unwrap_or_default();
        let name_score = score_gpu_hwmon_sensor(&hwmon_name, "");
        if name_score < 0 {
            continue;
        }

        let normalized_name = hwmon_name.to_ascii_lowercase();
        let is_nvidia = normalized_name == "nvidia" || normalized_name.contains("nvidia");

        if !is_gpu_hwmon_awake(&hwmon_path) {
            continue;
        }

        let Ok(file_entries) = fs::read_dir(&hwmon_path) else {
            continue;
        };
        for file_entry in file_entries.flatten() {
            let file_path = file_entry.path();
            if !fs::metadata(&file_path)
                .map(|m| m.is_file())
                .unwrap_or(false)
            {
                continue;
            }

            let file_name = file_entry.file_name().to_string_lossy().into_owned();
            if !file_name.starts_with("temp") || !file_name.ends_with("_input") {
                continue;
            }

            let base = &file_name[..file_name.len() - "_input".len()];
            let label =
                read_small_text_file(&hwmon_path.join(format!("{base}_label"))).unwrap_or_default();
            let Some(temp_c) = read_temp_input_celsius(&file_path) else {
                continue;
            };

            let score = score_gpu_hwmon_sensor(&hwmon_name, &label);
            if is_nvidia {
                probe.found_nvidia = true;
            }
            let best_temp = probe.reading.as_ref().map(|r| r.temp_c);
            if is_better_hwmon_sensor(score, temp_c, best_score, best_temp) {
                best_score = score;
                probe.reading = Some(TempSensorReading {
                    temp_c,
                    score,
                    source: format_hwmon_temp_source(&hwmon_name, &label, &file_path),
                    is_nvidia,
                });
            }
        }
    }

    probe
}

/// Port of `detectNvidiaPciDisplayDeviceState`.
pub fn detect_nvidia_pci_display_device_state(pci_root: &Path) -> NvidiaDisplayDeviceState {
    let Ok(entries) = fs::read_dir(pci_root) else {
        return NvidiaDisplayDeviceState::None;
    };

    let mut found_inactive_nvidia_display = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false) {
            continue;
        }

        let vendor = read_small_text_file(&path.join("vendor"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if vendor != NVIDIA_PCI_VENDOR {
            continue;
        }

        let device_class = read_small_text_file(&path.join("class"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !device_class.starts_with("0x03") {
            continue;
        }

        if is_device_runtime_suspended(&path) {
            found_inactive_nvidia_display = true;
            continue;
        }
        return NvidiaDisplayDeviceState::Active;
    }

    if found_inactive_nvidia_display {
        NvidiaDisplayDeviceState::InactiveOnly
    } else {
        NvidiaDisplayDeviceState::None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "noctalia-gpu-sysfs-test-{}-{name}",
            std::process::id()
        ))
    }

    // --- score_gpu_hwmon_sensor / is_better_hwmon_sensor ---

    #[test]
    fn score_gpu_hwmon_sensor_recognizes_known_drivers_and_rejects_others() {
        assert_eq!(score_gpu_hwmon_sensor("amdgpu", ""), 20);
        assert_eq!(score_gpu_hwmon_sensor("nvidia", ""), 20);
        assert_eq!(score_gpu_hwmon_sensor("NVIDIA-something", ""), 20);
        assert_eq!(score_gpu_hwmon_sensor("i915", ""), 20);
        assert_eq!(score_gpu_hwmon_sensor("xe", ""), 20);
        assert_eq!(score_gpu_hwmon_sensor("nouveau", ""), 10);
        assert_eq!(score_gpu_hwmon_sensor("k10temp", ""), -1);
    }

    #[test]
    fn score_gpu_hwmon_sensor_boosts_junction_edge_over_gpu_mem_labels() {
        assert_eq!(score_gpu_hwmon_sensor("amdgpu", "junction"), 20 + 30);
        assert_eq!(score_gpu_hwmon_sensor("amdgpu", "edge"), 20 + 30);
        assert_eq!(score_gpu_hwmon_sensor("amdgpu", "gpu"), 20 + 25);
        assert_eq!(score_gpu_hwmon_sensor("amdgpu", "mem"), 20 + 25);
        assert_eq!(score_gpu_hwmon_sensor("amdgpu", "unrelated"), 20);
    }

    #[test]
    fn is_better_hwmon_sensor_prefers_higher_score_then_higher_temp_on_tie() {
        assert!(is_better_hwmon_sensor(50, 10.0, 20, Some(99.0)));
        assert!(!is_better_hwmon_sensor(20, 10.0, 50, Some(1.0)));
        assert!(is_better_hwmon_sensor(50, 60.0, 50, Some(59.9)));
        assert!(!is_better_hwmon_sensor(50, 40.0, 50, Some(59.9)));
        assert!(is_better_hwmon_sensor(50, 10.0, 50, None));
    }

    // --- merge_gpu_vram / has_usable_vram ---

    #[test]
    fn has_usable_vram_rejects_zero_total_or_used_over_total() {
        assert!(!has_usable_vram(&GpuVramReading {
            used_bytes: 0,
            total_bytes: 0,
            ..Default::default()
        }));
        assert!(!has_usable_vram(&GpuVramReading {
            used_bytes: 10,
            total_bytes: 5,
            ..Default::default()
        }));
        assert!(has_usable_vram(&GpuVramReading {
            used_bytes: 5,
            total_bytes: 10,
            ..Default::default()
        }));
    }

    #[test]
    fn merge_gpu_vram_sums_bytes_and_joins_sources() {
        let mut target = GpuVramReading {
            used_bytes: 100,
            total_bytes: 1000,
            source: "first".to_string(),
            is_nvidia: false,
        };
        merge_gpu_vram(
            &mut target,
            &GpuVramReading {
                used_bytes: 50,
                total_bytes: 500,
                source: "second".to_string(),
                is_nvidia: true,
            },
        );
        assert_eq!(target.used_bytes, 150);
        assert_eq!(target.total_bytes, 1500);
        assert_eq!(target.source, "first + second");
        assert!(target.is_nvidia);
    }

    #[test]
    fn merge_gpu_vram_ignores_an_unusable_source() {
        let mut target = GpuVramReading {
            used_bytes: 100,
            total_bytes: 1000,
            source: "first".to_string(),
            is_nvidia: false,
        };
        merge_gpu_vram(
            &mut target,
            &GpuVramReading {
                used_bytes: 999,
                total_bytes: 0,
                source: "bogus".to_string(),
                is_nvidia: false,
            },
        );
        assert_eq!(target.used_bytes, 100);
        assert_eq!(target.total_bytes, 1000);
        assert_eq!(target.source, "first");
    }

    // --- AMD sysfs device discovery / usage / temp / vram ---

    #[allow(clippy::too_many_arguments)]
    fn write_amd_card(
        drm_root: &Path,
        card: &str,
        vendor: &str,
        driver: &str,
        suspended: bool,
        busy_percent: Option<u64>,
        hwmon_temp_milli: Option<i64>,
        vram_used: Option<u64>,
        vram_total: Option<u64>,
    ) {
        let card_dir = drm_root.join(card);
        fs::create_dir_all(&card_dir).expect("create card dir");

        let pci_target = drm_root.join("_devices").join(card);
        fs::create_dir_all(&pci_target).expect("create pci target dir");
        fs::write(pci_target.join("vendor"), format!("{vendor}\n")).expect("write vendor");
        if suspended {
            fs::create_dir_all(pci_target.join("power")).expect("create power dir");
            fs::write(
                pci_target.join("power").join("runtime_status"),
                "suspended\n",
            )
            .expect("write runtime_status");
        }

        let driver_target = drm_root.join("_drivers").join(driver);
        fs::create_dir_all(&driver_target).expect("create driver target dir");
        std::os::unix::fs::symlink(&driver_target, pci_target.join("driver"))
            .expect("symlink driver");

        std::os::unix::fs::symlink(&pci_target, card_dir.join("device")).expect("symlink device");

        if let Some(percent) = busy_percent {
            fs::write(pci_target.join("gpu_busy_percent"), percent.to_string())
                .expect("write busy percent");
        }
        if let Some(used) = vram_used {
            fs::write(pci_target.join("mem_info_vram_used"), used.to_string())
                .expect("write vram used");
        }
        if let Some(total) = vram_total {
            fs::write(pci_target.join("mem_info_vram_total"), total.to_string())
                .expect("write vram total");
        }
        if let Some(milli) = hwmon_temp_milli {
            let hwmon_dir = pci_target.join("hwmon").join("hwmon0");
            fs::create_dir_all(&hwmon_dir).expect("create hwmon dir");
            fs::write(hwmon_dir.join("temp1_input"), milli.to_string()).expect("write hwmon temp");
        }
    }

    #[test]
    fn find_amd_gpu_sysfs_devices_filters_vendor_driver_and_suspended_state() {
        let dir = fixture_dir("find-devices");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            Some(42),
            Some(55000),
            Some(1_000_000),
            Some(2_000_000),
        );
        write_amd_card(
            &dir,
            "card1",
            "0x8086",
            "amdgpu",
            false,
            Some(1),
            None,
            None,
            None,
        );
        write_amd_card(
            &dir,
            "card2",
            "0x1002",
            "radeon",
            false,
            Some(1),
            None,
            None,
            None,
        );
        write_amd_card(
            &dir,
            "card3",
            "0x1002",
            "amdgpu",
            true,
            Some(1),
            None,
            None,
            None,
        );

        let devices = find_amd_gpu_sysfs_devices(&dir);
        assert_eq!(
            devices.len(),
            1,
            "wrong vendor, wrong driver, and a suspended card should all be excluded"
        );
        assert!(devices[0].has_busy);
        assert!(devices[0].has_temp);
        assert!(devices[0].has_vram);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_amd_gpu_sysfs_devices_on_missing_directory_is_empty() {
        assert!(find_amd_gpu_sysfs_devices(Path::new("/definitely/not/a/real/drm/dir")).is_empty());
    }

    #[test]
    fn read_amd_gpu_sysfs_usage_clamps_and_formats_source() {
        let dir = fixture_dir("usage");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            Some(150),
            None,
            None,
            None,
        );

        let reading = read_amd_gpu_sysfs_usage(&dir).expect("should read usage");
        assert_eq!(reading.percent, 100.0, "150% should clamp to 100");
        assert!(reading.source.starts_with("amdgpu sysfs:"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_amd_gpu_sysfs_usage_skips_devices_without_a_busy_file() {
        let dir = fixture_dir("usage-none");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            None,
            Some(50000),
            None,
            None,
        );
        assert!(read_amd_gpu_sysfs_usage(&dir).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_amd_gpu_sysfs_temp_sensor_converts_millidegrees() {
        let dir = fixture_dir("temp");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            None,
            Some(62500),
            None,
            None,
        );

        let reading = read_amd_gpu_sysfs_temp_sensor(&dir).expect("should read temp");
        assert!((reading.temp_c - 62.5).abs() < 1e-9);
        assert!(reading.source.starts_with("amdgpu sysfs:"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_amd_gpu_vram_aggregates_multiple_devices() {
        let dir = fixture_dir("vram-multi");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            None,
            None,
            Some(1_000_000),
            Some(4_000_000),
        );
        write_amd_card(
            &dir,
            "card1",
            "0x1002",
            "amdgpu",
            false,
            None,
            None,
            Some(500_000),
            Some(2_000_000),
        );

        let reading = read_amd_gpu_vram(&dir).expect("should aggregate vram");
        assert_eq!(reading.used_bytes, 1_500_000);
        assert_eq!(reading.total_bytes, 6_000_000);
        assert_eq!(reading.source, "amdgpu sysfs (2 devices)");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_amd_gpu_vram_single_device_names_the_source_path() {
        let dir = fixture_dir("vram-single");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            None,
            None,
            Some(1_000_000),
            Some(4_000_000),
        );

        let reading = read_amd_gpu_vram(&dir).expect("should read vram");
        assert!(reading.source.starts_with("amdgpu:"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_amd_gpu_vram_rejects_used_greater_than_total() {
        let dir = fixture_dir("vram-invalid");
        write_amd_card(
            &dir,
            "card0",
            "0x1002",
            "amdgpu",
            false,
            None,
            None,
            Some(5_000_000),
            Some(1_000_000),
        );
        assert!(read_amd_gpu_vram(&dir).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    // --- read_gpu_hwmon_temp_sensor ---

    fn write_hwmon(
        hwmon_root: &Path,
        dir_name: &str,
        name: &str,
        temp_index: u32,
        label: Option<&str>,
        milli: i64,
        device_suspended: Option<bool>,
    ) {
        let hwmon_dir = hwmon_root.join(dir_name);
        fs::create_dir_all(&hwmon_dir).expect("create hwmon dir");
        fs::write(hwmon_dir.join("name"), name).expect("write name");
        fs::write(
            hwmon_dir.join(format!("temp{temp_index}_input")),
            milli.to_string(),
        )
        .expect("write temp input");
        if let Some(label) = label {
            fs::write(hwmon_dir.join(format!("temp{temp_index}_label")), label)
                .expect("write label");
        }
        if let Some(suspended) = device_suspended {
            let device_dir = hwmon_dir.join("device");
            fs::create_dir_all(device_dir.join("power")).expect("create device power dir");
            let status = if suspended { "suspended" } else { "active" };
            fs::write(device_dir.join("power").join("runtime_status"), status)
                .expect("write device runtime_status");
        }
    }

    #[test]
    fn read_gpu_hwmon_temp_sensor_prefers_junction_label_over_plain_gpu_hwmon() {
        let dir = fixture_dir("hwmon-prefer-junction");
        write_hwmon(&dir, "hwmon0", "k10temp", 1, Some("Tctl"), 55000, None);
        write_hwmon(&dir, "hwmon1", "amdgpu", 1, None, 40000, None);
        write_hwmon(&dir, "hwmon2", "amdgpu", 2, Some("junction"), 70000, None);

        let probe = read_gpu_hwmon_temp_sensor(&dir);
        let reading = probe.reading.expect("should find a reading");
        assert!((reading.temp_c - 70.0).abs() < 1e-9);
        assert!(reading.source.contains("junction"));
        assert!(!probe.found_nvidia);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_gpu_hwmon_temp_sensor_flags_nvidia_when_its_device_is_awake() {
        let dir = fixture_dir("hwmon-nvidia-awake");
        write_hwmon(&dir, "hwmon0", "nvidia", 1, None, 65000, Some(false));
        write_hwmon(&dir, "hwmon1", "amdgpu", 1, Some("edge"), 50000, None);

        let probe = read_gpu_hwmon_temp_sensor(&dir);
        assert!(
            probe.found_nvidia,
            "an awake nvidia hwmon should be flagged even if it doesn't win the best reading"
        );
        let reading = probe.reading.expect("should find the best-scoring reading");
        assert!(
            (reading.temp_c - 50.0).abs() < 1e-9,
            "amdgpu edge beats nvidia's unlabeled sensor"
        );
        assert!(!reading.is_nvidia);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_gpu_hwmon_temp_sensor_does_not_flag_nvidia_when_its_device_is_suspended() {
        // The awake check runs before the per-file loop that sets `found_nvidia`, so a suspended
        // nvidia hwmon is skipped in its entirety — it never gets a chance to set the flag. This
        // matches the C++'s `isGpuHwmonAwake` gate ordering exactly.
        let dir = fixture_dir("hwmon-nvidia-suspended");
        write_hwmon(&dir, "hwmon0", "nvidia", 1, None, 65000, Some(true));
        write_hwmon(&dir, "hwmon1", "amdgpu", 1, Some("edge"), 50000, None);

        let probe = read_gpu_hwmon_temp_sensor(&dir);
        assert!(!probe.found_nvidia);
        let reading = probe
            .reading
            .expect("should fall back to the amdgpu reading");
        assert!((reading.temp_c - 50.0).abs() < 1e-9);
        assert!(!reading.is_nvidia);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_gpu_hwmon_temp_sensor_on_missing_root_is_empty_probe() {
        let probe = read_gpu_hwmon_temp_sensor(Path::new("/definitely/not/a/real/hwmon/dir"));
        assert_eq!(probe, GpuHwmonProbe::default());
    }

    // --- detect_nvidia_pci_display_device_state ---

    fn write_pci_device(
        pci_root: &Path,
        slot: &str,
        vendor: &str,
        class: &str,
        runtime_status: Option<&str>,
    ) {
        let device_dir = pci_root.join(slot);
        fs::create_dir_all(&device_dir).expect("create pci device dir");
        fs::write(device_dir.join("vendor"), vendor).expect("write vendor");
        fs::write(device_dir.join("class"), class).expect("write class");
        if let Some(status) = runtime_status {
            fs::create_dir_all(device_dir.join("power")).expect("create power dir");
            fs::write(device_dir.join("power").join("runtime_status"), status)
                .expect("write runtime_status");
        }
    }

    #[test]
    fn detect_nvidia_pci_display_device_state_active_when_an_awake_display_device_exists() {
        let dir = fixture_dir("pci-active");
        write_pci_device(&dir, "0000:01:00.0", "0x10de", "0x030000", None);
        assert_eq!(
            detect_nvidia_pci_display_device_state(&dir),
            NvidiaDisplayDeviceState::Active
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_nvidia_pci_display_device_state_inactive_only_when_suspended() {
        let dir = fixture_dir("pci-inactive");
        write_pci_device(
            &dir,
            "0000:01:00.0",
            "0x10de",
            "0x030000",
            Some("suspended"),
        );
        assert_eq!(
            detect_nvidia_pci_display_device_state(&dir),
            NvidiaDisplayDeviceState::InactiveOnly
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_nvidia_pci_display_device_state_ignores_non_nvidia_and_non_display_class() {
        let dir = fixture_dir("pci-none");
        write_pci_device(&dir, "0000:01:00.0", "0x1002", "0x030000", None);
        write_pci_device(&dir, "0000:02:00.0", "0x10de", "0x060000", None);
        assert_eq!(
            detect_nvidia_pci_display_device_state(&dir),
            NvidiaDisplayDeviceState::None
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_nvidia_pci_display_device_state_active_wins_over_an_earlier_inactive_one() {
        let dir = fixture_dir("pci-mixed");
        write_pci_device(
            &dir,
            "0000:01:00.0",
            "0x10de",
            "0x030000",
            Some("suspended"),
        );
        write_pci_device(&dir, "0000:02:00.0", "0x10de", "0x030000", None);
        assert_eq!(
            detect_nvidia_pci_display_device_state(&dir),
            NvidiaDisplayDeviceState::Active
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_nvidia_pci_display_device_state_on_missing_root_is_none() {
        assert_eq!(
            detect_nvidia_pci_display_device_state(Path::new("/definitely/not/a/real/pci/dir")),
            NvidiaDisplayDeviceState::None
        );
    }
}
