//! Port of `src/system/cpu_stat.{h,cpp}`, plus (task 5.6.5.1) the private
//! `SystemMonitorService::readLoadAvg` from `system_monitor_service.cpp` — the one leftover
//! `/proc` stat reader the earlier `mem`/`net`/`disk`/`cpu_temp` pure-reader split didn't cover,
//! grouped here since it pairs naturally with this module's other `/proc` readers.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Idle and total jiffies accumulated by one CPU since boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub total: u64,
    pub idle: u64,
}

/// Everything before the first space: "cpu" for the aggregate row, "cpuN" for a core. Empty for
/// a row with no space at all.
fn label_of(line: &str) -> &str {
    match line.find(' ') {
        Some(idx) => &line[..idx],
        None => "",
    }
}

fn is_core_label(label: &str) -> bool {
    match label.strip_prefix("cpu") {
        Some(index) if !index.is_empty() => index.bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

/// Parses one `/proc/stat` cpu row. `expected_label` pins which row is accepted ("cpu" for the
/// aggregate, "cpuN" for a core), so the aggregate cannot be mistaken for core 0. guest and
/// guest_nice are not read: the kernel already folds them into user and nice.
pub fn parse_line(line: &str, expected_label: &str) -> Option<Totals> {
    let mut fields = line.split_whitespace();
    let label = fields.next()?;
    // user/nice/system/idle are mandatory, so a row with a plausible label but unreadable
    // counters is rejected rather than read as all-zero.
    let user: u64 = fields.next()?.parse().ok()?;
    let nice: u64 = fields.next()?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    let idle: u64 = fields.next()?.parse().ok()?;
    if label != expected_label {
        return None;
    }
    // The trailing fields are optional: a kernel predating one leaves it zero rather than
    // poisoning the sum. (Minor divergence from the C++'s `istream::operator>>` chain: an
    // unparseable trailing field only zeroes that one field here, where the C++'s sticky
    // stream-fail state would also zero every field after it — unreachable in practice, real
    // `/proc/stat` rows never contain a malformed numeric field.)
    let iowait: u64 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let irq: u64 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let softirq: u64 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let steal: u64 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    Some(Totals {
        idle: idle + iowait,
        total: user + nice + system + idle + iowait + irq + softirq + steal,
    })
}

/// Busy percentage in `[0, 100]` between two samples, or `None` when the window holds no jiffies
/// or the counters went backwards (suspend/resume, container reset).
pub fn usage_between(prev: &Totals, current: &Totals) -> Option<f64> {
    if current.total <= prev.total {
        return None;
    }
    let total_delta = current.total - prev.total;
    let idle_delta = current.idle.saturating_sub(prev.idle);
    let busy = 1.0 - (idle_delta as f64 / total_delta as f64);
    Some((100.0 * busy).clamp(0.0, 100.0))
}

/// The stat path is a parameter so tests can feed fixtures instead of the live `/proc`.
pub fn read_totals(stat_path: &Path) -> Option<Totals> {
    let file = fs::File::open(stat_path).ok()?;
    let mut line = String::new();
    let bytes_read = BufReader::new(file).read_line(&mut line).ok()?;
    if bytes_read == 0 {
        return None;
    }
    parse_line(&line, "cpu")
}

/// Online cores in `/proc/stat` order, skipping the leading aggregate row. Offline cores are
/// absent from the file, so the "cpuN" labels are not necessarily contiguous and an entry's
/// position is not its core id. `None` when the file is unreadable, holds no cpuN rows, or holds
/// one that does not parse.
pub fn read_core_totals(stat_path: &Path) -> Option<Vec<Totals>> {
    let file = fs::File::open(stat_path).ok()?;
    let mut cores = Vec::new();
    // The cpu rows lead the file, so stop at the first row that is not one rather than scanning
    // the whole of /proc/stat.
    for line in BufReader::new(file).lines() {
        let line = line.ok()?;
        let label = label_of(&line);
        if label == "cpu" {
            continue; // the aggregate row
        }
        if !is_core_label(label) {
            break;
        }
        // The expected label comes from the row itself: offline cores are omitted from
        // /proc/stat, so predicting "cpu" + cores.len() would desync permanently at the first gap.
        cores.push(parse_line(&line, label)?);
    }

    if cores.is_empty() { None } else { Some(cores) }
}

/// Port of the private `readLoadAvg`: the 1/5/15-minute load averages from `/proc/loadavg`.
/// `None` when the file is unreadable or its first three whitespace-separated fields aren't all
/// parseable floats (matches the C++'s `file >> la[0] >> la[1] >> la[2]` plus `file.fail()`
/// check).
pub fn read_load_avg(loadavg_path: &Path) -> Option<[f64; 3]> {
    let mut content = String::new();
    fs::File::open(loadavg_path)
        .ok()?
        .read_to_string(&mut content)
        .ok()?;
    let mut fields = content.split_whitespace();
    let one: f64 = fields.next()?.parse().ok()?;
    let five: f64 = fields.next()?.parse().ok()?;
    let fifteen: f64 = fields.next()?.parse().ok()?;
    Some([one, five, fifteen])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_stat(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut file = fs::File::create(&path).expect("create fixture");
        file.write_all(text.as_bytes()).expect("write fixture");
        path
    }

    #[test]
    fn parse_line_folds_iowait_into_idle_and_sums_all_fields() {
        let totals =
            parse_line("cpu  100 20 30 700 50 5 5 10", "cpu").expect("aggregate row should parse");
        assert_eq!(totals.idle, 750, "idle should fold iowait in (700 + 50)");
        assert_eq!(totals.total, 920, "total should sum all eight fields");
    }

    #[test]
    fn parse_line_pins_the_expected_label() {
        assert!(parse_line("cpu  100 20 30 700 50 5 5 10", "cpu0").is_none());
        assert!(parse_line("cpu0 100 20 30 700 50 5 5 10", "cpu").is_none());
        assert!(parse_line("cpu7 1 2 3 4 5 6 7 8", "cpu7").is_some());
        assert!(parse_line("intr 12345 0 0", "cpu").is_none());
    }

    #[test]
    fn parse_line_handles_missing_trailing_fields() {
        let short_row = parse_line("cpu  100 20 30 700 50 5 5", "cpu")
            .expect("row without steal should still parse");
        assert_eq!(
            short_row.total, 910,
            "total should omit the absent steal field"
        );
    }

    #[test]
    fn usage_between_computes_busy_percentage() {
        assert!(
            (usage_between(
                &Totals {
                    total: 1000,
                    idle: 800
                },
                &Totals {
                    total: 1100,
                    idle: 825
                }
            )
            .unwrap()
                - 75.0)
                .abs()
                < 0.001
        );
        assert_eq!(
            usage_between(
                &Totals {
                    total: 1000,
                    idle: 800
                },
                &Totals {
                    total: 1100,
                    idle: 900
                }
            ),
            Some(0.0)
        );
        assert_eq!(
            usage_between(
                &Totals {
                    total: 1000,
                    idle: 800
                },
                &Totals {
                    total: 1100,
                    idle: 800
                }
            ),
            Some(100.0)
        );
    }

    #[test]
    fn usage_between_rejects_empty_or_backwards_windows() {
        assert!(
            usage_between(
                &Totals {
                    total: 1000,
                    idle: 800
                },
                &Totals {
                    total: 1000,
                    idle: 800
                }
            )
            .is_none()
        );
        assert!(
            usage_between(
                &Totals {
                    total: 2000,
                    idle: 900
                },
                &Totals {
                    total: 1000,
                    idle: 400
                }
            )
            .is_none()
        );
        // Idle going backwards (but total forwards) must clamp to 100, not wrap via underflow.
        assert_eq!(
            usage_between(
                &Totals {
                    total: 1000,
                    idle: 900
                },
                &Totals {
                    total: 1100,
                    idle: 500
                }
            ),
            Some(100.0)
        );
    }

    #[test]
    fn read_fixtures_round_trip() {
        let dir = tempfile_dir();
        let stat = "cpu  400 0 100 500 0 0 0 0\n\
                    cpu0 100 0 25 125 0 0 0 0\n\
                    cpu1 100 0 25 125 0 0 0 0\n\
                    cpu2 100 0 25 125 0 0 0 0\n\
                    cpu3 100 0 25 125 0 0 0 0\n\
                    intr 999 0 0\n\
                    ctxt 4242\n";
        let path = write_stat(&dir, "stat", stat);

        let aggregate = read_totals(&path).expect("aggregate should read from a fixture");
        assert_eq!(aggregate.total, 1000);
        assert_eq!(aggregate.idle, 500);

        let cores = read_core_totals(&path).expect("cores should read from a fixture");
        assert_eq!(
            cores.len(),
            4,
            "should find exactly 4 cores, skipping the aggregate row"
        );
        assert_eq!(
            cores[0],
            Totals {
                total: 250,
                idle: 125
            }
        );
        assert_eq!(
            cores[3],
            Totals {
                total: 250,
                idle: 125
            }
        );

        let offlined = "cpu  400 0 100 500 0 0 0 0\n\
                         cpu0 100 0 25 125 0 0 0 0\n\
                         cpu1 100 0 25 125 0 0 0 0\n\
                         cpu3 100 0 25 125 0 0 0 0\n\
                         cpu5 100 0 25 125 0 0 0 0\n\
                         intr 999 0 0\n";
        let gapped = read_core_totals(&write_stat(&dir, "stat-offlined", offlined));
        assert_eq!(
            gapped.map(|c| c.len()),
            Some(4),
            "non-contiguous cpuN labels should all be reported"
        );

        let single = read_core_totals(&write_stat(
            &dir,
            "stat-single",
            "cpu  8 0 0 2 0 0 0 0\ncpu0 8 0 0 2 0 0 0 0\n",
        ));
        assert_eq!(single.map(|c| c.len()), Some(1));

        assert!(
            read_core_totals(&write_stat(
                &dir,
                "stat-nocores",
                "cpu  8 0 0 2 0 0 0 0\nintr 1\n"
            ))
            .is_none(),
            "stat without cpuN rows should yield None"
        );

        assert!(
            read_core_totals(&write_stat(
                &dir,
                "stat-broken",
                "cpu  8 0 0 2 0 0 0 0\ncpu0 8 0 0 2\ncpu1 x\n"
            ))
            .is_none(),
            "an unparseable cpuN row should yield None"
        );

        assert!(read_totals(&dir.join("does-not-exist")).is_none());
        assert!(read_core_totals(&dir.join("does-not-exist")).is_none());
        assert!(read_totals(&write_stat(&dir, "stat-empty", "")).is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_core_deltas_are_computed_independently() {
        let prev = [
            Totals {
                total: 100,
                idle: 100,
            },
            Totals {
                total: 100,
                idle: 100,
            },
        ];
        let next = [
            Totals {
                total: 200,
                idle: 100,
            },
            Totals {
                total: 200,
                idle: 200,
            },
        ];
        assert_eq!(usage_between(&prev[0], &next[0]), Some(100.0));
        assert_eq!(usage_between(&prev[1], &next[1]), Some(0.0));
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-cpu-stat-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn read_load_avg_parses_the_first_three_fields() {
        let dir = tempfile_dir();
        let path = write_stat(&dir, "loadavg", "0.52 0.58 0.59 2/1234 56789\n");
        assert_eq!(read_load_avg(&path), Some([0.52, 0.58, 0.59]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_load_avg_rejects_missing_or_malformed_files() {
        assert!(read_load_avg(Path::new("/does/not/exist")).is_none());

        let dir = tempfile_dir();
        let short = write_stat(&dir, "loadavg-short", "0.52 0.58\n");
        assert!(
            read_load_avg(&short).is_none(),
            "fewer than 3 fields should fail"
        );

        let malformed = write_stat(&dir, "loadavg-malformed", "0.52 not-a-number 0.59\n");
        assert!(read_load_avg(&malformed).is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
