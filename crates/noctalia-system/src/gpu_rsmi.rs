//! Port of `AmdRsmiReader` carved out of `system_monitor_service.cpp` (task 5.6.5.3): a
//! `dlopen`'d `librocm_smi64.so{,.5,.6,.7,.1.0}`, hand-rolled against the ROCm SMI C ABI, including
//! its v5/v6 `rsmi_dev_gpu_clk_freq_get` struct-layout branch selected by version-probing
//! `rsmi_dev_activity_metric_get`'s symbol presence.
//!
//! The `dlopen`/`dlsym` cascade itself is [`crate::dl`]'s job (shared with [`crate::gpu_nvml`]).
//! What's ROCm-SMI-specific — the ambiguous-major-version-1 disambiguation and the resulting v5-
//! vs-v6 clock-ABI branch — is a pure function ([`resolve_clock_abi`]) below, fixture-tested
//! without any real library or `dlopen` call, matching this task's own done bar ("the dlsym-
//! loading/version-detection logic must be structured so it's testable without the real `.so`
//! present").
//!
//! `rsmi_dev_name_get`/`rsmi_dev_power_cap_get`/`rsmi_dev_memory_busy_percent_get`/
//! `rsmi_dev_power_ave_get`/`rsmi_dev_memory_total_get`/`rsmi_dev_memory_usage_get`/
//! `rsmi_dev_pci_throughput_get`/`rsmi_dev_gpu_clk_freq_get` are all loaded (their absence fails
//! `ensure_ready`, exactly like the C++) but the C++ itself never reads back most of their
//! results beyond the one-time per-device probe loop at init — matching that, this port loads and
//! (where the C++ calls them) probes them the same way, without keeping unused fields around
//! afterward (the C++'s member-field storage of e.g. `m_powerCapGet` for its own sake has no
//! behavioral effect once init succeeds, since nothing reads the field again after the probe
//! loop).
//!
//! This dev host has no AMD GPU (`lspci | grep -i vga` shows only an Intel Meteor Lake Arc iGPU,
//! same as task 5.6.3/5.6.5.2's precedent), so the real `dlopen`+init path (`ensure_ready`) is not
//! exercised live; live behavior against a real ROCm SMI install is a manual check only.

use crate::dl::{DlLibrary, SymbolSource, load_fn};
use crate::gpu_sysfs::{SysfsGpuUsageReading, TempSensorReading};

const RSMI_SUCCESS: i32 = 0;
const RSMI_TEMP_TYPE_EDGE: u32 = 0;
const RSMI_TEMP_CURRENT: i32 = 0;
const RSMI_TEMP_MAX: i32 = 1;
const RSMI_DEVICE_NAME_BUFFER_SIZE: usize = 128;
const RSMI_MAX_NUM_FREQUENCIES_V5: usize = 32;
const RSMI_MAX_NUM_FREQUENCIES_V6: usize = 33;

const RSMI_LIBRARY_CANDIDATES: [&str; 6] = [
    "/opt/rocm/lib/librocm_smi64.so",
    "librocm_smi64.so",
    "librocm_smi64.so.5",
    "librocm_smi64.so.1.0",
    "librocm_smi64.so.6",
    "librocm_smi64.so.7",
];

#[repr(C)]
struct RsmiVersion {
    major: u32,
    minor: u32,
    patch: u32,
    build: *const std::ffi::c_char,
}

#[repr(C)]
struct RsmiFrequenciesV5 {
    num_supported: u32,
    current: u32,
    frequency: [u64; RSMI_MAX_NUM_FREQUENCIES_V5],
}

#[repr(C)]
struct RsmiFrequenciesV6 {
    has_deep_sleep: bool,
    num_supported: u32,
    current: u32,
    frequency: [u64; RSMI_MAX_NUM_FREQUENCIES_V6],
}

