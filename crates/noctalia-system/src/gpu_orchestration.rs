//! Port of the GPU vendor-priority orchestration carved out of `system_monitor_service.{h,cpp}`
//! (task 5.6.5.4): `readGpuTempData`/`readGpuUsageData`/`readGpuVramData` — the decision tree that
//! picks which reader wins based on [`NvidiaDisplayDeviceState`] (reused from
//! [`crate::gpu_sysfs`], task 5.6.5.2) and each reader's availability — plus the `IntelGpuReader`
//! wrapper struct that stable-partitions discrete-before-integrated Intel devices and owns their
//! per-device `UsageSampler`s.
//!
//! `mergeGpuVram`/`hasUsableVram` were already implemented in `gpu_sysfs.rs` (task 5.6.5.2, at
//! their first real use in `read_amd_gpu_vram`'s multi-device aggregation) and are reused here,
//! not duplicated.
//!
//! The three lower-level readers ([`NvidiaNvmlReader`](crate::gpu_nvml::NvidiaNvmlReader),
//! [`AmdRsmiReader`](crate::gpu_rsmi::AmdRsmiReader), and this module's own [`IntelGpuReader`])
//! each dlopen a vendor library or scan a real DRM device tree, so they can't be fixture-driven
//! for decision-tree testing the way `gpu_sysfs.rs`'s pure sysfs scans can. Instead, the decision
//! tree itself is written against the [`NvmlGpuSource`]/[`RsmiGpuSource`]/[`IntelGpuSource`]
//! traits (implemented for the real readers by trivial delegation below), so tests substitute
//! small fakes and exercise every branch — which reader wins per [`NvidiaDisplayDeviceState`]
//! value and per reader-availability combination — without needing real GPU hardware. The
//! sysfs/hwmon calls the tree also makes (`read_amd_gpu_sysfs_temp_sensor`,
//! `read_gpu_hwmon_temp_sensor`, `read_amd_gpu_sysfs_usage`, `read_amd_gpu_vram`) are left
//! un-mocked and called directly with a `drm_root`/`hwmon_root` parameter, same
//! fixture-testability precedent as `gpu_sysfs.rs` itself: tests point them at an empty directory
//! for "unavailable" or a minimal fixture tree for "available".

use std::collections::HashMap;
use std::path::Path;

use crate::gpu_nvml::NvidiaNvmlReader;
use crate::gpu_rsmi::AmdRsmiReader;
use crate::gpu_sysfs::{
    self, GpuVramReading, NvidiaDisplayDeviceState, SysfsGpuUsageReading, TempSensorReading,
    has_usable_vram, merge_gpu_vram,
};
use crate::intel_gpu;

/// Port of `SystemMonitorService::GpuTempData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuTempData {
    pub temp_c: Option<f64>,
    pub source: String,
    pub detail: String,
}

/// Port of `SystemMonitorService::GpuUsageData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuUsageData {
    pub percent: Option<f64>,
    pub source: String,
}

/// Port of `SystemMonitorService::GpuVramData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuVramData {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub source: String,
}

/// Testability seam for [`NvidiaNvmlReader`]: lets the decision tree in this module be exercised
/// with a fake instead of a real `dlopen`'d NVML library.
pub trait NvmlGpuSource {
    fn read_gpu_temp_sensor(&mut self) -> Option<TempSensorReading>;
    fn read_gpu_usage_percent(&mut self) -> Option<f64>;
    fn read_gpu_vram(&mut self) -> Option<GpuVramReading>;
}

impl NvmlGpuSource for NvidiaNvmlReader {
    fn read_gpu_temp_sensor(&mut self) -> Option<TempSensorReading> {
        NvidiaNvmlReader::read_gpu_temp_sensor(self)
    }

    fn read_gpu_usage_percent(&mut self) -> Option<f64> {
        NvidiaNvmlReader::read_gpu_usage_percent(self)
    }

    fn read_gpu_vram(&mut self) -> Option<GpuVramReading> {
        NvidiaNvmlReader::read_gpu_vram(self)
    }
}

/// Testability seam for [`AmdRsmiReader`], same rationale as [`NvmlGpuSource`].
pub trait RsmiGpuSource {
    fn ready(&mut self) -> bool;
    fn read_temp_sensor(&mut self) -> Option<TempSensorReading>;
    fn read_usage(&mut self) -> Option<SysfsGpuUsageReading>;
}

impl RsmiGpuSource for AmdRsmiReader {
    fn ready(&mut self) -> bool {
        AmdRsmiReader::ready(self)
    }

