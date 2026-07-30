//! Port of the memory-reading portion of `src/system/system_monitor_service.{h,cpp}`:
//! `SystemMonitorService::readMemoryKb` and its private `readZfsEvictableArcKb` helper.
//!
//! Only the pure stat-reading function is ported here, not the owning `SystemMonitorService`
//! class (polling thread, history rings, GPU vendor readers, retain/release ref-counting) — that
//! orchestration is far larger than "stat readers" and is tracked under task 5.6.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemData {
    pub total_kb: u64,
    pub used_kb: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
}

/// Port of the private `readZfsEvictableArcKb`. `0` (not evictable) when the file is missing,
/// unreadable, or the ARC is already at its floor.
fn read_zfs_evictable_arc_kb(path: &Path) -> u64 {
    let Ok(file) = fs::File::open(path) else {
        return 0;
    };

    let mut arc_size: u64 = 0;
    let mut arc_min: u64 = 0;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let mut fields = line.split_whitespace();
        let (Some(key), Some(kind), Some(value)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // The C++'s `iss >> key >> type >> value` requires all three tokens to extract, even
        // though `type` itself is discarded.
        if kind.parse::<u32>().is_err() {
            continue;
        }
        let Ok(value) = value.parse::<u64>() else {
            continue;
        };
        match key {
            "size" => arc_size = value,
            "c_min" => arc_min = value,
            _ => {}
        }
    }

    arc_size.saturating_sub(arc_min) / 1024
}