type RsmiInitFn = unsafe extern "C" fn(u64) -> i32;
type RsmiShutdownFn = unsafe extern "C" fn() -> i32;
type RsmiVersionGetFn = unsafe extern "C" fn(*mut RsmiVersion) -> i32;
type RsmiNumMonitorDevicesFn = unsafe extern "C" fn(*mut u32) -> i32;
type RsmiDevNameGetFn = unsafe extern "C" fn(u32, *mut std::ffi::c_char, usize) -> i32;
type RsmiDevPowerCapGetFn = unsafe extern "C" fn(u32, u32, *mut u64) -> i32;
type RsmiDevTempMetricGetFn = unsafe extern "C" fn(u32, u32, i32, *mut i64) -> i32;
type RsmiDevBusyPercentGetFn = unsafe extern "C" fn(u32, *mut u32) -> i32;
type RsmiDevMemoryBusyPercentGetFn = unsafe extern "C" fn(u32, *mut u32) -> i32;
type RsmiDevGpuClockFreqGetV5Fn = unsafe extern "C" fn(u32, i32, *mut RsmiFrequenciesV5) -> i32;
type RsmiDevGpuClockFreqGetV6Fn = unsafe extern "C" fn(u32, i32, *mut RsmiFrequenciesV6) -> i32;
type RsmiDevPowerAverageGetFn = unsafe extern "C" fn(u32, u32, *mut u64) -> i32;
type RsmiDevMemoryTotalGetFn = unsafe extern "C" fn(u32, i32, *mut u64) -> i32;
type RsmiDevMemoryUsageGetFn = unsafe extern "C" fn(u32, i32, *mut u64) -> i32;
type RsmiDevPciThroughputGetFn = unsafe extern "C" fn(u32, *mut u64, *mut u64, *mut u64) -> i32;

/// Which `rsmi_dev_gpu_clk_freq_get` struct layout a ROCm SMI install uses. Selected once at init
/// by [`resolve_clock_abi`] and only used to decide which symbol-and-struct pairing to validate —
/// like the C++, nothing reads clock frequencies back afterward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RsmiClockAbi {
    V5,
    V6,
}

/// Port of the `effectiveMajor` derivation plus the `effectiveMajor == 5 / 6 || 7 / else` branch
/// in `AmdRsmiReader::ensureReady`: ROCm SMI's version-1 releases report an ambiguous major
/// version, disambiguated by probing whether `rsmi_dev_activity_metric_get` (a v6-only symbol)
/// exists.
fn resolve_clock_abi(major: u32, has_v6_activity_symbol: bool) -> Option<RsmiClockAbi> {
    let effective_major = if major == 1 {
        if has_v6_activity_symbol { 6 } else { 5 }
    } else {
        major
    };
    match effective_major {
        5 => Some(RsmiClockAbi::V5),
        6 | 7 => Some(RsmiClockAbi::V6),
        _ => None,
    }
}

/// Port of the `rocm-smi`/`rocm-smi:device{N}` source-string formatting shared by
/// `AmdRsmiReader::readTempSensor` and `::readUsage`.
fn device_source(device_count: u32, index: u32) -> String {
    if device_count == 1 {
        "rocm-smi".to_string()
    } else {
        format!("rocm-smi:device{index}")
    }
}

struct RsmiFunctions {
    shutdown: RsmiShutdownFn,
    temp_metric_get: RsmiDevTempMetricGetFn,
    busy_percent_get: RsmiDevBusyPercentGetFn,
}

struct RsmiSession {
    // Kept alive so `functions`' resolved pointers stay valid; never read directly.
    #[allow(dead_code)]
    library: DlLibrary,
    functions: RsmiFunctions,
    device_count: u32,
}

impl Drop for RsmiSession {
    fn drop(&mut self) {
        // SAFETY: `shutdown` was resolved from `library`, which is still loaded (this `Drop` runs
        // before `library`'s own `Drop`, which is the one that `dlclose`s); pairs with the
        // `rsmi_init` call that produced this session.
        unsafe {
            (self.functions.shutdown)();
        }
    }
}

enum RsmiState {
    Uninitialized,
    Unavailable,
    Ready(RsmiSession),
}

/// Port of `SystemMonitorService::AmdRsmiReader`.
pub struct AmdRsmiReader {
    state: RsmiState,
}

impl Default for AmdRsmiReader {
    fn default() -> Self {
        Self {
            state: RsmiState::Uninitialized,
        }
    }
}