    fn read_temp_sensor(&mut self) -> Option<TempSensorReading> {
        AmdRsmiReader::read_temp_sensor(self)
    }

    fn read_usage(&mut self) -> Option<SysfsGpuUsageReading> {
        AmdRsmiReader::read_usage(self)
    }
}

/// Testability seam for [`IntelGpuReader`], same rationale as [`NvmlGpuSource`].
pub trait IntelGpuSource {
    fn ready(&self) -> bool;
    fn usage_source(&self) -> String;
    fn read_usage(&mut self) -> Option<intel_gpu::UsageReading>;
    fn read_vram(&self) -> Option<intel_gpu::VramReading>;
}

/// Port of `SystemMonitorService::IntelGpuReader`.
pub struct IntelGpuReader {
    devices: Vec<intel_gpu::Device>,
    samplers: HashMap<String, intel_gpu::UsageSampler>,
}

impl IntelGpuReader {
    /// The C++ default-constructs against the real `/sys/class/drm`; callers here pass
    /// [`crate::intel_gpu::DEFAULT_DRM_ROOT`] explicitly, same precedent as
    /// `intel_gpu::find_devices`'s own `drm_root` parameter.
    pub fn new(drm_root: &Path) -> Self {
        let mut devices = intel_gpu::find_devices(drm_root);
        // A discrete card is the one that reports VRAM; order it ahead of an integrated GPU so
        // the stats describe the card the user cares about. `sort_by_cached_key` calls the key
        // function exactly once per element, matching `std::stable_partition`'s "exactly N
        // predicate applications" guarantee — important since the predicate performs a real ioctl
        // per device.
        devices.sort_by_cached_key(|device| intel_gpu::read_vram(device).is_none());
        Self {
            devices,
            samplers: HashMap::new(),
        }
    }
}

impl IntelGpuSource for IntelGpuReader {
    fn ready(&self) -> bool {
        !self.devices.is_empty()
    }

    // The first scan only baselines the counters, so name the source before it can report a
    // value.
    fn usage_source(&self) -> String {
        self.devices
            .first()
            .map(intel_gpu::usage_source)
            .unwrap_or_default()
    }

    fn read_usage(&mut self) -> Option<intel_gpu::UsageReading> {
        for device in &self.devices {
            let sampler = self.samplers.entry(device.pci_slot.clone()).or_default();
            if let Some(reading) = sampler.sample(device) {
                return Some(reading);
            }
        }
        None
    }

    fn read_vram(&self) -> Option<intel_gpu::VramReading> {
        self.devices.iter().find_map(intel_gpu::read_vram)
    }
}

/// Port of `readIntelGpuUsageData`.
pub fn read_intel_gpu_usage_data(intel: &mut impl IntelGpuSource) -> GpuUsageData {
    if !intel.ready() {
        return GpuUsageData::default();
    }
    if let Some(usage) = intel.read_usage() {
        return GpuUsageData {
            percent: Some(usage.percent),
            source: usage.source,
        };
    }
    GpuUsageData {
        percent: None,
        source: intel.usage_source(),
    }
}

/// Port of `readIntelGpuVram`.
pub fn read_intel_gpu_vram(intel: &mut impl IntelGpuSource) -> Option<GpuVramData> {
    if !intel.ready() {
        return None;
    }
    let vram = intel.read_vram()?;
    Some(GpuVramData {
        used_bytes: vram.used_bytes,
        total_bytes: vram.total_bytes,
        source: vram.source,
    })
}

