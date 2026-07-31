//! Port of `NvidiaNvmlReader` carved out of `system_monitor_service.cpp` (task 5.6.5.3): a
//! `dlopen`'d `libnvidia-ml.so.1`, hand-rolled against the NVML C ABI (init/shutdown/device-count/
//! handle/temperature/utilization/memory-info function pointers).
//!
//! The `dlopen`/`dlsym` cascade and per-symbol name fallback is [`crate::dl`]'s job (shared with
//! [`crate::gpu_rsmi`]) and is unit-tested there against a fake [`crate::dl::SymbolSource`]. What's
//! specific to NVML — picking the best (highest) temperature across multiple devices, averaging
//! utilization across devices, and the single-vs-multi-device source-string formatting — is split
//! into pure functions below ([`select_best_temp`], [`average_usage`], [`temp_source`],
//! [`vram_source`]) and fixture-tested, same "pure core, fixture-tested" precedent as
//! `intel_gpu.rs`'s `parse_vram_query`/`sample_from_clients`.
//!
//! This dev host has no NVIDIA GPU (`lspci | grep -i vga` shows only an Intel Meteor Lake Arc
//! iGPU, same as task 5.6.3/5.6.5.2's precedent), so the real `dlopen`+device-enumeration path
//! (`NvidiaNvmlReader::ensure_ready`) is not exercised live — task 5.6.5.3's own done bar is to
//! structure that logic to be testable without the real `.so`, not to test it live; live behavior
//! against a real NVML install is a manual check only.

use std::ffi::c_void;

use crate::dl::{DlLibrary, load_fn};
use crate::gpu_sysfs::{GpuVramReading, TempSensorReading, has_usable_vram};

const NVML_SUCCESS: i32 = 0;
const NVML_TEMPERATURE_GPU: u32 = 0;
const NVML_LIBRARY: &str = "libnvidia-ml.so.1";

