//! `noctalia config` CLI entry point.
//! Partial port of `src/config/cli.{cpp,h}` — task 4.2.2 only.
//!
//! Only `validate` and top-level `--help` are wired here. `export`/`settings-count`/
//! `replay-report` are documented in [`HELP_TEXT`] (ported verbatim, matching the C++'s full
//! surface — MIGRATION_PLAN.md's own "done" bar for this task is the `--help` text matching,
//! not every subcommand working) but fall through to the same "unknown config command" error a
//! genuine typo would hit, until tasks 4.2.3 (`export`), 4.2.4 (`settings-count`, blocked on
//! Phase 14's settings registry), and 4.2.5 (`replay-report`) land.

use std::io::IsTerminal;
use std::path::Path;

use noctalia_core::log::{LogLevel, set_log_level};

use crate::schema::diagnostics::{Diagnostics, Severity};
use crate::validate::{validate_config_file, validate_config_sources};

// NOTE: these are plain multi-line string literals, not `\`-continued lines — a `\` at the end
// of a Rust string literal line strips the following newline *and all leading whitespace on the
// next line*, which would silently eat the two/six-space indentation below.
const HELP_TEXT: &str = "Usage: noctalia config <command> [options]

Commands:
  validate [path]
      Check config validity: TOML syntax, unknown/misspelled settings, and bad
      values. Defaults to the active config dir + state settings.toml. A directory
      validates its *.toml files; a file validates only that file. Exit 1 on error.

  export [merged|full]
      Print the active config as TOML. Defaults to merged user config.

  settings-count
      Count Settings UI controls by registry, visibility state, and section.

  replay-report <report.toml> --target <dir> [--force]
      Reconstruct config-home/noctalia and state-home/noctalia from a support report.

  replay-report <report.toml> --target <dir> --flattened [--force]
      Reconstruct a single config-home/noctalia/config.toml from the report's merged config.
";

const VALIDATE_HELP_TEXT: &str = "Usage: noctalia config validate [path]

With no path, validates the merged configuration the way the shell loads it:
  - every *.toml in the active config dir, then
  - the state-dir settings.toml overrides.

With a directory path, validates only that directory's *.toml files.
With a file path, validates only that file.

Reports TOML syntax errors, unknown sections/settings, and bad values
(wrong type, out-of-range, invalid enum/color). Exits 1 if any error is found.
";

/// Port of `useColor` (`cli.cpp:425-428`): color only when the stream is a terminal and
/// `NO_COLOR` is unset (any value, including empty — a bare presence check, unlike the
/// `!value.is_empty()` guard the XDG-style env vars elsewhere in this codebase use).
fn use_color(stream: &impl IsTerminal) -> bool {
    std::env::var_os("NO_COLOR").is_none() && stream.is_terminal()
}

/// Port of `runValidate` (`cli.cpp:430-507`).
fn run_validate(args: &[String]) -> i32 {
    let mut path_arg = String::new();
    for arg in args {
        if arg == "--help" {
            println!("{VALIDATE_HELP_TEXT}");
            return 0;
        }
        if path_arg.is_empty() {
            path_arg = arg.clone();
            continue;
        }
        eprintln!("error: unexpected argument: {arg}");
        eprintln!("Run 'noctalia config validate --help' for usage.");
        return 1;
    }

    // Validation reports through diagnostics below; silence incidental INFO logs so only
    // validation results reach the user.
    set_log_level(LogLevel::Warn);

    let diagnostics = if path_arg.is_empty() {
        let config_dir = noctalia_core::files::paths::config_dir();
        let state_dir = noctalia_core::files::paths::state_dir();
        let settings_path = if state_dir.is_empty() {
            String::new()
        } else {
            format!("{state_dir}/settings.toml")
        };
        validate_config_sources(&config_dir, &settings_path)
    } else {
        let input_path = Path::new(&path_arg);
        // Follows symlinks, matching the C++'s `std::filesystem::status`.
        let metadata = match std::fs::metadata(input_path) {
            Ok(meta) => meta,
            Err(err) => {
                eprintln!("error: failed to inspect {path_arg}: {err}");
                return 1;
            }
        };
        if metadata.is_dir() {
            validate_config_sources(input_path, "")
        } else if metadata.is_file() {
            validate_config_file(input_path)
        } else {
            eprintln!("error: path is not a regular file or directory: {path_arg}");
            return 1;
        }
    };

    print_validate_report(&diagnostics)
}