/// Port of `readGpuTempData`.
pub fn read_gpu_temp_data(
    nvidia_display_state: NvidiaDisplayDeviceState,
    nvml: &mut impl NvmlGpuSource,
    rsmi: &mut impl RsmiGpuSource,
    drm_root: &Path,
    hwmon_root: &Path,
) -> GpuTempData {
    match nvidia_display_state {
        NvidiaDisplayDeviceState::Active => {
            let reading = nvml.read_gpu_temp_sensor();
            let detail = if reading.is_some() {
                "NVML-only mode active"
            } else {
                "NVML-only mode active; NVML unavailable"
            };
            return GpuTempData {
                temp_c: reading.as_ref().map(|r| r.temp_c),
                source: reading.map(|r| r.source).unwrap_or_default(),
                detail: detail.to_string(),
            };
        }
        NvidiaDisplayDeviceState::InactiveOnly => {
            if let Some(amd_rsmi) = rsmi.read_temp_sensor() {
                return GpuTempData {
                    temp_c: Some(amd_rsmi.temp_c),
                    source: amd_rsmi.source,
                    detail: "NVML skipped; using ROCm SMI edge temperature".to_string(),
                };
            }
            if rsmi.ready() {
                return GpuTempData {
                    temp_c: None,
                    source: String::new(),
                    detail: "NVML skipped; ROCm SMI temperature unavailable".to_string(),
                };
            }
            if let Some(amd_sysfs) = gpu_sysfs::read_amd_gpu_sysfs_temp_sensor(drm_root) {
                return GpuTempData {
                    temp_c: Some(amd_sysfs.temp_c),
                    source: amd_sysfs.source,
                    detail: "NVML skipped; using amdgpu sysfs temp1_input".to_string(),
                };
            }
            let hwmon = gpu_sysfs::read_gpu_hwmon_temp_sensor(hwmon_root);
            return GpuTempData {
                temp_c: hwmon.reading.as_ref().map(|r| r.temp_c),
                source: hwmon.reading.map(|r| r.source).unwrap_or_default(),
                detail: "NVML skipped; NVIDIA display device is runtime-suspended".to_string(),
            };
        }
        NvidiaDisplayDeviceState::None => {}
    }

    if let Some(amd_rsmi) = rsmi.read_temp_sensor() {
        return GpuTempData {
            temp_c: Some(amd_rsmi.temp_c),
            source: amd_rsmi.source,
            detail: "using ROCm SMI edge temperature".to_string(),
        };
    }
    if rsmi.ready() {
        return GpuTempData {
            temp_c: None,
            source: String::new(),
            detail: "ROCm SMI temperature unavailable".to_string(),
        };
    }

    if let Some(amd_sysfs) = gpu_sysfs::read_amd_gpu_sysfs_temp_sensor(drm_root) {
        return GpuTempData {
            temp_c: Some(amd_sysfs.temp_c),
            source: amd_sysfs.source,
            detail: "using amdgpu sysfs temp1_input".to_string(),
        };
    }

    let hwmon = gpu_sysfs::read_gpu_hwmon_temp_sensor(hwmon_root);
    if hwmon.found_nvidia {
        return GpuTempData {
            temp_c: hwmon.reading.as_ref().map(|r| r.temp_c),
            source: hwmon.reading.map(|r| r.source).unwrap_or_default(),
            detail: "NVIDIA hwmon present; NVML fallback not needed".to_string(),
        };
    }

    let mut best = hwmon.reading;
    let nvml_reading = nvml.read_gpu_temp_sensor();
    if let Some(nvml_reading) = &nvml_reading {
        let nvml_is_better = best
            .as_ref()
            .map(|current_best| nvml_reading.temp_c > current_best.temp_c)
            .unwrap_or(true);
        if nvml_is_better {
            best = Some(nvml_reading.clone());
        }
    }
    GpuTempData {
        temp_c: best.as_ref().map(|r| r.temp_c),
        source: best.map(|r| r.source).unwrap_or_default(),
        detail: if nvml_reading.is_some() {
            "NVML fallback available".to_string()
        } else {
            "NVML fallback unavailable".to_string()
        },
    }
}

/// Port of `readGpuUsageData`.
pub fn read_gpu_usage_data(
    nvidia_display_state: NvidiaDisplayDeviceState,
    nvml: &mut impl NvmlGpuSource,
    rsmi: &mut impl RsmiGpuSource,
    intel: &mut impl IntelGpuSource,
    drm_root: &Path,
) -> GpuUsageData {
    match nvidia_display_state {
        NvidiaDisplayDeviceState::Active => {
            return match nvml.read_gpu_usage_percent() {
                Some(percent) => GpuUsageData {
                    percent: Some(percent),
                    source: "nvml".to_string(),
                },
                None => GpuUsageData::default(),
            };
        }
        NvidiaDisplayDeviceState::InactiveOnly => {
            if let Some(rsmi_reading) = rsmi.read_usage() {
                return GpuUsageData {
                    percent: Some(rsmi_reading.percent),
                    source: rsmi_reading.source,
                };
            }
            if rsmi.ready() {
                return GpuUsageData::default();
            }
            if let Some(sysfs) = gpu_sysfs::read_amd_gpu_sysfs_usage(drm_root) {
                return GpuUsageData {
                    percent: Some(sysfs.percent),
                    source: sysfs.source,
                };
            }
            // An Optimus laptop with the discrete GPU asleep renders on the Intel integrated GPU.
            return read_intel_gpu_usage_data(intel);
        }
        NvidiaDisplayDeviceState::None => {}
    }

    if let Some(rsmi_reading) = rsmi.read_usage() {
        return GpuUsageData {
            percent: Some(rsmi_reading.percent),
            source: rsmi_reading.source,
        };
    }
    if rsmi.ready() {
        return GpuUsageData::default();
    }

    if let Some(sysfs) = gpu_sysfs::read_amd_gpu_sysfs_usage(drm_root) {
        return GpuUsageData {
            percent: Some(sysfs.percent),
            source: sysfs.source,
        };
    }

    if let Some(percent) = nvml.read_gpu_usage_percent() {
        return GpuUsageData {
            percent: Some(percent),
            source: "nvml".to_string(),
        };
    }

    read_intel_gpu_usage_data(intel)
}

