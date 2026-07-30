//! Port of the DDC/CI (`ddcutil`) slice of `src/system/brightness_service.cpp` (task 5.3.2):
//! subprocess argv building, `ddcutil` output parsing, and the detect/query orchestration built
//! on them. Self-contained — no Wayland/D-Bus dependency, unlike task 5.3.3 (the rest of
//! `BrightnessService`, which attributes a DDC display to a Wayland connector and folds it into
//! the shared backlight-candidate machinery).
//!
//! `detect_ddc_displays` is restructured relative to the C++ for testability, not behavior: the
//! C++'s `detectDdcDisplays` inlines a live per-display `queryDdcBrightness` subprocess call
//! directly inside its `flushCurrent` line-parsing closure, so the parsing and the live I/O can't
//! be exercised independently. This splits that into a pure `parse_ddc_detect_output` (parses
//! `ddcutil detect` text into candidate stubs, no subprocess involved — testable against fixture
//! text) and `detect_ddc_displays` (re-attaches the live per-candidate brightness query in the
//! same order and under the same gates the C++ applies, so the end-to-end candidate set this
//! produces is identical to the C++'s). Same precedent as task 1.6.4's `command_exists_on_path`/
//! `resolve_privilege_escalator_on_path` extraction (session 12) — a testability-driven pure-core
//! extraction that doesn't change observable behavior.
//!
//! No `ddcutil` binary or DDC/CI-capable display is available on this dev host (confirmed:
//! `ddcutil` isn't on `$PATH` in the Nix devshell, and it isn't a devshell/build dependency
//! anywhere in this repo — the C++ only ever shells out to it as an optional *runtime* tool, same
//! as `pkexec`/`run0` in task 1.6.3), so `detect_ddc_displays`/`query_ddc_brightness`'s live path
//! has no manual end-to-end check to log here, unlike task 1.6.5's systemd check.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use noctalia_core::log::Logger;
use noctalia_core::process;

use crate::brightness::c_trim;

/// The C++ shares one `constexpr Logger kLog("brightness")` across the whole
/// `brightness_service.cpp` translation unit; `task 5.3.1` didn't need it (no C++ `kLog` calls
/// fall inside its ported slice), but 5.3.2's `detectDdcDisplays` does, so it lands here.
const LOG: Logger = Logger::new("brightness");

pub const DDC_QUERY_TIMEOUT: Duration = Duration::from_millis(10_000);

/// Port of the free function `parseTrailingInteger`: the contiguous run of ASCII digits
/// immediately preceding the end of `input` (after any trailing non-digit characters are
/// skipped), or `None` if `input` has no digits at all. Never negative — a `-` immediately before
/// the digit run is not part of it, matching the C++'s digit-only backward scan.
///
/// Divergence (unreachable in practice): a digit run wide enough to overflow `i32` returns `None`
/// here; the C++'s `std::atoi` has implementation-defined behavior on overflow instead. Real
/// `ddcutil` output only ever produces bus numbers and 8-bit VCP values (at most 3 digits).
pub fn parse_trailing_integer(input: &str) -> Option<i32> {
    let bytes = input.as_bytes();
    let mut end = bytes.len();
    while end > 0 && !bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    if start == end {
        return None;
    }

    std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()
}

/// Port of `parseI2cBus`: the bus number from a `/dev/i2c-N` path if present anywhere in `line`,
/// else the trailing integer of the whole line.
pub fn parse_i2c_bus(line: &str) -> Option<i32> {
    match line.find("/dev/i2c-") {
        Some(pos) => parse_trailing_integer(&line[pos..]),
        None => parse_trailing_integer(line),
    }
}

/// Port of `normalizeConnectorName`: trims `raw`, and if it starts with `"card"`, drops the
/// `cardN-` prefix up to and including the first `-` (so `"card1-DP-1"` becomes `"DP-1"`).
pub fn normalize_connector_name(raw: &str) -> String {
    let trimmed = c_trim(raw);
    if let Some(rest) = trimmed.strip_prefix("card")
        && let Some(dash) = rest.find('-')
    {
        return rest[dash + 1..].to_string();
    }
    trimmed.to_string()
}