/// Opaque `struct nvmlDevice_st*`.
type NvmlDevice = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct NvmlUsage {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

type NvmlInitFn = unsafe extern "C" fn() -> i32;
type NvmlShutdownFn = unsafe extern "C" fn() -> i32;
type NvmlDeviceGetCountFn = unsafe extern "C" fn(*mut u32) -> i32;
type NvmlDeviceGetHandleByIndexFn = unsafe extern "C" fn(u32, *mut NvmlDevice) -> i32;
type NvmlDeviceGetTemperatureFn = unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> i32;
type NvmlDeviceGetUsageRatesFn = unsafe extern "C" fn(NvmlDevice, *mut NvmlUsage) -> i32;
type NvmlDeviceGetMemoryInfoFn = unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> i32;

struct NvmlFunctions {
    shutdown: NvmlShutdownFn,
    get_temperature: NvmlDeviceGetTemperatureFn,
    get_usage_rates: NvmlDeviceGetUsageRatesFn,
    get_memory_info: NvmlDeviceGetMemoryInfoFn,
}

struct NvmlDeviceRef {
    index: u32,
    handle: NvmlDevice,
}

/// Owns the loaded library and its resolved function pointers; the device handles in `devices`
/// are only valid while `library` stays loaded, so they're kept together.
struct NvmlSession {
    // Kept alive so `functions`' resolved pointers and `devices`' handles stay valid; never read
    // directly.
    #[allow(dead_code)]
    library: DlLibrary,
    functions: NvmlFunctions,
    devices: Vec<NvmlDeviceRef>,
}

impl Drop for NvmlSession {
    fn drop(&mut self) {
        // SAFETY: `shutdown` was resolved from `library`, which is still loaded (this `Drop` runs
        // before `library`'s own `Drop`, which is the one that `dlclose`s); pairs with the
        // `nvmlInit`/`nvmlInit_v2` call that produced this session.
        unsafe {
            (self.functions.shutdown)();
        }
    }
}

enum NvmlState {
    Uninitialized,
    Unavailable,
    Ready(NvmlSession),
}

/// Port of `SystemMonitorService::NvidiaNvmlReader`.
pub struct NvidiaNvmlReader {
    state: NvmlState,
}

impl Default for NvidiaNvmlReader {
    fn default() -> Self {
        Self {
            state: NvmlState::Uninitialized,
        }
    }
}

impl NvidiaNvmlReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `readGpuTempSensor`: probes every device's temperature (no short-circuit, matching
    /// the C++'s unconditional full scan) and keeps the highest reading.
    pub fn read_gpu_temp_sensor(&mut self) -> Option<TempSensorReading> {
        let session = self.ensure_ready()?;
        if session.devices.is_empty() {
            return None;
        }

        let readings: Vec<(u32, Option<u32>)> = session
            .devices
            .iter()
            .map(|device| {
                let mut temp_c = 0u32;
                // SAFETY: `device.handle` came from a successful `nvmlDeviceGetHandleByIndex[_v2]`
                // call on this still-live session; `temp_c` is a valid, live `u32` out-parameter.
                let rc = unsafe {
                    (session.functions.get_temperature)(
                        device.handle,
                        NVML_TEMPERATURE_GPU,
                        &mut temp_c,
                    )
                };
                (device.index, (rc == NVML_SUCCESS).then_some(temp_c))
            })
            .collect();

        let (best_temp, best_index) = select_best_temp(&readings)?;
        Some(TempSensorReading {
            temp_c: best_temp,
            score: 0,
            source: temp_source(session.devices.len(), best_index),
            is_nvidia: true,
        })
    }

    /// Port of `readGpuUsagePercent`: averages utilization across every device that reports a
    /// sane (`<= 100`) value.
    pub fn read_gpu_usage_percent(&mut self) -> Option<f64> {
        let session = self.ensure_ready()?;
        if session.devices.is_empty() {
            return None;
        }

        let usages: Vec<Option<u32>> = session
            .devices
            .iter()
            .map(|device| {
                let mut usage = NvmlUsage::default();
                // SAFETY: `device.handle` is a live handle from this session; `usage` is a valid,
                // live out-parameter.
                let rc = unsafe { (session.functions.get_usage_rates)(device.handle, &mut usage) };
                (rc == NVML_SUCCESS).then_some(usage.gpu)
            })
            .collect();

        average_usage(&usages)
    }

    /// Port of `readGpuVram`: sums used/total bytes across every device with a usable reading.
    pub fn read_gpu_vram(&mut self) -> Option<GpuVramReading> {
        let session = self.ensure_ready()?;
        if session.devices.is_empty() {
            return None;
        }

        let mut total = GpuVramReading {
            used_bytes: 0,
            total_bytes: 0,
            source: vram_source(session.devices.len()),
            is_nvidia: true,
        };

        for device in &session.devices {
            let mut memory = NvmlMemory::default();
            // SAFETY: `device.handle` is a live handle from this session; `memory` is a valid,
            // live out-parameter.
            let rc = unsafe { (session.functions.get_memory_info)(device.handle, &mut memory) };
            if rc != NVML_SUCCESS || memory.total == 0 || memory.used > memory.total {
                continue;
            }
            total.used_bytes += memory.used;
            total.total_bytes += memory.total;
        }

        has_usable_vram(&total).then_some(total)
    }

    fn ensure_ready(&mut self) -> Option<&mut NvmlSession> {
        if matches!(self.state, NvmlState::Uninitialized) {
            self.state = match Self::init_session() {
                Some(session) => NvmlState::Ready(session),
                None => NvmlState::Unavailable,
            };
        }
        match &mut self.state {
            NvmlState::Ready(session) => Some(session),
            NvmlState::Unavailable | NvmlState::Uninitialized => None,
        }
    }

    /// Port of the `dlopen`/`dlsym`/`nvmlInit`/device-enumeration portion of `ensureReady`.
    fn init_session() -> Option<NvmlSession> {
        let library = DlLibrary::open_first(&[NVML_LIBRARY], libc::RTLD_LAZY | libc::RTLD_LOCAL)?;

        let init: NvmlInitFn = load_fn(&library, "nvmlInit_v2", Some("nvmlInit"))?;
        let shutdown: NvmlShutdownFn = load_fn(&library, "nvmlShutdown", None)?;
        let get_count: NvmlDeviceGetCountFn = load_fn(
            &library,
            "nvmlDeviceGetCount_v2",
            Some("nvmlDeviceGetCount"),
        )?;
        let get_handle: NvmlDeviceGetHandleByIndexFn = load_fn(
            &library,
            "nvmlDeviceGetHandleByIndex_v2",
            Some("nvmlDeviceGetHandleByIndex"),
        )?;
        let get_temperature: NvmlDeviceGetTemperatureFn =
            load_fn(&library, "nvmlDeviceGetTemperature", None)?;
        let get_usage_rates: NvmlDeviceGetUsageRatesFn =
            load_fn(&library, "nvmlDeviceGetUtilizationRates", None)?;
        let get_memory_info: NvmlDeviceGetMemoryInfoFn =
            load_fn(&library, "nvmlDeviceGetMemoryInfo", None)?;

        // SAFETY: `init` was resolved from a symbol NVML documents as taking no arguments and
        // returning an `nvmlReturn_t` status code; no preconditions beyond the library being
        // loaded.
        if unsafe { init() } != NVML_SUCCESS {
            return None;
        }

        let mut count = 0u32;
        // SAFETY: `count` is a valid, live `u32` out-parameter; NVML is now initialized (the call
        // above just succeeded).
        if unsafe { get_count(&mut count) } != NVML_SUCCESS {
            // SAFETY: `init` just succeeded, so shutting down here pairs with it (matches the
            // C++'s `close()` call on this failure path).
            unsafe {
                shutdown();
            }
            return None;
        }

        let mut devices = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut handle: NvmlDevice = std::ptr::null_mut();
            // SAFETY: `handle` is a valid, live out-parameter; `index` is in `0..count` as NVML
            // requires.
            if unsafe { get_handle(index, &mut handle) } == NVML_SUCCESS && !handle.is_null() {
                devices.push(NvmlDeviceRef { index, handle });
            }
        }

        Some(NvmlSession {
            library,
            functions: NvmlFunctions {
                shutdown,
                get_temperature,
                get_usage_rates,
                get_memory_info,
            },
            devices,
        })
    }
}

