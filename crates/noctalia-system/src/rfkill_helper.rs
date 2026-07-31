//! Port of `src/system/rfkill_helper.{cpp,h}` (task 5.6.2): soft/hard rfkill-switch state for
//! Bluetooth/WLAN radios, read from `/sys/class/rfkill` and toggled by writing a
//! `struct rfkill_event` to `/dev/rfkill`. Self-contained (`core::log` only), no forward-phase
//! dependency.
//!
//! `linux/rfkill.h`'s `rfkill_event`/`RFKILL_TYPE_*`/`RFKILL_OP_*` aren't in the `libc` crate, so
//! the handful of constants this needs are defined locally below rather than pulling in a new
//! dependency for four `u8`s — they're small, version-stable kernel UAPI (unchanged since the
//! rfkill subsystem's introduction). Rather than a `#[repr(C)]` struct plus an `unsafe` transmute
//! to serialize it, the write path builds the 8-byte wire buffer directly
//! (`u32` little/native-endian bytes + four `u8`s, matching `struct rfkill_event`'s no-padding
//! layout byte-for-byte) — this keeps the module free of `unsafe` entirely, reads and writes
//! going through `std::fs` instead of raw `libc` FFI.
//!
//! The two sysfs-scanning entry points (`list_rfkill_entries`/`rfkill_index_for_net_interface`)
//! take the `/sys/class/rfkill` and `/sys/class/net` directories as parameters rather than
//! hardcoding them, so they're fixture-testable — same precedent as `brightness::
//! scan_backlight_devices`'s `backlight_dir` parameter (task 5.3.1). The public API
//! (`set_rfkill_soft_blocked`/`is_rfkill_soft_blocked`/etc.) still always points at the real
//! system paths, matching the C++ exactly.
//!
//! Recorded divergence: the C++'s `/dev/rfkill` write is a single `write()` syscall that fails
//! immediately on any short write (no retry) or `EINTR` (looped once, retried). This port uses
//! `Write::write_all`, which retries through both `EINTR` and partial writes until the buffer is
//! fully written or a hard error occurs. For an 8-byte write to a misc character device whose
//! driver (`rfkill_fop_write` in the kernel) rejects undersized writes outright and otherwise
//! consumes the whole buffer in one op, a genuine short write is not reachable in practice — this
//! is a deliberate, low-risk divergence, not a parity gap.
//!
//! No C++ test exists for this file (confirmed: no `rfkill_helper_test.cpp` in `tests/`) — task
//! 5.6's own done bar is a smoke test. The read-only enumeration/lookup functions are tested
//! against both fixtures and (smoke-only) the real host `/sys/class/rfkill`; the `/dev/rfkill`
//! write path — which would toggle a real radio's soft-block state — is not exercised by the
//! automated suite, same precedent as task 1.6.5's systemd check and task 5.3.2's `ddcutil`
//! check: a manual check, logged in `PROGRESS.log`, not an automated test.

use std::fs;
use std::io::Write;
use std::path::Path;

use noctalia_core::log::Logger;

const LOG: Logger = Logger::new("rfkill");

const RFKILL_CLASS_DIR: &str = "/sys/class/rfkill";
const NET_CLASS_DIR: &str = "/sys/class/net";
const RFKILL_DEVICE_NODE: &str = "/dev/rfkill";

// `enum rfkill_type` (linux/rfkill.h). Only the two values this module ever writes.
const RFKILL_TYPE_WLAN: u8 = 1;
const RFKILL_TYPE_BLUETOOTH: u8 = 2;

// `enum rfkill_operation` (linux/rfkill.h).
const RFKILL_OP_CHANGE: u8 = 2;
const RFKILL_OP_CHANGE_ALL: u8 = 3;

/// Port of `RfkillDeviceType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RfkillDeviceType {
    Bluetooth,
    Wlan,
}

/// Port of `RfkillSwitchResult`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RfkillSwitchResult {
    pub success: bool,
    pub hard_blocked: bool,
    pub detail: String,
}

/// Port of `rfkillTypeStringFor`. The C++ returns `std::optional<string_view>` to stay defensive
/// against an out-of-range `enum class` value reaching the switch's fallthrough; `RfkillDeviceType`
/// is a closed 2-variant Rust enum with no such reachable case, so this returns the string
/// directly — not a behavior change, just dropping unreachable-in-Rust defensiveness.
fn rfkill_type_string_for(device_type: RfkillDeviceType) -> &'static str {
    match device_type {
        RfkillDeviceType::Bluetooth => "bluetooth",
        RfkillDeviceType::Wlan => "wlan",
    }
}

