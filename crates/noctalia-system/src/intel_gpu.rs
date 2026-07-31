//! Port of `src/system/intel_gpu.{h,cpp}` (task 5.6.3): Intel i915/xe GPU discovery, discrete-VRAM
//! reading via the xe DRM device-query ioctl, and engine-busyness sampling over `/proc/<pid>/
//! fdinfo`. Self-contained (PCI/DRM sysfs + `/proc` scanning), no forward-phase dependency; feeds
//! task 5.6.5's multi-vendor GPU reader.
//!
//! `read_small_text_file` is reused from `cpu_temp.rs` (promoted `pub(crate)` this session — same
//! "second real consumer" promotion precedent as `brightness::c_trim`/`icon_resolver::unquote`).
//!
//! The xe device-query ioctl's wire structs (`linux/drm/xe_drm.h`) aren't in the `libc` crate, so
//! they're mirrored locally as `#[repr(C)]` structs with `size_of` assertions pinning them to the
//! frozen kernel uapi layout — same "small, version-stable kernel uapi, define locally" precedent
//! as task 5.6.2's `rfkill_helper` constants. The ioctl request number is computed from the same
//! `_IOC`/`_IOWR` bit layout as `asm-generic/ioctl.h` rather than hardcoded, so the derivation is
//! checkable against the kernel header instead of being a single opaque magic number.
//!
//! `read_vram`'s ioctl response parsing is split into a pure `parse_vram_query` core (buffer in,
//! `VramReading` out) so it's fixture-testable without a real xe device — same pure-core-
//! extraction precedent as task 5.3.2's `ddc.rs` and task 5.6.2's `day_night_schedule.rs`.
//! `UsageSampler::sample`'s accumulation/delta/percent math is split the same way
//! (`sample_from_clients`, parameterized on an already-collected client map and clock reading)
//! since real `/proc` scanning can't be fixture-driven the way sysfs directories can.
//!
//! No C++ test exists for this file (confirmed: no `intel_gpu_test.cpp` in `tests/`) — task 5.6's
//! own done bar is fixture-driven tests for device discovery/VRAM/usage-delta sampling, which this
//! module has via `find_devices`/`parse_fdinfo` (both already parameterized on their scan root,
//! same precedent as `brightness::scan_backlight_devices`) and `parse_vram_query`/
//! `sample_from_clients`. `read_vram`'s real ioctl and `UsageSampler::sample`'s real `/proc` scan
//! are smoke-tested against the real host only (this dev host has an Intel Meteor Lake Arc GPU
//! bound to `i915`, so the xe ioctl path itself is not exercised live here — same manual-check-
//! only precedent as task 1.6.5's systemd check, task 5.3.2's `ddcutil` check, and task 5.6.2's
//! `/dev/rfkill` write).

use std::collections::HashMap;
use std::fs;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cpu_temp::read_small_text_file;

const INTEL_PCI_VENDOR: &str = "0x8086";
const DRM_NODE_DIR: &str = "/dev/dri/";

/// Render and compute share the execution units on Intel, so they are the two classes that define
/// overall load. Media engines are tracked by neither (matches `UsageSampler::kEngineClassCount`).
const ENGINE_CLASS_COUNT: usize = 2;

// uapi/drm/xe_drm.h, mirrored here so the build doesn't depend on a libdrm new enough to ship
// xe_drm.h — the layouts are frozen uapi, pinned by the `size_of` assertions below.
const DRM_COMMAND_BASE: u32 = 0x40;
const DRM_XE_DEVICE_QUERY: u32 = 0x00;
const DRM_XE_QUERY_MEM_REGIONS: u32 = 1;
const DRM_XE_MEM_REGION_CLASS_VRAM: u16 = 1;

// asm-generic/ioctl.h's `_IOC`/`_IOWR` bit layout, used to compute the ioctl request number
// programmatically instead of hardcoding the resulting magic number.
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

const fn ioc(dir: u32, kind: u32, nr: u32, size: u32) -> u32 {
    (dir << IOC_DIRSHIFT) | (kind << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT)
}

const fn iowr(kind: u32, nr: u32, size: usize) -> u32 {
    ioc(IOC_READ | IOC_WRITE, kind, nr, size as u32)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct DrmXeDeviceQuery {
    extensions: u64,
    query: u32,
    size: u32,
    data: u64,
    reserved: [u64; 2],
}
const _: () = assert!(size_of::<DrmXeDeviceQuery>() == 40);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct DrmXeMemRegion {
    mem_class: u16,
    instance: u16,
    min_page_size: u32,
    total_size: u64,
    used: u64,
    cpu_visible_size: u64,
    cpu_visible_used: u64,
    reserved: [u64; 6],
}
const _: () = assert!(size_of::<DrmXeMemRegion>() == 88);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct DrmXeQueryMemRegions {
    num_mem_regions: u32,
    pad: u32,
}
const _: () = assert!(size_of::<DrmXeQueryMemRegions>() == 8);

/// Port of `Driver`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    I915,
    Xe,
}