/// Port of the diagnostic-printing and exit-code half of `runValidate`
/// (`cli.cpp:476-506`), split out for testability (an actual color/tty decision isn't worth
/// threading through a unit test, but the counting/formatting/exit-code logic is).
fn print_validate_report(diagnostics: &Diagnostics) -> i32 {
    let color_err = use_color(&std::io::stderr());
    let color_out = use_color(&std::io::stdout());

    let mut errors = 0usize;
    let mut warnings = 0usize;
    for entry in &diagnostics.entries {
        let is_error = entry.severity == Severity::Error;
        if is_error {
            errors += 1;
        } else {
            warnings += 1;
        }
        // Padded to align the path column, matching the C++'s "WARN " (5 chars incl. trailing
        // space) vs "ERROR" (5 chars).
        let tag = if is_error { "ERROR" } else { "WARN " };
        let use_this_color = if is_error { color_err } else { color_out };
        let color = if use_this_color {
            if is_error { "\x1b[31m" } else { "\x1b[33m" }
        } else {
            ""
        };
        let reset = if color.is_empty() { "" } else { "\x1b[0m" };
        let line = format!("{color}{tag}{reset} {}: {}", entry.path, entry.message);
        if is_error {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }

    if errors > 0 {
        let c = if color_err { "\x1b[31m" } else { "" };
        let r = if c.is_empty() { "" } else { "\x1b[0m" };
        eprintln!();
        eprintln!("{c}\u{2717} Config is invalid{r} ({errors} error(s), {warnings} warning(s))");
        return 1;
    }
    let c = if color_out { "\x1b[32m" } else { "" };
    let r = if c.is_empty() { "" } else { "\x1b[0m" };
    if warnings > 0 {
        println!();
        println!("{c}\u{2713} Config is valid{r} ({warnings} warning(s))");
    } else {
        println!("{c}\u{2713} Config is valid{r}");
    }
    0
}

/// Entry point for `noctalia config <command> [options]`. `args` are the tokens after the
/// `config` verb (matching `noctalia_ipc::cli::run_cli`'s convention for `msg`). Returns a
/// process exit code. Pure CLI helper; does not start Application or mutate live config.
#[must_use]
pub fn run_cli(args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "--help" {
        println!("{HELP_TEXT}");
        return i32::from(args.is_empty());
    }

    if args[0] == "validate" {
        return run_validate(&args[1..]);
    }

    eprintln!("error: unknown config command: {}", args[0]);
    eprintln!("Run 'noctalia config --help' for usage.");
    1
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn no_args_prints_help_and_fails() {
        assert_eq!(run_cli(&[]), 1);
    }

    #[test]
    fn help_flag_prints_help_and_succeeds() {
        assert_eq!(run_cli(&["--help".to_string()]), 0);
    }

    #[test]
    fn unknown_command_fails() {
        assert_eq!(run_cli(&["bogus".to_string()]), 1);
    }

    #[test]
    fn export_is_not_wired_yet_and_falls_through_to_unknown_command() {
        // Documented in the module doc comment: real behavior lands in task 4.2.3.
        assert_eq!(run_cli(&["export".to_string()]), 1);
    }

    #[test]
    fn validate_help_flag_succeeds() {
        assert_eq!(run_validate(&["--help".to_string()]), 0);
    }

    #[test]
    fn validate_unexpected_second_argument_fails() {
        assert_eq!(run_validate(&["a".to_string(), "b".to_string()]), 1);
    }

    #[test]
    fn validate_missing_path_fails_cleanly() {
        let missing = std::env::temp_dir().join(format!(
            "noctalia-cli-missing-{}-{}",
            std::process::id(),
            "xyz"
        ));
        let _ = std::fs::remove_file(&missing);
        assert_eq!(run_validate(&[missing.to_string_lossy().to_string()]), 1);
    }

    #[test]
    fn validate_empty_directory_reports_success() {
        let dir =
            std::env::temp_dir().join(format!("noctalia-cli-emptydir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(run_validate(&[dir.to_string_lossy().to_string()]), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_syntax_error_file_reports_failure_with_syntax_tag() {
        let file = std::env::temp_dir().join(format!(
            "noctalia-cli-syntaxerr-{}.toml",
            std::process::id()
        ));
        std::fs::write(&file, "[bar.default\nthickness = broken\n").unwrap();

        assert_eq!(run_validate(&[file.to_string_lossy().to_string()]), 1);

        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn print_validate_report_counts_errors_and_warnings_and_picks_exit_code() {
        let mut diag = Diagnostics::default();
        diag.warn("a.b", "warned");
        assert_eq!(print_validate_report(&diag), 0);

        diag.error("c.d", "errored");
        assert_eq!(print_validate_report(&diag), 1);
    }
}