impl AmdRsmiReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `ready()`.
    pub fn ready(&mut self) -> bool {
        self.ensure_ready().is_some()
    }

    /// Port of `readTempSensor`: returns the first device with a positive edge temperature,
    /// matching the C++'s early-return loop (unlike NVML's full-scan-for-best).
    pub fn read_temp_sensor(&mut self) -> Option<TempSensorReading> {
        let session = self.ensure_ready()?;
        for index in 0..session.device_count {
            let mut temp = 0i64;
            // SAFETY: `index` is in `0..device_count` as ROCm SMI requires; `temp` is a valid,
            // live out-parameter; `temp_metric_get` was resolved from this still-live session.
            let rc = unsafe {
                (session.functions.temp_metric_get)(
                    index,
                    RSMI_TEMP_TYPE_EDGE,
                    RSMI_TEMP_CURRENT,
                    &mut temp,
                )
            };
            if rc != RSMI_SUCCESS || temp <= 0 {
                continue;
            }
            return Some(TempSensorReading {
                temp_c: temp as f64 / 1000.0,
                score: 0,
                source: device_source(session.device_count, index),
                is_nvidia: false,
            });
        }
        None
    }

    /// Port of `readUsage`: returns the first device that reports a busy percent.
    pub fn read_usage(&mut self) -> Option<SysfsGpuUsageReading> {
        let session = self.ensure_ready()?;
        for index in 0..session.device_count {
            let mut usage = 0u32;
            // SAFETY: `index` is in `0..device_count` as ROCm SMI requires; `usage` is a valid,
            // live out-parameter; `busy_percent_get` was resolved from this still-live session.
            let rc = unsafe { (session.functions.busy_percent_get)(index, &mut usage) };
            if rc != RSMI_SUCCESS {
                continue;
            }
            return Some(SysfsGpuUsageReading {
                percent: f64::from(usage),
                source: device_source(session.device_count, index),
            });
        }
        None
    }

    fn ensure_ready(&mut self) -> Option<&mut RsmiSession> {
        if matches!(self.state, RsmiState::Uninitialized) {
            self.state = match Self::init_session() {
                Some(session) => RsmiState::Ready(session),
                None => RsmiState::Unavailable,
            };
        }
        match &mut self.state {
            RsmiState::Ready(session) => Some(session),
            RsmiState::Unavailable | RsmiState::Uninitialized => None,
        }
    }

    /// Port of the `dlopen`/`dlsym`/`rsmi_init`/version-detection/device-count/per-device-probe
    /// portion of `ensureReady`.
    fn init_session() -> Option<RsmiSession> {
        let library = DlLibrary::open_first(&RSMI_LIBRARY_CANDIDATES, libc::RTLD_LAZY)?;

        let init: RsmiInitFn = load_fn(&library, "rsmi_init", None)?;
        let shutdown: RsmiShutdownFn = load_fn(&library, "rsmi_shut_down", None)?;
        let version_get: RsmiVersionGetFn = load_fn(&library, "rsmi_version_get", None)?;
        let num_monitor_devices: RsmiNumMonitorDevicesFn =
            load_fn(&library, "rsmi_num_monitor_devices", None)?;
        let name_get: RsmiDevNameGetFn = load_fn(&library, "rsmi_dev_name_get", None)?;
        let power_cap_get: RsmiDevPowerCapGetFn =
            load_fn(&library, "rsmi_dev_power_cap_get", None)?;
        let temp_metric_get: RsmiDevTempMetricGetFn =
            load_fn(&library, "rsmi_dev_temp_metric_get", None)?;
        let busy_percent_get: RsmiDevBusyPercentGetFn =
            load_fn(&library, "rsmi_dev_busy_percent_get", None)?;
        let _memory_busy_percent_get: RsmiDevMemoryBusyPercentGetFn =
            load_fn(&library, "rsmi_dev_memory_busy_percent_get", None)?;
        let _power_average_get: RsmiDevPowerAverageGetFn =
            load_fn(&library, "rsmi_dev_power_ave_get", None)?;
        let _memory_total_get: RsmiDevMemoryTotalGetFn =
            load_fn(&library, "rsmi_dev_memory_total_get", None)?;
        let _memory_usage_get: RsmiDevMemoryUsageGetFn =
            load_fn(&library, "rsmi_dev_memory_usage_get", None)?;
        let _pci_throughput_get: RsmiDevPciThroughputGetFn =
            load_fn(&library, "rsmi_dev_pci_throughput_get", None)?;

        // SAFETY: `init` was resolved from a symbol ROCm SMI documents as `rsmi_status_t
        // rsmi_init(uint64_t init_flags)`; `0` is the "no special flags" value the C++ passes.
        if unsafe { init(0) } != RSMI_SUCCESS {
            return None;
        }

        let mut version = RsmiVersion {
            major: 0,
            minor: 0,
            patch: 0,
            build: std::ptr::null(),
        };
        // SAFETY: `version` is a valid, live out-parameter; ROCm SMI is now initialized.
        if unsafe { version_get(&mut version) } != RSMI_SUCCESS {
            // SAFETY: `init` just succeeded, so shutting down here pairs with it (matches the
            // C++'s `close()` call on this failure path).
            unsafe {
                shutdown();
            }
            return None;
        }

        let has_v6_activity_symbol = library.has_symbol("rsmi_dev_activity_metric_get");
        let Some(clock_abi) = resolve_clock_abi(version.major, has_v6_activity_symbol) else {
            // SAFETY: see above — `init` succeeded, so this pairs with it.
            unsafe {
                shutdown();
            }
            return None;
        };
        let clock_abi_loaded = match clock_abi {
            RsmiClockAbi::V5 => {
                load_fn::<RsmiDevGpuClockFreqGetV5Fn>(&library, "rsmi_dev_gpu_clk_freq_get", None)
                    .is_some()
            }
            RsmiClockAbi::V6 => {
                load_fn::<RsmiDevGpuClockFreqGetV6Fn>(&library, "rsmi_dev_gpu_clk_freq_get", None)
                    .is_some()
            }
        };
        if !clock_abi_loaded {
            // SAFETY: see above — `init` succeeded, so this pairs with it.
            unsafe {
                shutdown();
            }
            return None;
        }

        let mut device_count = 0u32;
        // SAFETY: `device_count` is a valid, live out-parameter.
        if unsafe { num_monitor_devices(&mut device_count) } != RSMI_SUCCESS || device_count == 0 {
            // SAFETY: see above — `init` succeeded, so this pairs with it.
            unsafe {
                shutdown();
            }
            return None;
        }

        for index in 0..device_count {
            let mut name = [0 as std::ffi::c_char; RSMI_DEVICE_NAME_BUFFER_SIZE];
            // SAFETY: `name` is a valid, live buffer of `RSMI_DEVICE_NAME_BUFFER_SIZE` bytes,
            // matching the size passed; the C++ discards this call's result too (probe-only).
            unsafe {
                name_get(index, name.as_mut_ptr(), RSMI_DEVICE_NAME_BUFFER_SIZE);
            }

            let mut max_power = 0u64;
            // SAFETY: `max_power` is a valid, live out-parameter; result discarded, matching the
            // C++'s probe-only call.
            unsafe {
                power_cap_get(index, 0, &mut max_power);
            }

            let mut temp_max = 0i64;
            // SAFETY: `temp_max` is a valid, live out-parameter; result discarded, matching the
            // C++'s probe-only call.
            unsafe {
                temp_metric_get(index, RSMI_TEMP_TYPE_EDGE, RSMI_TEMP_MAX, &mut temp_max);
            }
        }

        Some(RsmiSession {
            library,
            functions: RsmiFunctions {
                shutdown,
                temp_metric_get,
                busy_percent_get,
            },
            device_count,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn resolve_clock_abi_major_1_uses_v6_symbol_presence() {
        assert_eq!(resolve_clock_abi(1, true), Some(RsmiClockAbi::V6));
        assert_eq!(resolve_clock_abi(1, false), Some(RsmiClockAbi::V5));
    }

    #[test]
    fn resolve_clock_abi_direct_major_versions() {
        assert_eq!(resolve_clock_abi(5, false), Some(RsmiClockAbi::V5));
        assert_eq!(resolve_clock_abi(6, false), Some(RsmiClockAbi::V6));
        assert_eq!(resolve_clock_abi(7, false), Some(RsmiClockAbi::V6));
        // The v6-symbol probe is only consulted when major == 1; a direct major of e.g. 6 doesn't
        // get reinterpreted even if the symbol happens to be present or absent.
        assert_eq!(resolve_clock_abi(6, true), Some(RsmiClockAbi::V6));
    }

    #[test]
    fn resolve_clock_abi_unsupported_major_is_none() {
        assert_eq!(resolve_clock_abi(0, false), None);
        assert_eq!(resolve_clock_abi(2, true), None);
        assert_eq!(resolve_clock_abi(8, false), None);
    }

    #[test]
    fn device_source_names_the_device_only_when_multiple_exist() {
        assert_eq!(device_source(1, 0), "rocm-smi");
        assert_eq!(device_source(2, 1), "rocm-smi:device1");
    }

    #[test]
    fn library_candidates_match_the_cpp_search_order() {
        assert_eq!(
            RSMI_LIBRARY_CANDIDATES,
            [
                "/opt/rocm/lib/librocm_smi64.so",
                "librocm_smi64.so",
                "librocm_smi64.so.5",
                "librocm_smi64.so.1.0",
                "librocm_smi64.so.6",
                "librocm_smi64.so.7",
            ]
        );
    }

    #[test]
    fn new_reader_reports_no_readings_without_a_real_rsmi_library() {
        // This dev host has no AMD GPU/driver, so `ensure_ready` genuinely fails to `dlopen` any
        // of the ROCm SMI candidates — exercising the `Unavailable` path for real, same precedent
        // as task 5.6.3's "no xe device on this host" note.
        let mut reader = AmdRsmiReader::new();
        assert!(!reader.ready());
        assert!(reader.read_temp_sensor().is_none());
        assert!(reader.read_usage().is_none());
    }
}