/// Port of `readGpuVramData`.
pub fn read_gpu_vram_data(
    nvidia_display_state: NvidiaDisplayDeviceState,
    nvml: &mut impl NvmlGpuSource,
    intel: &mut impl IntelGpuSource,
    drm_root: &Path,
) -> Option<GpuVramData> {
    match nvidia_display_state {
        NvidiaDisplayDeviceState::Active => {
            let nvml_reading = nvml.read_gpu_vram()?;
            if !has_usable_vram(&nvml_reading) {
                return None;
            }
            return Some(GpuVramData {
                used_bytes: nvml_reading.used_bytes,
                total_bytes: nvml_reading.total_bytes,
                source: nvml_reading.source,
            });
        }
        NvidiaDisplayDeviceState::InactiveOnly => {
            let mut combined = gpu_sysfs::read_amd_gpu_vram(drm_root);
            if combined.is_none()
                && let Some(intel_vram) = read_intel_gpu_vram(intel)
            {
                combined = Some(GpuVramReading {
                    used_bytes: intel_vram.used_bytes,
                    total_bytes: intel_vram.total_bytes,
                    source: intel_vram.source,
                    is_nvidia: false,
                });
            }
            let combined = combined?;
            if !has_usable_vram(&combined) {
                return None;
            }
            return Some(GpuVramData {
                used_bytes: combined.used_bytes,
                total_bytes: combined.total_bytes,
                source: combined.source,
            });
        }
        NvidiaDisplayDeviceState::None => {}
    }

    let mut combined = gpu_sysfs::read_amd_gpu_vram(drm_root);

    if let Some(nvml_reading) = nvml.read_gpu_vram() {
        match &mut combined {
            Some(existing) => merge_gpu_vram(existing, &nvml_reading),
            None => combined = Some(nvml_reading),
        }
    }

    if let Some(intel_vram) = read_intel_gpu_vram(intel) {
        let reading = GpuVramReading {
            used_bytes: intel_vram.used_bytes,
            total_bytes: intel_vram.total_bytes,
            source: intel_vram.source,
            is_nvidia: false,
        };
        match &mut combined {
            Some(existing) => merge_gpu_vram(existing, &reading),
            None => combined = Some(reading),
        }
    }

    let combined = combined?;
    if !has_usable_vram(&combined) {
        return None;
    }
    Some(GpuVramData {
        used_bytes: combined.used_bytes,
        total_bytes: combined.total_bytes,
        source: combined.source,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "noctalia-gpu-orchestration-test-{}-{name}",
            std::process::id()
        ))
    }

    fn temp_reading(temp_c: f64, source: &str) -> TempSensorReading {
        TempSensorReading {
            temp_c,
            score: 0,
            source: source.to_string(),
            is_nvidia: false,
        }
    }

    // --- fakes for the three lower-level readers ---

    #[derive(Default)]
    struct FakeNvml {
        temp: Option<TempSensorReading>,
        usage: Option<f64>,
        vram: Option<GpuVramReading>,
    }

    impl NvmlGpuSource for FakeNvml {
        fn read_gpu_temp_sensor(&mut self) -> Option<TempSensorReading> {
            self.temp.clone()
        }

        fn read_gpu_usage_percent(&mut self) -> Option<f64> {
            self.usage
        }

        fn read_gpu_vram(&mut self) -> Option<GpuVramReading> {
            self.vram.clone()
        }
    }

    #[derive(Default)]
    struct FakeRsmi {
        ready: bool,
        temp: Option<TempSensorReading>,
        usage: Option<SysfsGpuUsageReading>,
    }

    impl RsmiGpuSource for FakeRsmi {
        fn ready(&mut self) -> bool {
            self.ready
        }

        fn read_temp_sensor(&mut self) -> Option<TempSensorReading> {
            self.temp.clone()
        }

        fn read_usage(&mut self) -> Option<SysfsGpuUsageReading> {
            self.usage.clone()
        }
    }

    #[derive(Default)]
    struct FakeIntel {
        ready: bool,
        usage_source: String,
        usage: Option<intel_gpu::UsageReading>,
        vram: Option<intel_gpu::VramReading>,
    }

    impl IntelGpuSource for FakeIntel {
        fn ready(&self) -> bool {
            self.ready
        }

        fn usage_source(&self) -> String {
            self.usage_source.clone()
        }

        fn read_usage(&mut self) -> Option<intel_gpu::UsageReading> {
            self.usage.clone()
        }

        fn read_vram(&self) -> Option<intel_gpu::VramReading> {
            self.vram.clone()
        }
    }

    // --- sysfs/hwmon fixture helpers, same shape as gpu_sysfs.rs's own test helpers ---

    #[allow(clippy::too_many_arguments)]
    fn write_amd_card(
        drm_root: &Path,
        card: &str,
        busy_percent: Option<u64>,
        hwmon_temp_milli: Option<i64>,
        vram_used: Option<u64>,
        vram_total: Option<u64>,
    ) {
        let card_dir = drm_root.join(card);
        fs::create_dir_all(&card_dir).expect("create card dir");

        let pci_target = drm_root.join("_devices").join(card);
        fs::create_dir_all(&pci_target).expect("create pci target dir");
        fs::write(pci_target.join("vendor"), "0x1002\n").expect("write vendor");

        let driver_target = drm_root.join("_drivers").join("amdgpu");
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

    fn write_hwmon(hwmon_root: &Path, dir_name: &str, name: &str, milli: i64) {
        let hwmon_dir = hwmon_root.join(dir_name);
        fs::create_dir_all(&hwmon_dir).expect("create hwmon dir");
        fs::write(hwmon_dir.join("name"), name).expect("write name");
        fs::write(hwmon_dir.join("temp1_input"), milli.to_string()).expect("write temp input");
    }

    // --- read_gpu_temp_data ---

    #[test]
    fn temp_active_uses_nvml_when_available() {
        let empty = fixture_dir("temp-active-nvml-some");
        let mut nvml = FakeNvml {
            temp: Some(temp_reading(55.0, "nvml:0")),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::Active,
            &mut nvml,
            &mut rsmi,
            &empty,
            &empty,
        );
        assert_eq!(result.temp_c, Some(55.0));
        assert_eq!(result.source, "nvml:0");
        assert_eq!(result.detail, "NVML-only mode active");
    }

    #[test]
    fn temp_active_reports_unavailable_when_nvml_absent() {
        let empty = fixture_dir("temp-active-nvml-none");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::Active,
            &mut nvml,
            &mut rsmi,
            &empty,
            &empty,
        );
        assert_eq!(result.temp_c, None);
        assert_eq!(result.source, "");
        assert_eq!(result.detail, "NVML-only mode active; NVML unavailable");
    }

    #[test]
    fn temp_inactive_only_prefers_rsmi_over_sysfs_and_hwmon() {
        let empty = fixture_dir("temp-inactive-rsmi");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi {
            temp: Some(temp_reading(60.0, "rsmi:0")),
            ..Default::default()
        };

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &empty,
            &empty,
        );
        assert_eq!(result.temp_c, Some(60.0));
        assert_eq!(
            result.detail,
            "NVML skipped; using ROCm SMI edge temperature"
        );
    }

    #[test]
    fn temp_inactive_only_reports_unavailable_when_rsmi_ready_but_no_reading() {
        let empty = fixture_dir("temp-inactive-rsmi-ready-empty");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi {
            ready: true,
            ..Default::default()
        };

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &empty,
            &empty,
        );
        assert_eq!(result.temp_c, None);
        assert_eq!(
            result.detail,
            "NVML skipped; ROCm SMI temperature unavailable"
        );
    }

    #[test]
    fn temp_inactive_only_falls_back_to_amd_sysfs() {
        let dir = fixture_dir("temp-inactive-sysfs");
        write_amd_card(&dir, "card0", None, Some(65000), None, None);
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &dir,
            &dir,
        );
        assert_eq!(result.temp_c, Some(65.0));
        assert_eq!(
            result.detail,
            "NVML skipped; using amdgpu sysfs temp1_input"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_inactive_only_falls_back_to_hwmon() {
        let dir = fixture_dir("temp-inactive-hwmon");
        write_hwmon(&dir, "hwmon0", "amdgpu", 70000);
        let missing_drm = dir.join("no-drm-here");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &missing_drm,
            &dir,
        );
        assert_eq!(result.temp_c, Some(70.0));
        assert_eq!(
            result.detail,
            "NVML skipped; NVIDIA display device is runtime-suspended"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_none_prefers_rsmi_over_sysfs_and_hwmon() {
        let empty = fixture_dir("temp-none-rsmi");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi {
            temp: Some(temp_reading(58.0, "rsmi:0")),
            ..Default::default()
        };

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &empty,
            &empty,
        );
        assert_eq!(result.temp_c, Some(58.0));
        assert_eq!(result.detail, "using ROCm SMI edge temperature");
    }

    #[test]
    fn temp_none_reports_unavailable_when_rsmi_ready_but_no_reading() {
        let empty = fixture_dir("temp-none-rsmi-ready-empty");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi {
            ready: true,
            ..Default::default()
        };

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &empty,
            &empty,
        );
        assert_eq!(result.temp_c, None);
        assert_eq!(result.detail, "ROCm SMI temperature unavailable");
    }

    #[test]
    fn temp_none_falls_back_to_amd_sysfs() {
        let dir = fixture_dir("temp-none-sysfs");
        write_amd_card(&dir, "card0", None, Some(72000), None, None);
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &dir,
            &dir,
        );
        assert_eq!(result.temp_c, Some(72.0));
        assert_eq!(result.detail, "using amdgpu sysfs temp1_input");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_none_prefers_nvidia_hwmon_over_nvml() {
        let dir = fixture_dir("temp-none-hwmon-nvidia");
        write_hwmon(&dir, "hwmon0", "nvidia", 75000);
        let missing_drm = dir.join("no-drm-here");
        let mut nvml = FakeNvml {
            temp: Some(temp_reading(99.0, "nvml:0")),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &missing_drm,
            &dir,
        );
        assert_eq!(result.temp_c, Some(75.0));
        assert_eq!(
            result.detail,
            "NVIDIA hwmon present; NVML fallback not needed"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_none_prefers_nvml_over_non_nvidia_hwmon_when_hotter() {
        let dir = fixture_dir("temp-none-hwmon-vs-nvml-hotter");
        write_hwmon(&dir, "hwmon0", "amdgpu", 40000);
        let missing_drm = dir.join("no-drm-here");
        let mut nvml = FakeNvml {
            temp: Some(temp_reading(90.0, "nvml:0")),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &missing_drm,
            &dir,
        );
        assert_eq!(result.temp_c, Some(90.0));
        assert_eq!(result.source, "nvml:0");
        assert_eq!(result.detail, "NVML fallback available");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_none_keeps_hwmon_when_nvml_not_hotter_but_detail_still_reflects_nvml_present() {
        let dir = fixture_dir("temp-none-hwmon-cooler-fallback");
        write_hwmon(&dir, "hwmon0", "amdgpu", 40000);
        let missing_drm = dir.join("no-drm-here");
        let mut nvml = FakeNvml {
            temp: Some(temp_reading(10.0, "nvml:0")),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &missing_drm,
            &dir,
        );
        assert_eq!(result.temp_c, Some(40.0));
        assert_ne!(
            result.source, "nvml:0",
            "hwmon reading should win, not nvml's"
        );
        assert_eq!(result.detail, "NVML fallback available");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_none_reports_unavailable_when_nothing_reports() {
        let missing = fixture_dir("temp-none-nothing").join("missing");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &missing,
            &missing,
        );
        assert_eq!(result.temp_c, None);
        assert_eq!(result.detail, "NVML fallback unavailable");
    }

    #[test]
    fn temp_none_uses_nvml_when_no_hwmon_reading_exists_at_all() {
        // Isolates the `!best.has_value() || nvml->tempC > best->tempC` tie-break's `!best
        // .has_value()` half: no hwmon directory at all (not even a non-GPU sensor), so `best`
        // starts as `None` and must unconditionally take the NVML reading.
        let missing = fixture_dir("temp-none-no-hwmon-nvml-only").join("missing");
        let mut nvml = FakeNvml {
            temp: Some(temp_reading(50.0, "nvml:0")),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();

        let result = read_gpu_temp_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &missing,
            &missing,
        );
        assert_eq!(result.temp_c, Some(50.0));
        assert_eq!(result.source, "nvml:0");
        assert_eq!(result.detail, "NVML fallback available");
    }

    // --- read_gpu_usage_data ---

    #[test]
    fn usage_active_uses_nvml_when_available() {
        let empty = fixture_dir("usage-active-some");
        let mut nvml = FakeNvml {
            usage: Some(42.0),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::Active,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result.percent, Some(42.0));
        assert_eq!(result.source, "nvml");
    }

    #[test]
    fn usage_active_defaults_when_nvml_absent() {
        let empty = fixture_dir("usage-active-none");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::Active,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result, GpuUsageData::default());
    }

    #[test]
    fn usage_inactive_only_falls_through_rsmi_sysfs_intel() {
        let empty = fixture_dir("usage-inactive-intel");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();
        let mut intel = FakeIntel {
            ready: true,
            usage: Some(intel_gpu::UsageReading {
                percent: 12.0,
                source: "xe fdinfo:0000:00:02.0".to_string(),
            }),
            ..Default::default()
        };

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result.percent, Some(12.0));
        assert_eq!(result.source, "xe fdinfo:0000:00:02.0");
    }

    #[test]
    fn usage_inactive_only_prefers_rsmi_over_sysfs_and_intel() {
        let empty = fixture_dir("usage-inactive-rsmi");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi {
            usage: Some(SysfsGpuUsageReading {
                percent: 77.0,
                source: "rsmi:0".to_string(),
            }),
            ..Default::default()
        };
        let mut intel = FakeIntel {
            ready: true,
            usage: Some(intel_gpu::UsageReading {
                percent: 12.0,
                source: "should not be used".to_string(),
            }),
            ..Default::default()
        };

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result.percent, Some(77.0));
        assert_eq!(result.source, "rsmi:0");
    }

    #[test]
    fn usage_inactive_only_rsmi_ready_no_reading_defaults_without_intel_fallback() {
        let empty = fixture_dir("usage-inactive-rsmi-ready-empty");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi {
            ready: true,
            ..Default::default()
        };
        let mut intel = FakeIntel {
            ready: true,
            usage: Some(intel_gpu::UsageReading {
                percent: 99.0,
                source: "should not be used".to_string(),
            }),
            ..Default::default()
        };

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result, GpuUsageData::default());
    }

    #[test]
    fn usage_none_prefers_rsmi_over_sysfs_nvml_and_intel() {
        let empty = fixture_dir("usage-none-rsmi");
        let mut nvml = FakeNvml {
            usage: Some(1.0),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi {
            usage: Some(SysfsGpuUsageReading {
                percent: 88.0,
                source: "rsmi:0".to_string(),
            }),
            ..Default::default()
        };
        let mut intel = FakeIntel::default();

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result.percent, Some(88.0));
        assert_eq!(result.source, "rsmi:0");
    }

    #[test]
    fn usage_none_falls_through_rsmi_sysfs_nvml_intel() {
        let empty = fixture_dir("usage-none-nvml");
        let mut nvml = FakeNvml {
            usage: Some(33.0),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result.percent, Some(33.0));
        assert_eq!(result.source, "nvml");
    }

    #[test]
    fn usage_none_falls_back_to_amd_sysfs_before_nvml() {
        let dir = fixture_dir("usage-none-sysfs");
        write_amd_card(&dir, "card0", Some(150), None, None, None);
        let mut nvml = FakeNvml {
            usage: Some(1.0),
            ..Default::default()
        };
        let mut rsmi = FakeRsmi::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &dir,
        );
        assert_eq!(result.percent, Some(100.0), "150% should clamp to 100");
        assert!(result.source.contains("amdgpu sysfs"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn usage_none_falls_all_the_way_to_intel() {
        let empty = fixture_dir("usage-none-intel");
        let mut nvml = FakeNvml::default();
        let mut rsmi = FakeRsmi::default();
        let mut intel = FakeIntel {
            ready: true,
            usage_source: "xe fdinfo:0000:00:02.0".to_string(),
            usage: None,
            ..Default::default()
        };

        let result = read_gpu_usage_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut rsmi,
            &mut intel,
            &empty,
        );
        assert_eq!(result.percent, None);
        assert_eq!(result.source, "xe fdinfo:0000:00:02.0");
    }

    // --- read_gpu_vram_data ---

    #[test]
    fn vram_active_returns_usable_nvml_reading() {
        let empty = fixture_dir("vram-active-some");
        let mut nvml = FakeNvml {
            vram: Some(GpuVramReading {
                used_bytes: 100,
                total_bytes: 1000,
                source: "nvml".to_string(),
                is_nvidia: true,
            }),
            ..Default::default()
        };
        let mut intel = FakeIntel::default();

        let result = read_gpu_vram_data(
            NvidiaDisplayDeviceState::Active,
            &mut nvml,
            &mut intel,
            &empty,
        );
        assert_eq!(
            result,
            Some(GpuVramData {
                used_bytes: 100,
                total_bytes: 1000,
                source: "nvml".to_string(),
            })
        );
    }

    #[test]
    fn vram_active_none_when_nvml_absent() {
        let empty = fixture_dir("vram-active-none");
        let mut nvml = FakeNvml::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_vram_data(
            NvidiaDisplayDeviceState::Active,
            &mut nvml,
            &mut intel,
            &empty,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn vram_inactive_only_prefers_amd_sysfs_over_intel() {
        let dir = fixture_dir("vram-inactive-sysfs");
        write_amd_card(&dir, "card0", None, None, Some(500), Some(2000));
        let mut nvml = FakeNvml::default();
        let mut intel = FakeIntel {
            ready: true,
            vram: Some(intel_gpu::VramReading {
                used_bytes: 1,
                total_bytes: 1,
                source: "should not be used".to_string(),
            }),
            ..Default::default()
        };

        let result = read_gpu_vram_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut intel,
            &dir,
        );
        assert_eq!(
            result,
            Some(GpuVramData {
                used_bytes: 500,
                total_bytes: 2000,
                source: format!(
                    "amdgpu:{}",
                    dir.join("card0")
                        .join("device")
                        .join("mem_info_vram_used")
                        .display()
                ),
            })
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn vram_inactive_only_falls_back_to_intel_when_sysfs_absent() {
        let missing = fixture_dir("vram-inactive-intel").join("missing");
        let mut nvml = FakeNvml::default();
        let mut intel = FakeIntel {
            ready: true,
            vram: Some(intel_gpu::VramReading {
                used_bytes: 300,
                total_bytes: 1500,
                source: "xe vram:0000:00:02.0".to_string(),
            }),
            ..Default::default()
        };

        let result = read_gpu_vram_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut intel,
            &missing,
        );
        assert_eq!(
            result,
            Some(GpuVramData {
                used_bytes: 300,
                total_bytes: 1500,
                source: "xe vram:0000:00:02.0".to_string(),
            })
        );
    }

    #[test]
    fn vram_inactive_only_none_when_neither_reports() {
        let missing = fixture_dir("vram-inactive-none").join("missing");
        let mut nvml = FakeNvml::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_vram_data(
            NvidiaDisplayDeviceState::InactiveOnly,
            &mut nvml,
            &mut intel,
            &missing,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn vram_none_merges_amd_sysfs_and_nvml_and_intel() {
        let dir = fixture_dir("vram-none-merge");
        write_amd_card(&dir, "card0", None, None, Some(100), Some(1000));
        let mut nvml = FakeNvml {
            vram: Some(GpuVramReading {
                used_bytes: 50,
                total_bytes: 500,
                source: "nvml".to_string(),
                is_nvidia: true,
            }),
            ..Default::default()
        };
        let mut intel = FakeIntel {
            ready: true,
            vram: Some(intel_gpu::VramReading {
                used_bytes: 10,
                total_bytes: 100,
                source: "intel".to_string(),
            }),
            ..Default::default()
        };

        let result =
            read_gpu_vram_data(NvidiaDisplayDeviceState::None, &mut nvml, &mut intel, &dir)
                .expect("should merge into a usable reading");
        assert_eq!(result.used_bytes, 160);
        assert_eq!(result.total_bytes, 1600);
        assert!(result.source.contains("nvml"));
        assert!(result.source.contains("intel"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn vram_none_none_when_nothing_reports() {
        let missing = fixture_dir("vram-none-nothing").join("missing");
        let mut nvml = FakeNvml::default();
        let mut intel = FakeIntel::default();

        let result = read_gpu_vram_data(
            NvidiaDisplayDeviceState::None,
            &mut nvml,
            &mut intel,
            &missing,
        );
        assert_eq!(result, None);
    }

    // --- IntelGpuReader wrapper ---

    #[test]
    fn intel_gpu_reader_not_ready_on_missing_drm_root() {
        let missing = fixture_dir("intel-reader-missing").join("missing");
        let reader = IntelGpuReader::new(&missing);
        assert!(!reader.ready());
        assert_eq!(reader.usage_source(), "");
    }
}