/// Port of `readMemoryKb`. `None` when `/proc/meminfo` is unreadable, is missing `MemTotal`/
/// `MemAvailable`, or reports available memory exceeding total (all treated as "can't trust this
/// sample", matching the C++'s guard).
///
/// Divergence (unreachable in practice, not a behavior change): this parses `/proc/meminfo`
/// line-by-line, where the C++ reads `key value unit` as a raw token stream spanning line
/// boundaries (`while (file >> key >> value_kb >> unit)`), stopping the whole read the first time
/// a triple fails to extract. Every real `/proc/meminfo` line up to `SwapFree` (where the C++
/// stops) is a well-formed 3-token `key value kB` line, so the two approaches agree on every real
/// kernel's output; a fixture missing the unit column on the affected lines could observe a
/// difference (this implementation would keep reading using its own line boundaries, whereas the
/// C++'s stream would resync mid-line into the next line's fields).
pub fn read_memory_kb(meminfo_path: &Path, zfs_arcstats_path: &Path) -> Option<MemData> {
    let file = fs::File::open(meminfo_path).ok()?;

    let mut total_kb = 0u64;
    let mut available_kb = 0u64;
    let mut swap_total_kb = 0u64;
    let mut swap_free_kb = 0u64;

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let mut fields = line.split_whitespace();
        let (Some(key), Some(value_str), Some(_unit)) =
            (fields.next(), fields.next(), fields.next())
        else {
            break;
        };
        let Ok(value_kb) = value_str.parse::<u64>() else {
            break;
        };
        match key {
            "MemTotal:" => total_kb = value_kb,
            "MemAvailable:" => available_kb = value_kb,
            "SwapTotal:" => swap_total_kb = value_kb,
            "SwapFree:" => swap_free_kb = value_kb,
            _ => {}
        }

        // SwapFree appears last, after that there's nothing we need.
        if key == "SwapFree:" {
            break;
        }
    }

    if total_kb == 0 || available_kb == 0 || available_kb > total_kb {
        return None;
    }

    let zfs_arc_kb = read_zfs_evictable_arc_kb(zfs_arcstats_path);
    let available_kb = (available_kb + zfs_arc_kb).min(total_kb);

    Some(MemData {
        total_kb,
        used_kb: total_kb - available_kb,
        swap_total_kb,
        swap_used_kb: swap_total_kb.saturating_sub(swap_free_kb),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-mem-test-{}-{}",
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

    const MEMINFO: &str = "MemTotal:       16333484 kB\n\
                            MemFree:         1000000 kB\n\
                            MemAvailable:    9000000 kB\n\
                            Buffers:          200000 kB\n\
                            Cached:          3000000 kB\n\
                            SwapCached:            0 kB\n\
                            SwapTotal:       8000000 kB\n\
                            SwapFree:        6000000 kB\n\
                            HugePages_Total:       0\n";

    #[test]
    fn reads_totals_and_used_swap() {
        let dir = tempfile_dir();
        let meminfo = write_file(&dir, "meminfo", MEMINFO);
        let missing_arcstats = dir.join("does-not-exist-arcstats");

        let data = read_memory_kb(&meminfo, &missing_arcstats).expect("should parse");
        assert_eq!(data.total_kb, 16333484);
        assert_eq!(data.used_kb, 16333484 - 9000000);
        assert_eq!(data.swap_total_kb, 8000000);
        assert_eq!(data.swap_used_kb, 2000000);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn zfs_evictable_arc_reduces_used_memory() {
        let dir = tempfile_dir();
        let meminfo = write_file(&dir, "meminfo", MEMINFO);
        // size - c_min = 500000 KB evictable -> 500000/1024 = 488 KB folded into "available".
        let arcstats = write_file(
            &dir,
            "arcstats",
            "name                            type data\n\
             size                            4    512000000\n\
             c_min                           4    11500000\n",
        );

        let without_arc = read_memory_kb(&meminfo, &dir.join("does-not-exist")).expect("baseline");
        let with_arc = read_memory_kb(&meminfo, &arcstats).expect("with arc");
        assert!(
            with_arc.used_kb < without_arc.used_kb,
            "evictable ARC should reduce reported used memory"
        );
        assert_eq!(
            without_arc.used_kb - with_arc.used_kb,
            (512000000 - 11500000) / 1024
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn zfs_available_never_exceeds_total() {
        let dir = tempfile_dir();
        // Available is already 99% of total; a huge evictable ARC must clamp, not overflow used
        // below zero.
        let meminfo = write_file(
            &dir,
            "meminfo",
            "MemTotal:       1000000 kB\n\
             MemAvailable:    990000 kB\n\
             SwapTotal:            0 kB\n\
             SwapFree:              0 kB\n",
        );
        let arcstats = write_file(
            &dir,
            "arcstats",
            "size                            4    999999999\n\
             c_min                           4    0\n",
        );

        let data = read_memory_kb(&meminfo, &arcstats).expect("should parse");
        assert_eq!(data.total_kb, 1000000);
        assert_eq!(
            data.used_kb, 0,
            "available should clamp to total, not exceed it"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_missing_or_inconsistent_fields() {
        let dir = tempfile_dir();
        assert!(read_memory_kb(&dir.join("nope"), &dir.join("nope")).is_none());

        let no_available = write_file(
            &dir,
            "no-available",
            "MemTotal: 1000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        );
        assert!(read_memory_kb(&no_available, &dir.join("nope")).is_none());

        let inconsistent = write_file(
            &dir,
            "inconsistent",
            "MemTotal: 1000 kB\nMemAvailable: 2000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        );
        assert!(
            read_memory_kb(&inconsistent, &dir.join("nope")).is_none(),
            "available exceeding total should be rejected"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn zfs_arcstats_ignores_malformed_lines_and_missing_file() {
        assert_eq!(
            read_zfs_evictable_arc_kb(Path::new("/does/not/exist")),
            0,
            "a missing arcstats file should contribute nothing"
        );

        let dir = tempfile_dir();
        let arcstats = write_file(
            &dir,
            "arcstats",
            "size not-a-number 123\n\
             size                            4    2048\n\
             c_min                           4    1024\n\
             junk line with too few fields\n",
        );
        // (2048 - 1024) / 1024 = 1
        assert_eq!(read_zfs_evictable_arc_kb(&arcstats), 1);

        let below_floor = write_file(&dir, "below-floor", "size 4 1024\nc_min 4 2048\n");
        assert_eq!(
            read_zfs_evictable_arc_kb(&below_floor),
            0,
            "ARC at or below its floor should not be evictable"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