/// Port of `Device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub device_path: PathBuf,
    pub render_node: PathBuf,
    pub pci_slot: String,
    pub driver: Driver,
}

/// Port of `VramReading`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VramReading {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub source: String,
}

/// Port of `UsageReading`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageReading {
    pub percent: f64,
    pub source: String,
}

fn driver_name(driver: Driver) -> &'static str {
    match driver {
        Driver::Xe => "xe",
        Driver::I915 => "i915",
    }
}

fn is_drm_card_name(name: &str) -> bool {
    name.starts_with("card") && name.len() > 4 && name[4..].chars().all(|c| c.is_ascii_digit())
}

fn is_pid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())
}

/// Port of `parseLeadingUint`: leading run of ASCII digits (after skipping leading spaces/tabs),
/// `None` if none is found. Real fdinfo engine/id values are always non-negative, so — like the
/// C++'s `std::from_chars<std::uint64_t>` — no sign is recognized.
fn parse_leading_uint(value: &str) -> Option<u64> {
    let trimmed = value.trim_start_matches([' ', '\t']);
    let digits_len = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    trimmed[..digits_len].parse().ok()
}

/// Port of `findRenderNode`: the unprivileged entry point for the device query; the card node
/// would need the seat ACL and DRM master. Empty path when no `renderD*` sibling exists.
fn find_render_node(device_path: &Path) -> PathBuf {
    let Ok(entries) = fs::read_dir(device_path.join("drm")) else {
        return PathBuf::new();
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("renderD") {
            return Path::new(DRM_NODE_DIR).join(name);
        }
    }
    PathBuf::new()
}

/// Port of `findDevices`. Discovery is by PCI vendor `0x8086` plus the bound driver name, so a
/// card with no driver loaded is not reported. The C++'s default argument (`drmRoot =
/// "/sys/class/drm"`) has no Rust equivalent; callers pass [`DEFAULT_DRM_ROOT`] explicitly.
pub fn find_devices(drm_root: &Path) -> Vec<Device> {
    let Ok(entries) = fs::read_dir(drm_root) else {
        return Vec::new();
    };

    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_drm_card_name(&name) {
            continue;
        }

        let device_path = entry.path().join("device");
        if read_small_text_file(&device_path.join("vendor")).as_deref() != Some(INTEL_PCI_VENDOR) {
            continue;
        }

        let Ok(driver_link) = fs::read_link(device_path.join("driver")) else {
            continue;
        };
        let driver = match driver_link.file_name().and_then(|n| n.to_str()) {
            Some("xe") => Driver::Xe,
            Some("i915") => Driver::I915,
            _ => continue,
        };

        // The PCI slot is the fdinfo `drm-pdev` string.
        let Ok(device_link) = fs::read_link(entry.path().join("device")) else {
            continue;
        };
        let pci_slot = device_link
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        devices.push(Device {
            render_node: find_render_node(&device_path),
            device_path,
            pci_slot,
            driver,
        });
    }

    devices
}

/// The real `/sys/class/drm` — the default `find_devices`'s C++ counterpart used when no path was
/// given explicitly.
pub const DEFAULT_DRM_ROOT: &str = "/sys/class/drm";

/// Port of `usageSource`.
pub fn usage_source(device: &Device) -> String {
    format!("{} fdinfo:{}", driver_name(device.driver), device.pci_slot)
}

/// Engine class keys, indexed to match [`ENGINE_CLASS_COUNT`]'s render/compute slots.
struct EngineKeys {
    busy: &'static str,
    gpu_ticks: &'static str,
    capacity: &'static str,
}

/// xe reports engine time in GPU cycles alongside a free-running GPU timestamp, so its
/// utilization needs no wall clock.
const XE_ENGINE_KEYS: [EngineKeys; ENGINE_CLASS_COUNT] = [
    EngineKeys {
        busy: "drm-cycles-rcs",
        gpu_ticks: "drm-total-cycles-rcs",
        capacity: "drm-engine-capacity-rcs",
    },
    EngineKeys {
        busy: "drm-cycles-ccs",
        gpu_ticks: "drm-total-cycles-ccs",
        capacity: "drm-engine-capacity-ccs",
    },
];

/// i915 reports nanoseconds and has no timestamp key.
const I915_ENGINE_KEYS: [EngineKeys; ENGINE_CLASS_COUNT] = [
    EngineKeys {
        busy: "drm-engine-render",
        gpu_ticks: "",
        capacity: "drm-engine-capacity-render",
    },
    EngineKeys {
        busy: "drm-engine-compute",
        gpu_ticks: "",
        capacity: "drm-engine-capacity-compute",
    },
];

fn engine_keys_for(driver: Driver) -> &'static [EngineKeys; ENGINE_CLASS_COUNT] {
    match driver {
        Driver::Xe => &XE_ENGINE_KEYS,
        Driver::I915 => &I915_ENGINE_KEYS,
    }
}

#[derive(Debug, Clone, Copy)]
struct FdinfoClient {
    client_id: u64,
    busy: [u64; ENGINE_CLASS_COUNT],
    gpu_ticks: [u64; ENGINE_CLASS_COUNT],
    capacity: [u64; ENGINE_CLASS_COUNT],
}