/// Port of `parseDdcVcpBrightness`: scans `output` line by line (matching the C++'s manual
/// `find('\n')` loop, including its trailing-empty-line iteration) for the first line containing
/// both `"current value"` and `"max value"` (case-insensitive) with `"max value"` after
/// `"current value"`, then reads the trailing integer from each side. `None` if no line qualifies
/// or the trailing integers don't parse / `max` isn't positive.
pub fn parse_ddc_vcp_brightness(output: &str) -> Option<(i32, i32)> {
    for line in output.split('\n') {
        let lower = line.to_ascii_lowercase();
        let Some(current_pos) = lower.find("current value") else {
            continue;
        };
        let Some(max_pos) = lower.find("max value") else {
            continue;
        };
        if max_pos <= current_pos {
            continue;
        }

        let current = parse_trailing_integer(&line[current_pos..max_pos]);
        let max = parse_trailing_integer(&line[max_pos..]);
        if let (Some(current), Some(max)) = (current, max)
            && max > 0
        {
            return Some((current, max));
        }
    }
    None
}

/// Port of `ddcDetectArgs`.
pub fn ddc_detect_args(ignore_mmids: &[String]) -> Vec<String> {
    let mut args = vec!["ddcutil".to_string(), "--noconfig".to_string()];
    for mmid in ignore_mmids {
        args.push("--ignore-mmid".to_string());
        args.push(mmid.clone());
    }
    args.push("detect".to_string());
    args
}

/// Port of `ddcBaseArgs`.
pub fn ddc_base_args(bus: i32) -> Vec<String> {
    vec![
        "ddcutil".to_string(),
        "--noconfig".to_string(),
        "--enable-dynamic-sleep".to_string(),
        "--sleep-multiplier".to_string(),
        "0.1".to_string(),
        "--bus".to_string(),
        bus.to_string(),
    ]
}

/// Port of the anonymous `CommandResult` struct, in-class defaults included (`exit_code: -1`,
/// matching the C++'s `int exitCode = -1;`).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandResult {
    pub launched: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub exit_code: i32,
    pub output: String,
}

impl Default for CommandResult {
    fn default() -> Self {
        Self {
            launched: false,
            timed_out: false,
            cancelled: false,
            exit_code: -1,
            output: String::new(),
        }
    }
}

/// Port of `runCommandCapture`: runs `args` (via `noctalia_core::process::run_sync_with_options`)
/// under `timeout`/`cancel`, and concatenates stdout+stderr with a `\n` separator when both are
/// non-empty (matching the C++'s "append err after out" combining). `process::RunResult`'s
/// `out`/`err` are raw bytes (no UTF-8 guarantee, same reasoning as `noctalia-core::process`'s own
/// `RunResult`); this decodes the combined bytes lossily into `output` — real `ddcutil` output is
/// plain ASCII text, so this is unreachable in practice.
pub fn run_command_capture(
    args: &[String],
    timeout: Duration,
    cancel: &Arc<AtomicBool>,
) -> CommandResult {
    if cancel.load(Ordering::Relaxed) {
        return CommandResult {
            cancelled: true,
            ..Default::default()
        };
    }

    let options = process::RunOptions {
        timeout: Some(timeout),
        cancel: Some(Arc::clone(cancel)),
        ..Default::default()
    };
    let run_result = process::run_sync_with_options(args, options);

    let mut combined = run_result.out;
    if !run_result.err.is_empty() {
        if !combined.is_empty() {
            combined.push(b'\n');
        }
        combined.extend_from_slice(&run_result.err);
    }
    let output = String::from_utf8_lossy(&combined).into_owned();

    let cancelled = cancel.load(Ordering::Relaxed);
    CommandResult {
        launched: run_result.exit_code >= 0,
        timed_out: run_result.timed_out && !cancelled,
        cancelled,
        exit_code: run_result.exit_code,
        output,
    }
}

/// Port of `queryDdcBrightness`. The C++'s `std::string* detailOut` out-param is always passed a
/// real, non-null buffer at its one call site (inside `detectDdcDisplays`), so this returns the
/// detail string directly as part of the tuple rather than threading an `Option<&mut String>`.
pub fn query_ddc_brightness(
    bus: i32,
    timeout: Duration,
    cancel: &Arc<AtomicBool>,
) -> (Option<(i32, i32)>, String) {
    let mut args = ddc_base_args(bus);
    args.push("getvcp".to_string());
    args.push("10".to_string());

    let result = run_command_capture(&args, timeout, cancel);
    let brightness =
        if !result.launched || result.timed_out || result.cancelled || result.exit_code != 0 {
            None
        } else {
            parse_ddc_vcp_brightness(&result.output)
        };
    (brightness, result.output)
}

