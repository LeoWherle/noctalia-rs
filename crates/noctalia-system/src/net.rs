//! Port of the network-reading portion of `src/system/system_monitor_service.{h,cpp}`:
//! `SystemMonitorService::readNetBytes` plus the per-interface throughput math inlined in
//! `samplingLoop`.
//!
//! Only these pure functions are ported here, not the owning `SystemMonitorService` class
//! (polling thread, previous-sample state, history rings) — see task 5.6.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Cumulative byte counters for one interface since boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IfaceBytes {
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Throughput {
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
}

/// Port of `readNetBytes`. Skips the 2 `/proc/net/dev` header lines; an interface with both
/// counters at 0 is dropped (matches the C++'s `if (rxBytes == 0 && txBytes == 0) continue;`).
/// `None` when the file can't be opened.
///
/// Divergence (unreachable in practice, not a behavior change): the C++ reads through one
/// `istringstream` per line via `operator>>`, so once `rxBytes` fails to extract the stream is
/// left in a fail state and every subsequent extraction on that line (the 7 skipped fields, then
/// `txBytes`) is a no-op, leaving `txBytes` at 0 too. This reads the 7 skipped fields and
/// `txBytes` independently by position, so a line with an unparseable rx field but well-formed
/// later fields would read a nonzero `txBytes` here where the C++ would report 0. Real
/// kernel-generated `/proc/net/dev` rows are always well-formed integers, so this never triggers
/// in practice — same class of divergence already recorded for `cpu_stat::parse_line`'s trailing
/// fields.
pub fn read_net_bytes(net_dev_path: &Path) -> Option<HashMap<String, IfaceBytes>> {
    let file = fs::File::open(net_dev_path).ok()?;
    let mut lines = BufReader::new(file).lines();
    // Skip 2 header lines.
    lines.next();
    lines.next();

    let mut result = HashMap::new();
    for line in lines {
        let Ok(line) = line else { break };
        let Some(colon) = line.find(':') else {
            continue;
        };
        let iface = line[..colon].trim_start_matches(' ').to_string();

        let mut fields = line[colon + 1..].split_whitespace();
        let rx_bytes = fields
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // Skip 7 fields (rx_packets, errs, drop, fifo, frame, compressed, multicast).
        for _ in 0..7 {
            fields.next();
        }
        let tx_bytes = fields
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        if rx_bytes == 0 && tx_bytes == 0 {
            continue;
        }

        result.insert(
            iface,
            IfaceBytes {
                rx: rx_bytes,
                tx: tx_bytes,
            },
        );
    }

    Some(result)
}