impl Default for FdinfoClient {
    fn default() -> Self {
        Self {
            client_id: 0,
            busy: [0; ENGINE_CLASS_COUNT],
            gpu_ticks: [0; ENGINE_CLASS_COUNT],
            capacity: [1; ENGINE_CLASS_COUNT],
        }
    }
}

/// Port of `parseFdinfo`: `fdinfo` values are `"key:\tvalue"`; i915 suffixes engine times with
/// `" ns"`, so only the leading integer is parsed. `None` unless the driver name, PCI device, and
/// a client id were all found.
fn parse_fdinfo(
    path: &Path,
    device: &Device,
    keys: &[EngineKeys; ENGINE_CLASS_COUNT],
) -> Option<FdinfoClient> {
    let content = fs::read_to_string(path).ok()?;

    let mut client = FdinfoClient::default();
    let mut matched_driver = false;
    let mut matched_device = false;
    let mut have_client_id = false;

    for line in content.split('\n') {
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = &line[..colon];
        let value = &line[colon + 1..];

        match key {
            "drm-driver" => {
                matched_driver = value.contains(driver_name(device.driver));
                continue;
            }
            "drm-pdev" => {
                matched_device = value.contains(device.pci_slot.as_str());
                continue;
            }
            "drm-client-id" => {
                if let Some(parsed) = parse_leading_uint(value) {
                    client.client_id = parsed;
                    have_client_id = true;
                }
                continue;
            }
            _ => {}
        }

        let Some(parsed) = parse_leading_uint(value) else {
            continue;
        };

        for (engine, engine_keys) in keys.iter().enumerate() {
            if key == engine_keys.busy {
                client.busy[engine] = parsed;
            } else if !engine_keys.gpu_ticks.is_empty() && key == engine_keys.gpu_ticks {
                client.gpu_ticks[engine] = parsed;
            } else if key == engine_keys.capacity && parsed > 0 {
                client.capacity[engine] = parsed;
            }
        }
    }

    if !matched_driver || !matched_device || !have_client_id {
        return None;
    }
    Some(client)
}

/// Port of `collectFdinfoClients`: every open of the DRM device is one `drm_file` with one client
/// id, which shows up under every fd that dup()'d or inherited it, in any process — the id is the
/// dedup key. `fdinfo` is `PTRACE_MODE_READ` gated: other users' processes are unreadable and
/// silently skipped.
fn collect_fdinfo_clients(device: &Device, out: &mut HashMap<u64, FdinfoClient>) {
    let keys = engine_keys_for(device.driver);
    let Ok(proc_entries) = fs::read_dir("/proc") else {
        return;
    };

    for proc_entry in proc_entries.flatten() {
        let pid = proc_entry.file_name().to_string_lossy().into_owned();
        if !is_pid_name(&pid) {
            continue;
        }

        let Ok(fd_entries) = fs::read_dir(proc_entry.path().join("fd")) else {
            continue;
        };

        for fd_entry in fd_entries.flatten() {
            let Ok(target) = fs::read_link(fd_entry.path()) else {
                continue;
            };
            if !target.to_string_lossy().starts_with(DRM_NODE_DIR) {
                continue;
            }

            let fdinfo_path = proc_entry.path().join("fdinfo").join(fd_entry.file_name());
            if let Some(client) = parse_fdinfo(&fdinfo_path, device, keys) {
                out.insert(client.client_id, client);
            }
        }
    }
}

/// The DRM usage-stats contract allows a counter to regress; the larger previous value stands
/// until a monotonic update arrives (port of `monotonicDelta`).
fn monotonic_delta(current: u64, previous: u64) -> u64 {
    current.saturating_sub(previous)
}

/// Parses a `DRM_IOCTL_XE_DEVICE_QUERY`/`DRM_XE_DEVICE_QUERY_MEM_REGIONS` response buffer into a
/// [`VramReading`] — the pure core of `readVram`, split out so it's fixture-testable without a
/// real xe device. `buffer` must hold at least `query_size` bytes (callers, including tests,
/// uphold this).
fn parse_vram_query(query_size: u32, buffer: &[u64], pci_slot: &str) -> Option<VramReading> {
    if (query_size as usize) < size_of::<DrmXeQueryMemRegions>() {
        return None;
    }

    let base_ptr = buffer.as_ptr().cast::<u8>();
    // SAFETY: `buffer` holds at least `query_size >= size_of::<DrmXeQueryMemRegions>()` bytes
    // (checked above), so reading an 8-byte header from offset 0 is in-bounds.
    // `read_unaligned` doesn't require 8-byte alignment (though `Vec<u64>` storage already
    // guarantees it), matching the C++'s `memcpy`-based type pun.
    let header = unsafe { base_ptr.cast::<DrmXeQueryMemRegions>().read_unaligned() };

    for i in 0..header.num_mem_regions {
        let offset = size_of::<DrmXeQueryMemRegions>() + (i as usize) * size_of::<DrmXeMemRegion>();
        if offset + size_of::<DrmXeMemRegion>() > query_size as usize {
            break;
        }

        // SAFETY: `offset + size_of::<DrmXeMemRegion>() <= query_size`, just checked, and
        // `buffer` holds at least `query_size` bytes — this read is in-bounds.
        let region = unsafe {
            base_ptr
                .add(offset)
                .cast::<DrmXeMemRegion>()
                .read_unaligned()
        };
        if region.mem_class != DRM_XE_MEM_REGION_CLASS_VRAM || region.total_size == 0 {
            continue;
        }

        // Kernels that still gate the used size behind CAP_PERFMON report zero; an idle GPU never
        // does, so zero means the kernel is withholding it.
        if region.used == 0 || region.used > region.total_size {
            return None;
        }

        return Some(VramReading {
            used_bytes: region.used,
            total_bytes: region.total_size,
            source: format!("xe drm query:{pci_slot}"),
        });
    }

    None
}