/// Port of the anonymous `DdcCandidate` struct, in-class defaults included (`bus: -1`,
/// `current_raw: -1`, `max_raw: 100`).
#[derive(Debug, Clone, PartialEq)]
pub struct DdcCandidate {
    pub connector_name: String,
    pub label: String,
    pub bus: i32,
    pub current_raw: i32,
    pub max_raw: i32,
}

impl Default for DdcCandidate {
    fn default() -> Self {
        Self {
            connector_name: String::new(),
            label: String::new(),
            bus: -1,
            current_raw: -1,
            max_raw: 100,
        }
    }
}

/// The pre-live-query gate `flushCurrent` applies before a candidate is even considered for a
/// brightness query: `inDisplay && current.bus >= 0 && !current.connectorName.empty()`.
fn is_queryable_ddc_candidate(candidate: &DdcCandidate, in_display: bool) -> bool {
    in_display && candidate.bus >= 0 && !candidate.connector_name.is_empty()
}

/// Parses `ddcutil detect` output into candidate displays, stopping short of the C++'s inlined
/// live `queryDdcBrightness` call — see the module doc comment for why. Every candidate returned
/// here already passed `flushCurrent`'s pre-query gate (`is_queryable_ddc_candidate`); it's the
/// caller's job (`detect_ddc_displays`) to run the live query and apply the post-query gate.
pub fn parse_ddc_detect_output(output: &str) -> Vec<DdcCandidate> {
    let mut candidates = Vec::new();
    let mut current = DdcCandidate::default();
    let mut in_display = false;

    for raw_line in output.split('\n') {
        let line = c_trim(raw_line);
        if line.starts_with("Display ") {
            if is_queryable_ddc_candidate(&current, in_display) {
                candidates.push(std::mem::take(&mut current));
            }
            current = DdcCandidate::default();
            in_display = !line.starts_with("Display not found");
        } else if line.starts_with("Invalid display") || line.starts_with("DDC_disabled") {
            if is_queryable_ddc_candidate(&current, in_display) {
                candidates.push(std::mem::take(&mut current));
            }
            current = DdcCandidate::default();
            in_display = false;
        } else if line.starts_with("I2C bus:") {
            if let Some(bus) = parse_i2c_bus(line) {
                current.bus = bus;
            }
        } else if let Some(rest) = line.strip_prefix("DRM connector:") {
            current.connector_name = normalize_connector_name(rest);
        } else if let Some(rest) = line.strip_prefix("DRM_connector:") {
            current.connector_name = normalize_connector_name(rest);
        } else if let Some(rest) = line.strip_prefix("Monitor:") {
            current.label = c_trim(rest).to_string();
        } else if let Some(rest) = line.strip_prefix("Model:")
            && current.label.is_empty()
        {
            current.label = c_trim(rest).to_string();
        }
    }

    if is_queryable_ddc_candidate(&current, in_display) {
        candidates.push(current);
    }
    candidates
}