/// Port of the per-interface throughput math in `samplingLoop`, given the current and previous
/// `readNetBytes` samples and the poll interval. `interval_seconds` is the *configured* poll
/// interval, not measured elapsed wall time — the C++ computes the same way
/// (`scale = 1 / networkInterval`), assuming the poll actually happened on schedule. A counter
/// that went backwards (interface reset) reports 0 for that direction rather than underflowing.
/// Returns `(total_rx_bytes_per_sec, total_tx_bytes_per_sec, per_interface)`; totals exclude the
/// loopback interface (`"lo"`), but `per_interface` still reports it.
pub fn throughput_since(
    current: &HashMap<String, IfaceBytes>,
    previous: &HashMap<String, IfaceBytes>,
    interval_seconds: f64,
) -> (f64, f64, HashMap<String, Throughput>) {
    let scale = if interval_seconds > 0.0 {
        1.0 / interval_seconds
    } else {
        1.0
    };

    let mut total_rx = 0.0;
    let mut total_tx = 0.0;
    let mut by_interface = HashMap::with_capacity(current.len());

    for (iface, cur) in current {
        let mut iface_rx = 0.0;
        let mut iface_tx = 0.0;
        if let Some(prev) = previous.get(iface) {
            if cur.rx >= prev.rx {
                iface_rx = (cur.rx - prev.rx) as f64 * scale;
            }
            if cur.tx >= prev.tx {
                iface_tx = (cur.tx - prev.tx) as f64 * scale;
            }
        }
        if iface != "lo" {
            total_rx += iface_rx;
            total_tx += iface_tx;
        }
        by_interface.insert(
            iface.clone(),
            Throughput {
                rx_bytes_per_sec: iface_rx,
                tx_bytes_per_sec: iface_tx,
            },
        );
    }

    (total_rx, total_tx, by_interface)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-net-test-{}-{}",
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

    // Built with an explicit `\n`-joined Vec rather than a multi-line string literal: a `\`
    // line-continuation would eat the leading spaces before each interface name, which this
    // fixture needs intact (real /proc/net/dev right-pads interface names with spaces).
    fn net_dev_fixture() -> String {
        [
            "Inter-|   Receive                                                |  Transmit",
            " face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed",
            "    lo:  1000       5    0    0    0     0          0         0     1000       5    0    0    0     0       0          0",
            "  eth0: 500000     300    0    0    0     0          0         0    20000     150    0    0    0     0       0          0",
            " wlan0:      0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0",
        ]
        .join("\n")
    }

    #[test]
    fn reads_nonzero_interfaces_and_drops_all_zero_ones() {
        let dir = tempfile_dir();
        let path = write_file(&dir, "net_dev", &net_dev_fixture());

        let bytes = read_net_bytes(&path).expect("should parse");
        assert_eq!(bytes.len(), 2, "wlan0 (all-zero) should be dropped");
        assert_eq!(bytes["lo"], IfaceBytes { rx: 1000, tx: 1000 });
        assert_eq!(
            bytes["eth0"],
            IfaceBytes {
                rx: 500000,
                tx: 20000
            }
        );
        assert!(!bytes.contains_key("wlan0"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(read_net_bytes(Path::new("/does/not/exist")).is_none());
    }

    #[test]
    fn throughput_since_excludes_loopback_from_totals_but_not_per_interface() {
        let mut prev = HashMap::new();
        prev.insert("lo".to_string(), IfaceBytes { rx: 1000, tx: 1000 });
        prev.insert(
            "eth0".to_string(),
            IfaceBytes {
                rx: 500000,
                tx: 20000,
            },
        );

        let mut cur = HashMap::new();
        cur.insert("lo".to_string(), IfaceBytes { rx: 3000, tx: 3000 });
        cur.insert(
            "eth0".to_string(),
            IfaceBytes {
                rx: 600000,
                tx: 40000,
            },
        );

        let (total_rx, total_tx, by_iface) = throughput_since(&cur, &prev, 2.0);
        assert_eq!(
            total_rx, 50000.0,
            "only eth0's delta/2s should count toward totals"
        );
        assert_eq!(total_tx, 10000.0);
        assert_eq!(
            by_iface["lo"],
            Throughput {
                rx_bytes_per_sec: 1000.0,
                tx_bytes_per_sec: 1000.0
            },
            "lo should still appear in the per-interface map"
        );
    }

    #[test]
    fn throughput_since_treats_unseen_or_backwards_counters_as_zero() {
        let prev = HashMap::new(); // no prior sample: every interface is "new"
        let mut cur = HashMap::new();
        cur.insert("eth0".to_string(), IfaceBytes { rx: 100, tx: 100 });
        let (total_rx, total_tx, _) = throughput_since(&cur, &prev, 1.0);
        assert_eq!((total_rx, total_tx), (0.0, 0.0));

        let mut prev2 = HashMap::new();
        prev2.insert("eth0".to_string(), IfaceBytes { rx: 1000, tx: 1000 });
        let mut cur2 = HashMap::new();
        // Interface reset: counters went backwards.
        cur2.insert("eth0".to_string(), IfaceBytes { rx: 10, tx: 10 });
        let (total_rx2, total_tx2, _) = throughput_since(&cur2, &prev2, 1.0);
        assert_eq!(
            (total_rx2, total_tx2),
            (0.0, 0.0),
            "backwards counters should report 0, not underflow"
        );
    }

    #[test]
    fn throughput_since_treats_nonpositive_interval_as_unscaled() {
        let mut prev = HashMap::new();
        prev.insert("eth0".to_string(), IfaceBytes { rx: 100, tx: 100 });
        let mut cur = HashMap::new();
        cur.insert("eth0".to_string(), IfaceBytes { rx: 300, tx: 300 });

        let (total_rx, total_tx, _) = throughput_since(&cur, &prev, 0.0);
        assert_eq!((total_rx, total_tx), (200.0, 200.0));
    }
}