/// Port of `readVram`: discrete VRAM via the xe DRM device-query ioctl on the render node.
/// Integrated GPUs have no VRAM memory region and report nothing. i915 never reports VRAM: its
/// used-size query is gated behind `CAP_PERFMON` on every kernel, and an unprivileged caller only
/// ever sees zero.
pub fn read_vram(device: &Device) -> Option<VramReading> {
    if device.driver != Driver::Xe || device.render_node.as_os_str().is_empty() {
        return None;
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&device.render_node)
        .ok()?;
    let fd = file.as_raw_fd();

    let request: libc::Ioctl = libc::Ioctl::from(iowr(
        u32::from(b'd'),
        DRM_COMMAND_BASE + DRM_XE_DEVICE_QUERY,
        size_of::<DrmXeDeviceQuery>(),
    ));

    let mut query = DrmXeDeviceQuery {
        query: DRM_XE_QUERY_MEM_REGIONS,
        ..Default::default()
    };
    // SAFETY: `fd` is a valid, open, live file descriptor for `device.render_node`; `query` is a
    // valid, exclusively-owned, correctly-sized `DrmXeDeviceQuery` for this ioctl's first
    // (size-probing) call.
    if unsafe { libc::ioctl(fd, request, &mut query) } != 0
        || (query.size as usize) < size_of::<DrmXeQueryMemRegions>()
    {
        return None;
    }

    // `u64` storage guarantees the 8-byte alignment the region structs need.
    let word_count = (query.size as usize).div_ceil(size_of::<u64>());
    let mut buffer: Vec<u64> = vec![0; word_count];
    query.data = buffer.as_mut_ptr() as u64;
    // SAFETY: `fd`/`query` as above; `query.data` now points at `buffer`, live for the duration of
    // this call, whose `word_count * 8 >= query.size` bytes is exactly what the kernel was told to
    // fill via `query.size`.
    if unsafe { libc::ioctl(fd, request, &mut query) } != 0 {
        return None;
    }
    drop(file);

    parse_vram_query(query.size, &buffer, &device.pci_slot)
}

#[derive(Debug, Clone, Copy, Default)]
struct EngineCounters {
    busy: [u64; ENGINE_CLASS_COUNT],
}

/// Port of `UsageSampler`. Engine busyness is only exposed per DRM client, so utilization is the
/// sum over every client's counter delta between two scans of `/proc/<pid>/fdinfo`. The sampler
/// holds the previous scan and returns nothing until it has two.
#[derive(Debug)]
pub struct UsageSampler {
    clients: HashMap<u64, EngineCounters>,
    gpu_ticks: [u64; ENGINE_CLASS_COUNT],
    sampled_at: Instant,
    has_baseline: bool,
}

impl Default for UsageSampler {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
            gpu_ticks: [0; ENGINE_CLASS_COUNT],
            // Never read before being overwritten: `sample_from_clients` only reads
            // `self.sampled_at` as "previous" when `self.has_baseline` was already `true`, which
            // requires a prior `sample_from_clients` call to have set both together.
            sampled_at: Instant::now(),
            has_baseline: false,
        }
    }
}