/// Port of `detectDdcDisplays`: runs `ddcutil detect`, parses it via `parse_ddc_detect_output`,
/// then (matching the C++'s inline `flushCurrent` behavior exactly) live-queries each resulting
/// candidate's current brightness and drops any candidate whose query fails or reports a
/// non-positive max — a display that doesn't answer `getvcp` is not returned at all. Returns
/// `(candidates, detect_output)`, the latter standing in for the C++'s `std::string* detailOut`
/// out-param (see `query_ddc_brightness`'s doc comment for the same reasoning).
pub fn detect_ddc_displays(
    timeout: Duration,
    ignore_mmids: &[String],
    cancel: &Arc<AtomicBool>,
) -> (Vec<DdcCandidate>, String) {
    let args = ddc_detect_args(ignore_mmids);
    let detect_result = run_command_capture(&args, timeout, cancel);

    if detect_result.cancelled {
        return (Vec::new(), detect_result.output);
    }
    if !detect_result.launched {
        LOG.warn(format_args!("ddcutil detect could not be launched"));
        return (Vec::new(), detect_result.output);
    }
    if detect_result.timed_out {
        LOG.warn(format_args!(
            "ddcutil detect timed out after {}ms",
            timeout.as_millis()
        ));
        return (Vec::new(), detect_result.output);
    }
    if detect_result.exit_code != 0 {
        LOG.warn(format_args!(
            "ddcutil detect failed with exit code {}: {}",
            detect_result.exit_code,
            c_trim(&detect_result.output)
        ));
        return (Vec::new(), detect_result.output);
    }

    let mut candidates = Vec::new();
    for mut candidate in parse_ddc_detect_output(&detect_result.output) {
        if cancel.load(Ordering::Relaxed) {
            return (Vec::new(), detect_result.output);
        }
        let bus = candidate.bus;
        let (brightness, detail) = query_ddc_brightness(bus, DDC_QUERY_TIMEOUT, cancel);
        match brightness {
            Some((current_raw, max_raw)) => {
                candidate.current_raw = current_raw;
                candidate.max_raw = max_raw;
            }
            None => {
                if !cancel.load(Ordering::Relaxed) {
                    LOG.warn(format_args!(
                        "ddcutil: skipping bus {bus} because brightness query failed: {}",
                        c_trim(&detail)
                    ));
                }
                continue;
            }
        }
        if candidate.current_raw >= 0 && candidate.max_raw > 0 {
            candidates.push(candidate);
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return (Vec::new(), detect_result.output);
    }
    LOG.info(format_args!(
        "ddcutil detect parsed {} candidate display(s)",
        candidates.len()
    ));
    (candidates, detect_result.output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_trailing_integer_extracts_the_final_digit_run() {
        assert_eq!(parse_trailing_integer("bus 7"), Some(7));
        assert_eq!(parse_trailing_integer("Current value = 42, "), Some(42));
        assert_eq!(parse_trailing_integer("no digits here"), None);
        assert_eq!(parse_trailing_integer(""), None);
        assert_eq!(
            parse_trailing_integer("-5"),
            Some(5),
            "a leading sign is not part of the digit run"
        );
    }

    #[test]
    fn parse_i2c_bus_prefers_the_dev_i2c_path() {
        assert_eq!(parse_i2c_bus("   I2C bus:  /dev/i2c-11"), Some(11));
        assert_eq!(
            parse_i2c_bus("   I2C bus:  7"),
            Some(7),
            "without a /dev/i2c- path, the trailing integer of the whole line is used"
        );
        assert_eq!(parse_i2c_bus("no bus info"), None);
    }

    #[test]
    fn normalize_connector_name_strips_card_prefix_up_to_first_dash() {
        assert_eq!(normalize_connector_name("  card1-DP-1  "), "DP-1");
        assert_eq!(normalize_connector_name("card-eDP-1"), "eDP-1");
        assert_eq!(
            normalize_connector_name("card"),
            "card",
            "no dash after the card prefix leaves it unchanged"
        );
        assert_eq!(
            normalize_connector_name("DP-2"),
            "DP-2",
            "a name that doesn't start with card is untouched (besides trimming)"
        );
    }

    #[test]
    fn parse_ddc_vcp_brightness_finds_the_first_qualifying_line() {
        let output = "VCP code 0x10 (Brightness): current value = 45, max value = 100\n";
        assert_eq!(parse_ddc_vcp_brightness(output), Some((45, 100)));
    }

    #[test]
    fn parse_ddc_vcp_brightness_is_case_insensitive() {
        let output = "Current Value =    60, Max Value =     100\n";
        assert_eq!(parse_ddc_vcp_brightness(output), Some((60, 100)));
    }

    #[test]
    fn parse_ddc_vcp_brightness_rejects_out_of_order_or_missing_markers() {
        assert_eq!(
            parse_ddc_vcp_brightness("max value = 100, current value = 45\n"),
            None,
            "max value must appear after current value on the line"
        );
        assert_eq!(parse_ddc_vcp_brightness("current value = 45\n"), None);
        assert_eq!(parse_ddc_vcp_brightness(""), None);
    }

    #[test]
    fn parse_ddc_vcp_brightness_rejects_a_non_positive_max() {
        assert_eq!(
            parse_ddc_vcp_brightness("current value = 45, max value = 0\n"),
            None,
            "a max of 0 should be rejected, not treated as a valid (45, 0) reading"
        );
    }

    const DETECT_OUTPUT: &str = "Invalid display\n\
        \n\
        Display 1\n   \
        I2C bus:  /dev/i2c-11\n   \
        DRM connector:   card1-DP-1\n   \
        Monitor:      Dell U2720Q\n\
        \n\
        Display 2\n   \
        I2C bus:  /dev/i2c-4\n   \
        DRM_connector:   card1-HDMI-A-1\n   \
        Model:      LG HDR 4K\n\
        \n\
        Display not found\n   \
        I2C bus:  /dev/i2c-99\n\
        \n\
        Display 3\n   \
        DRM connector:   card1-DP-2\n";

    #[test]
    fn parse_ddc_detect_output_extracts_only_queryable_candidates() {
        let candidates = parse_ddc_detect_output(DETECT_OUTPUT);

        assert_eq!(
            candidates.len(),
            2,
            "Display 3 (no I2C bus) and the \"Display not found\" block should be excluded"
        );

        assert_eq!(candidates[0].bus, 11);
        assert_eq!(candidates[0].connector_name, "DP-1");
        assert_eq!(candidates[0].label, "Dell U2720Q");
        assert_eq!(
            candidates[0].current_raw, -1,
            "the pure parser leaves current_raw at its default; the live query fills it in"
        );

        assert_eq!(candidates[1].bus, 4);
        assert_eq!(candidates[1].connector_name, "HDMI-A-1");
        assert_eq!(
            candidates[1].label, "LG HDR 4K",
            "Model: should be used when Monitor: is absent"
        );
    }

    #[test]
    fn parse_ddc_detect_output_prefers_monitor_over_model_when_both_present() {
        let output = "Display 1\n   I2C bus: /dev/i2c-1\n   DRM connector: card1-DP-1\n   \
                       Model: Generic\n   Monitor: Real Name\n";
        let candidates = parse_ddc_detect_output(output);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].label, "Real Name",
            "Monitor: always wins since it's applied after Model: in the input, but Model: \
             itself only ever applies when label is still empty"
        );
    }

    #[test]
    fn parse_ddc_detect_output_on_empty_or_no_display_output_is_empty() {
        assert!(parse_ddc_detect_output("").is_empty());
        assert!(parse_ddc_detect_output("No displays found\n").is_empty());
    }

    #[test]
    fn ddc_detect_args_places_ignore_mmid_pairs_before_detect() {
        let args = ddc_detect_args(&["ABC123".to_string(), "XYZ789".to_string()]);
        assert_eq!(
            args,
            vec![
                "ddcutil",
                "--noconfig",
                "--ignore-mmid",
                "ABC123",
                "--ignore-mmid",
                "XYZ789",
                "detect"
            ]
        );
    }

    #[test]
    fn ddc_detect_args_with_no_ignored_mmids() {
        assert_eq!(
            ddc_detect_args(&[]),
            vec!["ddcutil", "--noconfig", "detect"]
        );
    }

    #[test]
    fn ddc_base_args_builds_the_expected_argv() {
        assert_eq!(
            ddc_base_args(7),
            vec![
                "ddcutil",
                "--noconfig",
                "--enable-dynamic-sleep",
                "--sleep-multiplier",
                "0.1",
                "--bus",
                "7"
            ]
        );
    }

    #[test]
    fn run_command_capture_short_circuits_on_a_pre_cancelled_flag() {
        let cancel = Arc::new(AtomicBool::new(true));
        let result = run_command_capture(&["true".to_string()], Duration::from_secs(1), &cancel);
        assert_eq!(
            result,
            CommandResult {
                cancelled: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn run_command_capture_combines_stdout_and_stderr_with_a_newline() {
        let cancel = Arc::new(AtomicBool::new(false));
        let args = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "printf out; printf err 1>&2".to_string(),
        ];
        let result = run_command_capture(&args, Duration::from_secs(5), &cancel);
        assert!(result.launched);
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.output, "out\nerr");
    }

    #[test]
    fn run_command_capture_reports_a_failed_launch() {
        let cancel = Arc::new(AtomicBool::new(false));
        let args = vec!["/does/not/exist/at/all".to_string()];
        let result = run_command_capture(&args, Duration::from_secs(5), &cancel);
        assert!(!result.launched);
        assert_eq!(result.exit_code, -1);
    }

    #[test]
    fn query_ddc_brightness_reports_none_when_the_command_cannot_launch() {
        // A bogus bus makes no difference here: the real assertion is that a launch failure
        // (this dev host has no `ddcutil` at all) reports None rather than a bogus reading.
        let cancel = Arc::new(AtomicBool::new(false));
        let (brightness, _detail) = query_ddc_brightness(1, Duration::from_secs(1), &cancel);
        assert!(brightness.is_none());
    }

    #[test]
    fn detect_ddc_displays_reports_no_candidates_when_ddcutil_is_unavailable() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (candidates, _detail) = detect_ddc_displays(Duration::from_secs(1), &[], &cancel);
        assert!(
            candidates.is_empty(),
            "no ddcutil on this host means no candidates, not a panic"
        );
    }
}