/// Port of `rfkillTypeIdFor` — see [`rfkill_type_string_for`] on the dropped `Option`.
fn rfkill_type_id_for(device_type: RfkillDeviceType) -> u8 {
    match device_type {
        RfkillDeviceType::Bluetooth => RFKILL_TYPE_BLUETOOTH,
        RfkillDeviceType::Wlan => RFKILL_TYPE_WLAN,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RfkillEntry {
    index: u32,
    device_type: String,
    soft: bool,
    hard: bool,
}

/// Port of `readSysfsUnsigned`'s `fscanf(file, "%u", &value)`: skip leading whitespace, then take
/// the longest leading run of decimal digits. `None` when no digit is found (matches `fscanf`
/// returning 0 fields scanned) or the digit run overflows `u32`.
fn read_sysfs_unsigned(path: &Path) -> Option<u32> {
    let content = fs::read_to_string(path).ok()?;
    let digits: String = content
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Port of `readSysfsString`'s `fscanf(file, "%31s", buf)`: skip leading whitespace, then take up
/// to 31 non-whitespace characters. Truncates at 31 Unicode scalar values (`char`s) rather than
/// the C++'s 31 bytes — sysfs `type` values are always plain ASCII (`"wlan"`/`"bluetooth"`), so
/// this char-vs-byte distinction is inert in practice.
fn read_sysfs_string(path: &Path) -> Option<String> {
    let token = fs::read_to_string(path).ok()?;
    let token = token.split_whitespace().next()?;
    Some(token.chars().take(31).collect())
}

/// Port of `listRfkillEntries`.
fn list_rfkill_entries(rfkill_class_dir: &Path) -> Vec<RfkillEntry> {
    let Ok(read_dir) = fs::read_dir(rfkill_class_dir) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for dir_entry in read_dir.flatten() {
        let name = dir_entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("rfkill") {
            continue;
        }
        let base = rfkill_class_dir.join(name.as_ref());
        let Some(index) = read_sysfs_unsigned(&base.join("index")) else {
            continue;
        };
        let Some(device_type) = read_sysfs_string(&base.join("type")) else {
            continue;
        };
        let soft = read_sysfs_unsigned(&base.join("soft")).is_some_and(|v| v != 0);
        let hard = read_sysfs_unsigned(&base.join("hard")).is_some_and(|v| v != 0);
        entries.push(RfkillEntry {
            index,
            device_type,
            soft,
            hard,
        });
    }
    entries
}

/// Port of `findEntries`.
fn find_entries(rfkill_class_dir: &Path, device_type: RfkillDeviceType) -> Vec<RfkillEntry> {
    let wanted = rfkill_type_string_for(device_type);
    list_rfkill_entries(rfkill_class_dir)
        .into_iter()
        .filter(|entry| entry.device_type == wanted)
        .collect()
}

/// Port of `rfkillIndexForNetInterface`.
fn rfkill_index_for_net_interface(net_class_dir: &Path, ifname: &str) -> Option<u32> {
    if ifname.is_empty() {
        return None;
    }
    let phy_dir = net_class_dir.join(ifname).join("phy80211");
    let read_dir = fs::read_dir(&phy_dir).ok()?;

    for dir_entry in read_dir.flatten() {
        let name = dir_entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("rfkill") {
            continue;
        }
        if let Some(index) = read_sysfs_unsigned(&phy_dir.join(name.as_ref()).join("index")) {
            return Some(index);
        }
    }
    None
}

/// Port of `writeRfkillEvent` — see the module doc comment on the hand-built wire buffer and the
/// `write_all`-vs-single-`write()` divergence.
fn write_rfkill_event(idx: u32, kind: u8, op: u8, soft: u8, hard: u8) -> RfkillSwitchResult {
    let mut file = match fs::OpenOptions::new().write(true).open(RFKILL_DEVICE_NODE) {
        Ok(file) => file,
        Err(err) => {
            return RfkillSwitchResult {
                success: false,
                hard_blocked: false,
                detail: format!("cannot open /dev/rfkill: {err}"),
            };
        }
    };

    // `struct rfkill_event { __u32 idx; __u8 type; __u8 op; __u8 soft; __u8 hard; }` — no padding
    // on this target (a 4-byte-aligned `u32` followed by four `u8`s is already 8-byte-sized).
    let mut buf = [0u8; 8];
    buf[0..4].copy_from_slice(&idx.to_ne_bytes());
    buf[4] = kind;
    buf[5] = op;
    buf[6] = soft;
    buf[7] = hard;

    match file.write_all(&buf) {
        Ok(()) => RfkillSwitchResult {
            success: true,
            hard_blocked: false,
            detail: String::new(),
        },
        Err(err) => RfkillSwitchResult {
            success: false,
            hard_blocked: false,
            detail: err.to_string(),
        },
    }
}

fn set_rfkill_soft_blocked_by_index(index: u32, soft_blocked: bool) -> RfkillSwitchResult {
    write_rfkill_event(index, 0, RFKILL_OP_CHANGE, u8::from(soft_blocked), 0)
}

/// See the C++'s own comment: `RFKILL_OP_CHANGE_ALL` applies the soft block to every switch of the
/// type, and to switches of that type that appear later. Radios commonly sit behind several
/// switches (driver plus vendor platform module), and all of them must be unblocked for the radio
/// to come up.
fn set_rfkill_soft_blocked_by_type(type_id: u8, soft_blocked: bool) -> RfkillSwitchResult {
    write_rfkill_event(0, type_id, RFKILL_OP_CHANGE_ALL, u8::from(soft_blocked), 0)
}

/// Port of `setRfkillSoftBlocked`.
pub fn set_rfkill_soft_blocked(
    device_type: RfkillDeviceType,
    soft_blocked: bool,
) -> RfkillSwitchResult {
    let type_id = rfkill_type_id_for(device_type);
    let entries = find_entries(Path::new(RFKILL_CLASS_DIR), device_type);
    if entries.is_empty() {
        LOG.debug(format_args!(
            "setRfkillSoftBlocked: no rfkill entry for type {}",
            rfkill_type_string_for(device_type)
        ));
        return RfkillSwitchResult {
            success: false,
            hard_blocked: false,
            detail: "no rfkill switch found".to_string(),
        };
    }
    if entries.iter().any(|entry| entry.hard) {
        return RfkillSwitchResult {
            success: false,
            hard_blocked: true,
            detail: "rfkill hard block is active".to_string(),
        };
    }
    if entries.iter().all(|entry| entry.soft == soft_blocked) {
        return RfkillSwitchResult {
            success: true,
            hard_blocked: false,
            detail: String::new(),
        };
    }

    let result = set_rfkill_soft_blocked_by_type(type_id, soft_blocked);
    if !result.success {
        LOG.warn(format_args!(
            "setRfkillSoftBlocked: type {} failed: {}",
            rfkill_type_string_for(device_type),
            result.detail
        ));
    }
    result
}

/// Port of `setRfkillSoftBlockedForNetInterface`.
pub fn set_rfkill_soft_blocked_for_net_interface(
    ifname: &str,
    soft_blocked: bool,
) -> RfkillSwitchResult {
    let Some(index) = rfkill_index_for_net_interface(Path::new(NET_CLASS_DIR), ifname) else {
        return RfkillSwitchResult {
            success: false,
            hard_blocked: false,
            detail: "no rfkill switch for interface".to_string(),
        };
    };

    let wanted_type = rfkill_type_string_for(RfkillDeviceType::Wlan);
    for entry in list_rfkill_entries(Path::new(RFKILL_CLASS_DIR)) {
        if entry.index != index {
            continue;
        }
        if entry.device_type != wanted_type {
            return RfkillSwitchResult {
                success: false,
                hard_blocked: false,
                detail: "rfkill switch is not WLAN".to_string(),
            };
        }
        if entry.hard {
            return RfkillSwitchResult {
                success: false,
                hard_blocked: true,
                detail: "rfkill hard block is active".to_string(),
            };
        }
        if entry.soft == soft_blocked {
            return RfkillSwitchResult {
                success: true,
                hard_blocked: false,
                detail: String::new(),
            };
        }
        let result = set_rfkill_soft_blocked_by_index(entry.index, soft_blocked);
        if !result.success {
            LOG.warn(format_args!(
                "setRfkillSoftBlockedForNetInterface: index {} failed: {}",
                entry.index, result.detail
            ));
        }
        return result;
    }

    set_rfkill_soft_blocked_by_index(index, soft_blocked)
}

/// Port of `isRfkillSoftBlocked`: a radio is blocked when any of its switches blocks it.
pub fn is_rfkill_soft_blocked(device_type: RfkillDeviceType) -> bool {
    find_entries(Path::new(RFKILL_CLASS_DIR), device_type)
        .iter()
        .any(|entry| entry.soft)
}

/// Port of `isRfkillHardBlocked`.
pub fn is_rfkill_hard_blocked(device_type: RfkillDeviceType) -> bool {
    find_entries(Path::new(RFKILL_CLASS_DIR), device_type)
        .iter()
        .any(|entry| entry.hard)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "noctalia-rfkill-helper-test-{}-{name}",
            std::process::id()
        ))
    }

    fn write_device(dir: &Path, name: &str, index: u32, device_type: &str, soft: u32, hard: u32) {
        let base = dir.join(name);
        fs::create_dir_all(&base).expect("create device dir");
        fs::write(base.join("index"), format!("{index}\n")).expect("write index");
        fs::write(base.join("type"), format!("{device_type}\n")).expect("write type");
        fs::write(base.join("soft"), format!("{soft}\n")).expect("write soft");
        fs::write(base.join("hard"), format!("{hard}\n")).expect("write hard");
    }

    #[test]
    fn read_sysfs_unsigned_stops_at_the_first_non_digit() {
        let dir = fixture_dir("read-sysfs-unsigned");
        fs::create_dir_all(&dir).expect("create fixture dir");
        let path = dir.join("value");

        fs::write(&path, "  42\n").expect("write");
        assert_eq!(read_sysfs_unsigned(&path), Some(42));

        fs::write(&path, "notanumber\n").expect("write");
        assert_eq!(read_sysfs_unsigned(&path), None);

        fs::write(&path, "").expect("write");
        assert_eq!(read_sysfs_unsigned(&path), None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_sysfs_string_takes_the_first_whitespace_delimited_token() {
        let dir = fixture_dir("read-sysfs-string");
        fs::create_dir_all(&dir).expect("create fixture dir");
        let path = dir.join("value");

        fs::write(&path, "  wlan  \n").expect("write");
        assert_eq!(read_sysfs_string(&path).as_deref(), Some("wlan"));

        fs::write(&path, "").expect("write");
        assert_eq!(read_sysfs_string(&path), None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_rfkill_entries_reads_every_fixture_device() {
        let dir = fixture_dir("list-entries");
        fs::create_dir_all(&dir).expect("create fixture dir");
        write_device(&dir, "rfkill0", 0, "bluetooth", 0, 0);
        write_device(&dir, "rfkill1", 1, "wlan", 1, 0);
        // A non-"rfkill"-prefixed sibling must be ignored.
        fs::write(dir.join("not-an-rfkill-device"), "junk").expect("write");

        let mut entries = list_rfkill_entries(&dir);
        entries.sort_by_key(|entry| entry.index);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].index, 0);
        assert_eq!(entries[0].device_type, "bluetooth");
        assert!(!entries[0].soft);
        assert_eq!(entries[1].index, 1);
        assert_eq!(entries[1].device_type, "wlan");
        assert!(entries[1].soft);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_rfkill_entries_returns_empty_for_a_missing_directory() {
        assert!(list_rfkill_entries(Path::new("/definitely/not/a/real/rfkill/dir")).is_empty());
    }

    #[test]
    fn find_entries_filters_by_type() {
        let dir = fixture_dir("find-entries");
        fs::create_dir_all(&dir).expect("create fixture dir");
        write_device(&dir, "rfkill0", 0, "bluetooth", 0, 0);
        write_device(&dir, "rfkill1", 1, "wlan", 0, 1);

        let bluetooth = find_entries(&dir, RfkillDeviceType::Bluetooth);
        assert_eq!(bluetooth.len(), 1);
        assert_eq!(bluetooth[0].index, 0);

        let wlan = find_entries(&dir, RfkillDeviceType::Wlan);
        assert_eq!(wlan.len(), 1);
        assert!(wlan[0].hard);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rfkill_index_for_net_interface_finds_the_phy80211_rfkill_index() {
        let dir = fixture_dir("net-interface");
        let phy_dir = dir.join("wlan0").join("phy80211");
        fs::create_dir_all(phy_dir.join("rfkill3")).expect("create fixture dir");
        fs::write(phy_dir.join("rfkill3").join("index"), "3\n").expect("write index");

        assert_eq!(rfkill_index_for_net_interface(&dir, "wlan0"), Some(3));
        assert_eq!(rfkill_index_for_net_interface(&dir, "eth0"), None);
        assert_eq!(rfkill_index_for_net_interface(&dir, ""), None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_rfkill_soft_and_hard_blocked_read_the_real_host_state() {
        // Smoke test against the live system, same precedent as other sysfs-reading modules in
        // this crate: just confirm the real `/sys/class/rfkill` state is readable without
        // panicking, whatever it reports (this dev host's actual radio state isn't asserted on).
        let _ = is_rfkill_soft_blocked(RfkillDeviceType::Wlan);
        let _ = is_rfkill_hard_blocked(RfkillDeviceType::Bluetooth);
    }
}