impl UsageSampler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_baseline(&self) -> bool {
        self.has_baseline
    }

    /// Port of `UsageSampler::sample`'s real-I/O half: scans `/proc` and delegates the
    /// accumulation/delta/percent math to [`Self::sample_from_clients`].
    pub fn sample(&mut self, device: &Device) -> Option<UsageReading> {
        let mut clients = HashMap::new();
        collect_fdinfo_clients(device, &mut clients);
        self.sample_from_clients(device, clients, Instant::now())
    }

    /// The pure core of `UsageSampler::sample`, parameterized on an already-collected client
    /// snapshot and clock reading so it's testable without a real `/proc` scan.
    fn sample_from_clients(
        &mut self,
        device: &Device,
        clients: HashMap<u64, FdinfoClient>,
        now: Instant,
    ) -> Option<UsageReading> {
        let mut busy_delta = [0u64; ENGINE_CLASS_COUNT];
        let mut gpu_ticks = [0u64; ENGINE_CLASS_COUNT];
        let mut capacity = [1u64; ENGINE_CLASS_COUNT];

        let mut current: HashMap<u64, EngineCounters> = HashMap::with_capacity(clients.len());

        for (client_id, client) in &clients {
            let mut counters = EngineCounters::default();
            let previous = self.clients.get(client_id);

            for engine in 0..ENGINE_CLASS_COUNT {
                counters.busy[engine] = client.busy[engine];
                capacity[engine] = capacity[engine].max(client.capacity[engine]);

                // Every client on a device reports the same GPU timestamp, each sampled at its
                // own instant; the newest is the device's.
                gpu_ticks[engine] = gpu_ticks[engine].max(client.gpu_ticks[engine]);

                // A client first seen this scan only establishes a baseline — its counter already
                // holds work done before the sampler existed.
                if let Some(previous) = previous {
                    busy_delta[engine] +=
                        monotonic_delta(client.busy[engine], previous.busy[engine]);
                }
            }

            current.insert(*client_id, counters);
        }

        let had_baseline = self.has_baseline;
        let previous_ticks = self.gpu_ticks;
        let previous_at = self.sampled_at;

        self.clients = current;
        self.gpu_ticks = gpu_ticks;
        self.sampled_at = now;
        self.has_baseline = true;

        if !had_baseline {
            return None;
        }

        let mut percent = 0.0f64;
        for engine in 0..ENGINE_CLASS_COUNT {
            // The capacity key normalizes a counter that sums concurrent work across engine
            // instances.
            let span = capacity[engine] as f64;

            let engine_percent = if device.driver == Driver::Xe {
                let ticks_delta = monotonic_delta(gpu_ticks[engine], previous_ticks[engine]);
                if ticks_delta == 0 {
                    continue;
                }
                100.0 * busy_delta[engine] as f64 / (ticks_delta as f64 * span)
            } else {
                // `Instant::duration_since` saturates to zero rather than panicking if `now` is
                // somehow before `previous_at`; `steady_clock`/`Instant` are both monotonic, so
                // that never actually happens in a single sampler's sequential calls.
                let elapsed_ns = now.duration_since(previous_at).as_nanos();
                if elapsed_ns == 0 {
                    continue;
                }
                100.0 * busy_delta[engine] as f64 / (elapsed_ns as f64 * span)
            };

            // Render and compute contend for the same execution units, so overall load is the
            // busier of the two, not their sum.
            percent = f64::max(percent, engine_percent);
        }

        Some(UsageReading {
            percent: percent.clamp(0.0, 100.0),
            source: usage_source(device),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "noctalia-intel-gpu-test-{}-{name}",
            std::process::id()
        ))
    }

    fn write_drm_card(
        drm_root: &Path,
        card: &str,
        vendor: &str,
        driver: &str,
        pci_slot: &str,
        render_node: Option<&str>,
    ) {
        // Real sysfs has `card_dir/device` as a symlink to the actual PCI device directory, which
        // in turn has `driver` as a symlink to the bound driver. Mirrored here with real symlinks
        // (not plain subdirectories) so `fs::read_link` resolves them the same way `find_devices`
        // expects.
        let card_dir = drm_root.join(card);
        fs::create_dir_all(&card_dir).expect("create card dir");

        let pci_target = drm_root.join("_devices").join(pci_slot);
        fs::create_dir_all(&pci_target).expect("create pci target dir");
        fs::write(pci_target.join("vendor"), format!("{vendor}\n")).expect("write vendor");

        let driver_target = drm_root.join("_drivers").join(driver);
        fs::create_dir_all(&driver_target).expect("create driver target dir");
        std::os::unix::fs::symlink(&driver_target, pci_target.join("driver"))
            .expect("symlink driver");

        std::os::unix::fs::symlink(&pci_target, card_dir.join("device")).expect("symlink device");

        if let Some(render_node) = render_node {
            fs::create_dir_all(pci_target.join("drm").join(render_node))
                .expect("create render node dir");
        }
    }

    #[test]
    fn find_devices_discovers_intel_cards_by_vendor_and_bound_driver() {
        let dir = fixture_dir("find-devices");
        write_drm_card(
            &dir,
            "card0",
            "0x8086",
            "xe",
            "0000:00:02.0",
            Some("renderD128"),
        );
        write_drm_card(&dir, "card1", "0x1002", "amdgpu", "0000:03:00.0", None);
        write_drm_card(&dir, "card2", "0x8086", "vfio-pci", "0000:00:03.0", None);

        let mut devices = find_devices(&dir);
        devices.sort_by(|a, b| a.pci_slot.cmp(&b.pci_slot));

        assert_eq!(
            devices.len(),
            1,
            "non-Intel vendor and an unrecognized bound driver should both be skipped"
        );
        assert_eq!(devices[0].pci_slot, "0000:00:02.0");
        assert_eq!(devices[0].driver, Driver::Xe);
        assert_eq!(
            devices[0].render_node,
            Path::new("/dev/dri/renderD128"),
            "the renderD* sibling under device/drm should be found"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_devices_reports_i915_and_empty_render_node_when_none_found() {
        let dir = fixture_dir("find-devices-i915");
        write_drm_card(&dir, "card0", "0x8086", "i915", "0000:00:02.0", None);

        let devices = find_devices(&dir);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].driver, Driver::I915);
        assert!(devices[0].render_node.as_os_str().is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_devices_on_missing_directory_is_empty() {
        assert!(find_devices(Path::new("/definitely/not/a/real/drm/dir")).is_empty());
    }

    #[test]
    fn usage_source_formats_driver_and_pci_slot() {
        let device = Device {
            device_path: PathBuf::new(),
            render_node: PathBuf::new(),
            pci_slot: "0000:00:02.0".to_string(),
            driver: Driver::Xe,
        };
        assert_eq!(usage_source(&device), "xe fdinfo:0000:00:02.0");
    }

    fn xe_device(pci_slot: &str) -> Device {
        Device {
            device_path: PathBuf::new(),
            render_node: PathBuf::new(),
            pci_slot: pci_slot.to_string(),
            driver: Driver::Xe,
        }
    }

    fn i915_device(pci_slot: &str) -> Device {
        Device {
            device_path: PathBuf::new(),
            render_node: PathBuf::new(),
            pci_slot: pci_slot.to_string(),
            driver: Driver::I915,
        }
    }

    fn write_fdinfo(dir: &Path, name: &str, content: &str) -> PathBuf {
        fs::create_dir_all(dir).expect("create fdinfo dir");
        let path = dir.join(name);
        fs::write(&path, content).expect("write fdinfo");
        path
    }

    #[test]
    fn parse_fdinfo_extracts_xe_engine_counters_and_rejects_wrong_driver_or_device() {
        let dir = fixture_dir("parse-fdinfo-xe");
        let device = xe_device("0000:00:02.0");
        let keys = engine_keys_for(Driver::Xe);

        let path = write_fdinfo(
            &dir,
            "matching",
            "drm-driver:\txe\n\
             drm-pdev:\t0000:00:02.0\n\
             drm-client-id:\t7\n\
             drm-cycles-rcs:\t1000\n\
             drm-total-cycles-rcs:\t50000\n\
             drm-engine-capacity-rcs:\t2\n",
        );
        let client = parse_fdinfo(&path, &device, keys).expect("should parse");
        assert_eq!(client.client_id, 7);
        assert_eq!(client.busy[0], 1000);
        assert_eq!(client.gpu_ticks[0], 50000);
        assert_eq!(client.capacity[0], 2);
        assert_eq!(
            client.capacity[1], 1,
            "an engine with no capacity line keeps the default of 1"
        );

        let wrong_driver = write_fdinfo(
            &dir,
            "wrong-driver",
            "drm-driver:\ti915\ndrm-pdev:\t0000:00:02.0\ndrm-client-id:\t1\n",
        );
        assert!(parse_fdinfo(&wrong_driver, &device, keys).is_none());

        let wrong_device = write_fdinfo(
            &dir,
            "wrong-device",
            "drm-driver:\txe\ndrm-pdev:\t0000:00:03.0\ndrm-client-id:\t1\n",
        );
        assert!(parse_fdinfo(&wrong_device, &device, keys).is_none());

        let no_client_id = write_fdinfo(
            &dir,
            "no-client-id",
            "drm-driver:\txe\ndrm-pdev:\t0000:00:02.0\n",
        );
        assert!(parse_fdinfo(&no_client_id, &device, keys).is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_fdinfo_strips_i915_nanosecond_suffix_via_leading_integer_parse() {
        let dir = fixture_dir("parse-fdinfo-i915");
        let device = i915_device("0000:00:02.0");
        let keys = engine_keys_for(Driver::I915);

        let path = write_fdinfo(
            &dir,
            "matching",
            "drm-driver:\ti915\n\
             drm-pdev:\t0000:00:02.0\n\
             drm-client-id:\t3\n\
             drm-engine-render:\t123456 ns\n\
             drm-engine-capacity-render:\t1\n",
        );
        let client = parse_fdinfo(&path, &device, keys).expect("should parse");
        assert_eq!(
            client.busy[0], 123456,
            "the trailing ` ns` suffix should be ignored"
        );
        assert_eq!(client.gpu_ticks[0], 0, "i915 has no gpu-ticks key");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_fdinfo_on_missing_file_is_none() {
        let device = xe_device("0000:00:02.0");
        let keys = engine_keys_for(Driver::Xe);
        assert!(parse_fdinfo(Path::new("/does/not/exist"), &device, keys).is_none());
    }

    fn client(id: u64, busy0: u64, gpu_ticks0: u64, capacity0: u64) -> FdinfoClient {
        FdinfoClient {
            client_id: id,
            busy: [busy0, 0],
            gpu_ticks: [gpu_ticks0, 0],
            capacity: [capacity0, 1],
        }
    }

    fn client_both_engines(
        id: u64,
        busy: [u64; ENGINE_CLASS_COUNT],
        gpu_ticks: [u64; ENGINE_CLASS_COUNT],
        capacity: [u64; ENGINE_CLASS_COUNT],
    ) -> FdinfoClient {
        FdinfoClient {
            client_id: id,
            busy,
            gpu_ticks,
            capacity,
        }
    }

    #[test]
    fn sample_from_clients_returns_none_until_a_second_scan_establishes_a_delta() {
        let device = xe_device("0000:00:02.0");
        let mut sampler = UsageSampler::new();
        assert!(!sampler.has_baseline());

        let mut first = HashMap::new();
        first.insert(1, client(1, 1000, 10_000, 1));
        let t0 = Instant::now();
        assert!(sampler.sample_from_clients(&device, first, t0).is_none());
        assert!(sampler.has_baseline());
    }

    #[test]
    fn sample_from_clients_computes_xe_percent_from_ticks_and_busy_deltas() {
        let device = xe_device("0000:00:02.0");
        let mut sampler = UsageSampler::new();

        let mut first = HashMap::new();
        first.insert(1, client(1, 1000, 10_000, 1));
        let t0 = Instant::now();
        assert!(sampler.sample_from_clients(&device, first, t0).is_none());

        let mut second = HashMap::new();
        // Half the GPU ticks elapsed were spent busy on this client.
        second.insert(1, client(1, 6000, 20_000, 1));
        let t1 = t0 + std::time::Duration::from_millis(100);
        let reading = sampler
            .sample_from_clients(&device, second, t1)
            .expect("second scan should produce a reading");
        assert!((reading.percent - 50.0).abs() < 1e-9);
        assert_eq!(reading.source, "xe fdinfo:0000:00:02.0");
    }

    #[test]
    fn sample_from_clients_takes_the_busier_engine_not_the_sum() {
        let device = xe_device("0000:00:02.0");
        let mut sampler = UsageSampler::new();

        let mut first = HashMap::new();
        first.insert(
            1,
            client_both_engines(1, [1000, 1000], [10_000, 10_000], [1, 1]),
        );
        let t0 = Instant::now();
        assert!(sampler.sample_from_clients(&device, first, t0).is_none());

        let mut second = HashMap::new();
        // Render (engine 0): 1000/10000 = 10%. Compute (engine 1): 8000/10000 = 80%. If the two
        // were summed instead of maxed, this would read 90%.
        second.insert(
            1,
            client_both_engines(1, [2000, 9000], [20_000, 20_000], [1, 1]),
        );
        let t1 = t0 + std::time::Duration::from_millis(100);
        let reading = sampler
            .sample_from_clients(&device, second, t1)
            .expect("second scan should produce a reading");
        assert!(
            (reading.percent - 80.0).abs() < 1e-9,
            "overall load should be the busier engine (80%), not the sum (90%): got {}",
            reading.percent
        );
    }

    #[test]
    fn sample_from_clients_treats_a_regressed_counter_as_zero_delta_not_negative() {
        let device = xe_device("0000:00:02.0");
        let mut sampler = UsageSampler::new();

        let mut first = HashMap::new();
        first.insert(1, client(1, 5000, 10_000, 1));
        let t0 = Instant::now();
        assert!(sampler.sample_from_clients(&device, first, t0).is_none());

        // The busy counter for the same client goes backwards (e.g. a driver reset), while the
        // GPU timestamp still advances normally.
        let mut second = HashMap::new();
        second.insert(1, client(1, 3000, 20_000, 1));
        let t1 = t0 + std::time::Duration::from_millis(100);
        let reading = sampler
            .sample_from_clients(&device, second, t1)
            .expect("second scan should produce a reading");
        assert_eq!(
            reading.percent, 0.0,
            "a regressed counter should saturate to a zero delta, not go negative"
        );
    }

    #[test]
    fn sample_from_clients_computes_i915_percent_from_wall_clock_elapsed() {
        let device = i915_device("0000:00:02.0");
        let mut sampler = UsageSampler::new();

        let mut first = HashMap::new();
        first.insert(1, client(1, 0, 0, 1));
        let t0 = Instant::now();
        assert!(sampler.sample_from_clients(&device, first, t0).is_none());

        let mut second = HashMap::new();
        // 50ms busy out of 100ms elapsed = 50%.
        second.insert(1, client(1, 50_000_000, 0, 1));
        let t1 = t0 + std::time::Duration::from_millis(100);
        let reading = sampler
            .sample_from_clients(&device, second, t1)
            .expect("second scan should produce a reading");
        assert!((reading.percent - 50.0).abs() < 1e-6);
    }

    #[test]
    fn sample_from_clients_ignores_a_first_seen_clients_pre_existing_counter_value() {
        let device = xe_device("0000:00:02.0");
        let mut sampler = UsageSampler::new();

        let mut first = HashMap::new();
        first.insert(1, client(1, 1000, 10_000, 1));
        let t0 = Instant::now();
        assert!(sampler.sample_from_clients(&device, first, t0).is_none());

        // Client 1 disappears entirely this scan; a brand-new client 2 shows up with a large
        // existing counter, which should establish a baseline rather than count as a delta.
        let mut second = HashMap::new();
        second.insert(2, client(2, 999_999, 20_000, 1));
        let t1 = t0 + std::time::Duration::from_millis(100);
        let reading = sampler
            .sample_from_clients(&device, second, t1)
            .expect("second scan should produce a reading");
        assert_eq!(
            reading.percent, 0.0,
            "client 1 disappeared (no delta) and client 2 is new (baseline only, no delta)"
        );
    }

    #[test]
    fn parse_vram_query_finds_a_vram_region_after_a_leading_non_vram_region() {
        // Header: 2 regions. Region 0: system memory (memClass=0), skipped regardless of its
        // used/total values. Region 1: VRAM (memClass=1), totalSize=4_000_000, used=2_000_000.
        let mut buffer = vec![0u64; 1 + 11 + 11];
        buffer[0] = 2; // numMemRegions = 2
        buffer[1] = 0; // region 0: memClass = 0 (not VRAM)
        buffer[2] = 2_000_000; // region 0 totalSize (irrelevant, not VRAM)
        buffer[3] = 1_000_000; // region 0 used (irrelevant, not VRAM)
        buffer[12] = 1; // region 1 (word offset 1 + 11): memClass = 1 (VRAM)
        buffer[13] = 4_000_000; // region 1 totalSize
        buffer[14] = 2_000_000; // region 1 used
        let query_size = (8 + 88 * 2) as u32;

        let reading = parse_vram_query(query_size, &buffer, "0000:00:02.0")
            .expect("should find the VRAM region after skipping the non-VRAM one");
        assert_eq!(reading.used_bytes, 2_000_000);
        assert_eq!(reading.total_bytes, 4_000_000);
    }

    #[test]
    fn parse_vram_query_reads_the_first_vram_region_and_rejects_gated_used_size() {
        // Header: 1 region. Region: memClass=VRAM(1), totalSize=1_000_000, used=500_000.
        let mut buffer = vec![0u64; 1 + 11];
        buffer[0] = 1; // numMemRegions = 1, pad = 0
        buffer[1] = 1; // memClass=1 (VRAM), instance=0, minPageSize=0 (all in low 32 bits: 1)
        buffer[2] = 1_000_000; // totalSize
        buffer[3] = 500_000; // used
        let query_size = (8 + 88) as u32;

        let reading = parse_vram_query(query_size, &buffer, "0000:00:02.0")
            .expect("should find the VRAM region");
        assert_eq!(reading.used_bytes, 500_000);
        assert_eq!(reading.total_bytes, 1_000_000);
        assert_eq!(reading.source, "xe drm query:0000:00:02.0");
    }

    #[test]
    fn parse_vram_query_treats_zero_used_as_capperfmon_gated_not_idle() {
        let mut buffer = vec![0u64; 1 + 11];
        buffer[0] = 1;
        buffer[1] = 1; // memClass = VRAM
        buffer[2] = 1_000_000; // totalSize
        buffer[3] = 0; // used = 0
        let query_size = (8 + 88) as u32;

        assert!(
            parse_vram_query(query_size, &buffer, "0000:00:02.0").is_none(),
            "a zero used size should be reported as unavailable, not as an idle GPU"
        );
    }

    #[test]
    fn parse_vram_query_skips_non_vram_regions_and_handles_no_regions() {
        // A single system-memory region (memClass = 0) should be skipped, leaving no VRAM found.
        let mut buffer = vec![0u64; 1 + 11];
        buffer[0] = 1;
        buffer[1] = 0; // memClass = 0 (not VRAM)
        buffer[2] = 1_000_000;
        buffer[3] = 999_000;
        let query_size = (8 + 88) as u32;
        assert!(parse_vram_query(query_size, &buffer, "0000:00:02.0").is_none());

        // Zero regions at all.
        let empty_buffer = vec![0u64; 1];
        assert!(parse_vram_query(8, &empty_buffer, "0000:00:02.0").is_none());
    }

    #[test]
    fn read_vram_on_i915_device_is_none_without_touching_the_filesystem() {
        let device = i915_device("0000:00:02.0");
        assert!(
            read_vram(&device).is_none(),
            "i915 never reports VRAM regardless of render_node"
        );
    }

    #[test]
    fn read_vram_on_xe_device_with_no_render_node_is_none() {
        let mut device = xe_device("0000:00:02.0");
        device.render_node = PathBuf::new();
        assert!(read_vram(&device).is_none());
    }

    #[test]
    fn find_devices_and_usage_sampler_smoke_test_against_the_real_host() {
        // Same live-smoke-test precedent as other sysfs/proc-backed modules in this crate: just
        // confirm the real system is readable without panicking. This dev host's Intel GPU is
        // bound to i915 (Meteor Lake Arc), so this exercises the i915 fdinfo path; the xe ioctl
        // path in `read_vram` is not exercised live here (same manual-check-only precedent as
        // task 5.3.2's `ddcutil` check).
        let devices = find_devices(Path::new(DEFAULT_DRM_ROOT));
        for device in &devices {
            let _ = read_vram(device);
            let mut sampler = UsageSampler::new();
            let _ = sampler.sample(device);
        }
    }
}
