//! Port of `src/system/disk_mounts.{h,cpp}` (mount enumeration) and the private
//! `readDiskStatvfs` helper from `src/system/system_monitor_service.cpp` (per-path usage via
//! `statvfs`).
//!
//! Only these pure stat-reading functions are ported here, not the owning `SystemMonitorService`
//! class (retain/release ref-counting per watched path, history rings, polling thread) — see
//! task 5.6.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskMount {
    pub path: String,
    pub source: String,
    pub filesystem: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DiskStats {
    pub usage_percent: f32,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub available_bytes: u64,
}

/// `/proc/mounts` escapes space, tab, newline and backslash as octal `\NNN` sequences.
///
/// Divergence (unreachable in practice): this decodes into a `String`, requiring valid UTF-8,
/// where the C++'s `std::string` is a raw byte buffer with no such requirement. Real mount
/// sources/paths are ASCII or valid-UTF-8 device/directory names; a mount path containing
/// arbitrary non-UTF-8 bytes would lossily replace them here instead of preserving them exactly.
fn unescape_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
        {
            let value =
                (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(value);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Port of `physicalDiskMounts`: block-device-backed filesystems from `mounts_file`, deduped by
/// source (keeping the shortest mount path so a device's root wins over its bind mounts/
/// subvolumes) and sorted by path. Pseudo filesystems, loop/squashfs mounts, and boot mounts are
/// excluded. Returns an empty `Vec` when `mounts_file` can't be opened.
pub fn physical_disk_mounts(mounts_file: &Path) -> Vec<DiskMount> {
    let Ok(file) = fs::File::open(mounts_file) else {
        return Vec::new();
    };

    let mut by_source: HashMap<String, DiskMount> = HashMap::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let mut fields = line.split_whitespace();
        let (Some(source), Some(path), Some(filesystem)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };

        let source = unescape_mount_field(source);
        let path = unescape_mount_field(path);
        if !source.starts_with("/dev/")
            || source.starts_with("/dev/loop")
            || filesystem == "squashfs"
        {
            continue;
        }
        if path == "/boot" || path.starts_with("/boot/") {
            continue;
        }

        match by_source.get(&source) {
            Some(existing) if path.len() >= existing.path.len() => {}
            _ => {
                by_source.insert(
                    source.clone(),
                    DiskMount {
                        path,
                        source,
                        filesystem: filesystem.to_string(),
                    },
                );
            }
        }
    }

    let mut mounts: Vec<DiskMount> = by_source.into_values().collect();
    mounts.sort_by(|a, b| a.path.cmp(&b.path));
    mounts
}

/// Port of the private `readDiskStatvfs`. `None` when `statvfs` fails or reports zero total
/// blocks (an unmounted or bogus path), matching the C++'s `sv.f_blocks == 0` guard.
pub fn read_disk_statvfs(path: &Path) -> Option<DiskStats> {
    // A path containing an interior NUL byte can't be a real POSIX path (illegal in filenames).
    // Divergence (unreachable in practice): the C++'s `path.c_str()` would instead truncate at
    // that byte and call `statvfs` on the shorter prefix, possibly succeeding against a
    // different path, where this returns `None` outright without calling `statvfs` at all.
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

    // SAFETY: `libc::statvfs` is a C struct of integer fields; the all-zero bit pattern is a
    // valid value for each of them, and it is fully overwritten by the `statvfs` call below on
    // success.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a NUL-terminated C string valid for the call, and `stat` is a valid,
    // uniquely-owned out-parameter the kernel writes into on success.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 || stat.f_blocks == 0 {
        return None;
    }

    let total = stat.f_blocks as f64;
    let free_blocks = stat.f_bfree as f64;
    let used = total - free_blocks;
    let block_size = stat.f_frsize as u64;

    Some(DiskStats {
        usage_percent: (100.0 * used / total) as f32,
        total_bytes: stat.f_blocks as u64 * block_size,
        free_bytes: stat.f_bfree as u64 * block_size,
        available_bytes: stat.f_bavail as u64 * block_size,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-disk-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_file(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut file = fs::File::create(&path).expect("create fixture");
        file.write_all(text.as_bytes()).expect("write fixture");
        path
    }

    // Direct port of `tests/disk_mounts_test.cpp`'s single fixture and its exact assertions.
    #[test]
    fn physical_disk_mounts_matches_cpp_fixture() {
        let dir = tempfile_dir();
        let fixture = write_file(
            &dir,
            "mounts",
            "/dev/nvme0n1p2 / btrfs rw 0 0\n\
             /dev/nvme0n1p2 /home btrfs rw 0 0\n\
             /dev/sdb1 /mnt/My\\040Disk ext4 rw 0 0\n\
             /dev/mapper/vg-data /srv/data xfs rw 0 0\n\
             /dev/sda1 /boot/efi vfat rw 0 0\n\
             /dev/loop0 /snap/app squashfs ro 0 0\n\
             proc /proc proc rw 0 0\n\
             malformed\n",
        );

        let mounts = physical_disk_mounts(&fixture);
        assert_eq!(
            mounts.len(),
            3,
            "only deduplicated physical data mounts should remain"
        );
        assert_eq!(
            mounts[0],
            DiskMount {
                path: "/".to_string(),
                source: "/dev/nvme0n1p2".to_string(),
                filesystem: "btrfs".to_string()
            },
            "the shortest mount for a source should win and root should sort first"
        );
        assert_eq!(
            mounts[1],
            DiskMount {
                path: "/mnt/My Disk".to_string(),
                source: "/dev/sdb1".to_string(),
                filesystem: "ext4".to_string()
            },
            "escaped mount paths should be decoded"
        );
        assert_eq!(
            mounts[2],
            DiskMount {
                path: "/srv/data".to_string(),
                source: "/dev/mapper/vg-data".to_string(),
                filesystem: "xfs".to_string()
            },
            "device source and filesystem should be preserved"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn physical_disk_mounts_on_unreadable_file_returns_empty() {
        assert!(physical_disk_mounts(Path::new("/does/not/exist")).is_empty());
    }

    #[test]
    fn read_disk_statvfs_reports_real_usage_for_a_live_path() {
        let stats =
            read_disk_statvfs(&std::env::temp_dir()).expect("temp dir should be statvfs-able");
        assert!(stats.total_bytes > 0);
        assert!(stats.total_bytes >= stats.free_bytes);
        assert!((0.0..=100.0).contains(&stats.usage_percent));
    }

    #[test]
    fn read_disk_statvfs_on_nonexistent_path_returns_none() {
        assert!(read_disk_statvfs(Path::new("/does/not/exist/at/all")).is_none());
    }
}