/// Port of the best-of-multiple-devices comparison in `NvidiaNvmlReader::readGpuTempSensor`:
/// `None` per device means the `nvmlDeviceGetTemperature` call failed; `Some(0)` is also rejected
/// (the C++'s `tempC == 0` reject), matching `!= kNvmlSuccess || tempC == 0`.
fn select_best_temp(readings: &[(u32, Option<u32>)]) -> Option<(f64, u32)> {
    let mut best: Option<(f64, u32)> = None;
    for &(index, temp_c) in readings {
        let Some(temp_c) = temp_c.filter(|&t| t != 0) else {
            continue;
        };
        let temp_c = f64::from(temp_c);
        if best.is_none_or(|(best_temp, _)| temp_c > best_temp) {
            best = Some((temp_c, index));
        }
    }
    best
}

/// Port of the average-across-devices loop in `NvidiaNvmlReader::readGpuUsagePercent`: `None` per
/// device means the call failed; a `Some` value over 100 is also rejected (`usage.gpu > 100U`).
fn average_usage(usages: &[Option<u32>]) -> Option<f64> {
    let mut total = 0.0;
    let mut sampled = 0usize;
    for &usage in usages {
        let Some(usage) = usage.filter(|&u| u <= 100) else {
            continue;
        };
        total += f64::from(usage);
        sampled += 1;
    }
    (sampled > 0).then_some(total / sampled as f64)
}

/// Port of `readGpuTempSensor`'s source-string formatting: `"nvml"` for a single device, else the
/// index of whichever device won.
fn temp_source(device_count: usize, best_index: u32) -> String {
    if device_count == 1 {
        "nvml".to_string()
    } else {
        format!("nvml:device{best_index}")
    }
}

/// Port of `readGpuVram`'s source-string formatting: `"nvml"` for a single device, else the device
/// count (VRAM is summed, not attributed to a single winning device).
fn vram_source(device_count: usize) -> String {
    if device_count == 1 {
        "nvml".to_string()
    } else {
        format!("nvml ({device_count} devices)")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn select_best_temp_picks_the_highest_reading() {
        let readings = [(0, Some(40)), (1, Some(70)), (2, Some(55))];
        assert_eq!(select_best_temp(&readings), Some((70.0, 1)));
    }

    #[test]
    fn select_best_temp_skips_failed_calls_and_zero_readings() {
        let readings = [(0, None), (1, Some(0)), (2, Some(30))];
        assert_eq!(select_best_temp(&readings), Some((30.0, 2)));
    }

    #[test]
    fn select_best_temp_none_when_every_device_fails() {
        let readings = [(0, None), (1, Some(0))];
        assert_eq!(select_best_temp(&readings), None);
    }

    #[test]
    fn average_usage_averages_the_successful_samples() {
        assert_eq!(average_usage(&[Some(20), Some(40)]), Some(30.0));
    }

    #[test]
    fn average_usage_skips_failed_calls_and_over_100_values() {
        assert_eq!(average_usage(&[None, Some(200), Some(50)]), Some(50.0));
    }

    #[test]
    fn average_usage_none_when_no_device_sampled() {
        assert_eq!(average_usage(&[None, Some(150)]), None);
    }

    #[test]
    fn temp_source_names_the_device_only_when_multiple_exist() {
        assert_eq!(temp_source(1, 3), "nvml");
        assert_eq!(temp_source(2, 3), "nvml:device3");
    }

    #[test]
    fn vram_source_names_the_count_only_when_multiple_exist() {
        assert_eq!(vram_source(1), "nvml");
        assert_eq!(vram_source(3), "nvml (3 devices)");
    }

    #[test]
    fn new_reader_reports_no_readings_without_a_real_nvml_library() {
        // This dev host has no NVIDIA GPU/driver, so `ensure_ready` genuinely fails to `dlopen`
        // `libnvidia-ml.so.1` — exercising the `Unavailable` path for real, same precedent as
        // task 5.6.3's "no xe device on this host" note.
        let mut reader = NvidiaNvmlReader::new();
        assert_eq!(reader.read_gpu_temp_sensor(), None);
        assert_eq!(reader.read_gpu_usage_percent(), None);
        assert!(reader.read_gpu_vram().is_none());
    }
}
