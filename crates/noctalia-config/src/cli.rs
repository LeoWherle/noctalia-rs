//! `noctalia config` CLI entry point.
//! Partial port of `src/config/cli.{cpp,h}` — tasks 4.2.2 (`validate`), 4.2.3
//! (`export merged`), and 4.2.5 (`replay-report`).
//!
//! `validate`, `export merged`, and `replay-report` are wired. `export full` falls through to
//! its own explicit "not implemented yet" error (see [`run_export`] — it needs a
//! `parseConfigTable`/`makeDefaultConfig` equivalent whose launcher-provider step is blocked on
//! task 14.1, not just more plumbing; MIGRATION_PLAN.md task 4.2.3 has the full accounting).
//! `settings-count` is documented in [`HELP_TEXT`] (ported verbatim, matching the C++'s full
//! surface) but falls through to the same "unknown config command" error a genuine typo would
//! hit, until task 4.2.4 (blocked on Phase 14's settings registry) lands.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use noctalia_core::log::{LogLevel, set_log_level};

use crate::schema::diagnostics::{Diagnostics, Severity};
use crate::service::build_merged_user_config_from_sources;
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

const EXPORT_HELP_TEXT: &str = "Usage: noctalia config export [merged|full]

Prints TOML to stdout from the same config stack used by the shell:
  - every *.toml in the active config dir, then
  - the state-dir settings.toml overrides.

Modes:
  merged  Export merged user config only (default)
  full    Export full effective config, including built-in defaults
";

const REPLAY_HELP_TEXT: &str =
    "Usage: noctalia config replay-report <report.toml> --target <dir> [--flattened] [--force]

Options:
  --target <dir>  Directory where replay files are written
  --flattened     Write only merged_config.content as config.toml
  --force         Remove an existing target directory before writing
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

/// Port of `runExport` (`cli.cpp:509-551`). `full` mode is not implemented yet — see the module
/// doc comment for why.
fn run_export(args: &[String]) -> i32 {
    let mut mode = "merged".to_string();
    let mut mode_set = false;
    for arg in args {
        if arg == "--help" {
            println!("{EXPORT_HELP_TEXT}");
            return 0;
        }
        if !mode_set {
            mode = arg.clone();
            mode_set = true;
            continue;
        }
        eprintln!("error: unexpected argument: {arg}");
        eprintln!("Run 'noctalia config export --help' for usage.");
        return 1;
    }

    if mode == "full" {
        eprintln!(
            "error: `config export full` is not implemented yet (see MIGRATION_PLAN.md task 4.2.3)"
        );
        return 1;
    }
    if mode != "merged" {
        eprintln!("error: expected merged or full");
        return 1;
    }

    let config_dir = noctalia_core::files::paths::config_dir();
    let state_dir = noctalia_core::files::paths::state_dir();
    let settings_path = if state_dir.is_empty() {
        String::new()
    } else {
        format!("{state_dir}/settings.toml")
    };

    match build_merged_user_config_from_sources(Path::new(&config_dir), Path::new(&settings_path)) {
        Ok(content) => {
            print!("{content}");
            0
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

/// Port of `StringUtils::shellQuote` (`string_utils.h:491-502`), pulled forward minimally —
/// same pattern as task 1.6.5's `generate_uuid_v4` (`noctalia_core::process::systemd`) — since
/// this is the only function from `string_utils.h` needed so far.
fn shell_quote(text: &str) -> String {
    let mut result = String::from("'");
    for ch in text.chars() {
        if ch == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(ch);
        }
    }
    result.push('\'');
    result
}

/// Port of `ReplayOptions` (`cli.cpp:88-93`).
#[derive(Default)]
struct ReplayOptions {
    report_path: PathBuf,
    target_dir: PathBuf,
    flattened: bool,
    force: bool,
}

/// Port of `ReplayOptionsParse` (`cli.cpp:95-98`).
struct ReplayOptionsParse {
    options: ReplayOptions,
    help_requested: bool,
}

/// Port of `parseReplayOptions` (`cli.cpp:256-294`).
fn parse_replay_options(args: &[String]) -> Result<ReplayOptionsParse, String> {
    let mut options = ReplayOptions::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--help" {
            println!("{REPLAY_HELP_TEXT}");
            return Ok(ReplayOptionsParse {
                options,
                help_requested: true,
            });
        }
        if arg == "--target" {
            let Some(next) = args.get(i + 1) else {
                return Err("--target requires a directory".to_string());
            };
            options.target_dir = PathBuf::from(next);
            i += 2;
            continue;
        }
        if arg == "--flattened" {
            options.flattened = true;
            i += 1;
            continue;
        }
        if arg == "--force" {
            options.force = true;
            i += 1;
            continue;
        }
        if options.report_path.as_os_str().is_empty() {
            options.report_path = PathBuf::from(arg);
            i += 1;
            continue;
        }
        return Err(format!("unknown argument: {arg}"));
    }

    if options.report_path.as_os_str().is_empty() {
        return Err("missing report path".to_string());
    }
    if options.target_dir.as_os_str().is_empty() {
        return Err("missing --target <dir>".to_string());
    }
    Ok(ReplayOptionsParse {
        options,
        help_requested: false,
    })
}

/// Port of `writeTextFile` (`cli.cpp:203-219`).
fn write_text_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, content)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Lexical `.`/`..` collapse matching `std::filesystem::path::lexically_normal` closely enough
/// for the relative, `..`-free paths this module ever normalizes (target-dir resolution, and
/// `safe_relative_path`'s already-`..`-rejected paths).
fn lexically_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Port of `std::filesystem::absolute(...).lexically_normal()` as applied to `options.targetDir`
/// (`cli.cpp:305`).
fn resolve_target(target_dir: &Path) -> Result<PathBuf, String> {
    let joined = if target_dir.is_absolute() {
        target_dir.to_path_buf()
    } else {
        let cwd = std::env::current_dir()
            .map_err(|err| format!("failed to resolve current directory: {err}"))?;
        cwd.join(target_dir)
    };
    Ok(lexically_normal(&joined))
}

/// Port of `safeRelativePath` (`cli.cpp:221-242`).
fn safe_relative_path(table: &toml::Table, fallback: &str) -> Option<PathBuf> {
    let raw = match table.get("relative_path").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => fallback.to_string(),
    };
    if raw.is_empty() {
        return None;
    }

    let path = PathBuf::from(&raw);
    if path.is_absolute() {
        return None;
    }
    if path
        .components()
        .any(|part| part == std::path::Component::ParentDir)
    {
        return None;
    }
    Some(lexically_normal(&path))
}

/// Port of `prepareTarget` (`cli.cpp:244-254`).
fn prepare_target(target: &Path, force: bool) -> Result<(), String> {
    if target.exists() && !force {
        return Err(format!(
            "target already exists; pass --force to replace it: {}",
            target.display()
        ));
    }
    std::fs::create_dir_all(target)
        .map_err(|err| format!("failed to create target {}: {err}", target.display()))
}

/// Removes `path` if present; a no-op (not an error) if it doesn't exist, matching the C++'s
/// `std::filesystem::remove_all` with an `error_code` (never throws on a missing path).
fn remove_dir_all_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(path)
        .map_err(|err| format!("failed to remove {}: {err}", path.display()))
}

/// Port of `replayReport` (`cli.cpp:296-421`).
fn replay_report(options: &ReplayOptions) -> i32 {
    let report: toml::Table = match std::fs::read_to_string(&options.report_path)
        .map_err(|err| err.to_string())
        .and_then(|content| {
            content
                .parse::<toml::Table>()
                .map_err(|err| err.to_string())
        }) {
        Ok(table) => table,
        Err(err) => {
            eprintln!("error: failed to parse report: {err}");
            return 1;
        }
    };

    let target = match resolve_target(&options.target_dir) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("error: {err}");
            return 1;
        }
    };

    if let Err(err) = prepare_target(&target, options.force) {
        eprintln!("error: {err}");
        return 1;
    }

    let config_home = target.join("config-home");
    let state_home = target.join("state-home");
    let config_dir = config_home.join("noctalia");
    let state_dir = state_home.join("noctalia");

    if options.force {
        if let Err(err) = remove_dir_all_if_exists(&config_home) {
            eprintln!("error: {err}");
            return 1;
        }
        if let Err(err) = remove_dir_all_if_exists(&state_home) {
            eprintln!("error: {err}");
            return 1;
        }
    }

    if options.flattened {
        let Some(merged) = report
            .get("merged_config")
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_str())
        else {
            eprintln!("error: report has no [merged_config].content");
            return 1;
        };
        if let Err(err) = write_text_file(&config_dir.join("config.toml"), merged) {
            eprintln!("error: {err}");
            return 1;
        }
        if let Err(err) = std::fs::create_dir_all(&state_dir) {
            eprintln!("error: failed to create {}: {err}", state_dir.display());
            return 1;
        }
    } else {
        if let Some(sources) = report.get("config_sources").and_then(|v| v.as_array()) {
            let mut fallback_index = 0usize;
            for source_node in sources {
                let Some(source) = source_node.as_table() else {
                    continue;
                };
                let Some(content) = source.get("content").and_then(|v| v.as_str()) else {
                    continue;
                };

                let fallback = format!("config_{fallback_index}.toml");
                fallback_index += 1;
                let Some(relative) = safe_relative_path(source, &fallback) else {
                    eprintln!("error: report contains an unsafe config source path");
                    return 1;
                };
                if let Err(err) = write_text_file(&config_dir.join(&relative), content) {
                    eprintln!("error: {err}");
                    return 1;
                }
            }
        }

        let state = report.get("state_settings").and_then(|v| v.as_table());
        let mut state_exists = state.is_some();
        if let Some(state_table) = state
            && let Some(exists) = state_table.get("exists").and_then(|v| v.as_bool())
        {
            state_exists = exists;
        }
        if let Some(state_table) = state.filter(|_| state_exists) {
            let content = state_table
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Err(err) = write_text_file(&state_dir.join("settings.toml"), content) {
                eprintln!("error: {err}");
                return 1;
            }
        } else if let Err(err) = std::fs::create_dir_all(&state_dir) {
            eprintln!("error: failed to create {}: {err}", state_dir.display());
            return 1;
        }

        let app_state = report.get("app_state").and_then(|v| v.as_table());
        let mut app_state_exists = app_state.is_some();
        if let Some(app_state_table) = app_state
            && let Some(exists) = app_state_table.get("exists").and_then(|v| v.as_bool())
        {
            app_state_exists = exists;
        }
        if let Some(app_state_table) = app_state.filter(|_| app_state_exists) {
            let content = app_state_table
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Err(err) = write_text_file(&state_dir.join("state.toml"), content) {
                eprintln!("error: {err}");
                return 1;
            }
        }
    }

    println!("Replayed support report into {}", target.display());
    println!();
    println!("Config home: {}", config_home.display());
    println!("State home:  {}", state_home.display());
    println!();
    println!("Run with:");
    let argv0 = std::env::args().next().unwrap_or_default();
    println!(
        "  NOCTALIA_CONFIG_HOME={} NOCTALIA_STATE_HOME={} {}",
        shell_quote(&config_home.to_string_lossy()),
        shell_quote(&state_home.to_string_lossy()),
        shell_quote(&argv0)
    );
    0
}

/// Port of `runCli`'s `replay-report` dispatch branch (`cli.cpp:573-584`).
fn run_replay_report(args: &[String]) -> i32 {
    match parse_replay_options(args) {
        Ok(parsed) => {
            if parsed.help_requested {
                0
            } else {
                replay_report(&parsed.options)
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            eprintln!("Run 'noctalia config replay-report --help' for usage.");
            1
        }
    }
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

    if args[0] == "export" {
        return run_export(&args[1..]);
    }

    if args[0] == "replay-report" {
        return run_replay_report(&args[1..]);
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
    fn run_cli_dispatches_export_to_run_export() {
        // `--help` is safe to exercise through the full `run_cli` dispatch (unlike a bare
        // `export`, which would fall to `run_export`'s default-path branch and touch this
        // process's real `$HOME`/`$XDG_CONFIG_HOME` — not exercised by any unit test here, same
        // as `run_validate`'s equivalent default-path branch; see the manual check in
        // PROGRESS.log instead).
        assert_eq!(run_cli(&["export".to_string(), "--help".to_string()]), 0);
    }

    #[test]
    fn export_help_flag_succeeds() {
        assert_eq!(run_export(&["--help".to_string()]), 0);
    }

    #[test]
    fn export_unexpected_second_argument_fails() {
        assert_eq!(run_export(&["merged".to_string(), "extra".to_string()]), 1);
    }

    #[test]
    fn export_full_mode_is_not_implemented_yet() {
        assert_eq!(run_export(&["full".to_string()]), 1);
    }

    #[test]
    fn export_invalid_mode_fails() {
        assert_eq!(run_export(&["bogus-mode".to_string()]), 1);
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

    #[test]
    fn shell_quote_wraps_plain_text_in_single_quotes() {
        assert_eq!(shell_quote("hello"), "'hello'");
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn safe_relative_path_uses_fallback_when_relative_path_absent() {
        let table = toml::Table::new();
        assert_eq!(
            safe_relative_path(&table, "config_0.toml"),
            Some(PathBuf::from("config_0.toml"))
        );
    }

    #[test]
    fn safe_relative_path_uses_explicit_relative_path() {
        let table: toml::Table = "relative_path = \"nested/00-bar.toml\"".parse().unwrap();
        assert_eq!(
            safe_relative_path(&table, "fallback.toml"),
            Some(PathBuf::from("nested/00-bar.toml"))
        );
    }

    #[test]
    fn safe_relative_path_rejects_absolute_path() {
        let table: toml::Table = "relative_path = \"/etc/passwd\"".parse().unwrap();
        assert_eq!(safe_relative_path(&table, "fallback.toml"), None);
    }

    #[test]
    fn safe_relative_path_rejects_parent_dir_traversal() {
        let table: toml::Table = "relative_path = \"../../etc/passwd\"".parse().unwrap();
        assert_eq!(safe_relative_path(&table, "fallback.toml"), None);
    }

    #[test]
    fn safe_relative_path_normalizes_current_dir_components() {
        let table: toml::Table = "relative_path = \"./a/./b.toml\"".parse().unwrap();
        assert_eq!(
            safe_relative_path(&table, "fallback.toml"),
            Some(PathBuf::from("a/b.toml"))
        );
    }

    fn replay_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-cli-replay-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_replay_report_help_flag_succeeds() {
        assert_eq!(run_replay_report(&["--help".to_string()]), 0);
    }

    #[test]
    fn run_replay_report_missing_report_path_fails() {
        assert_eq!(
            run_replay_report(&["--target".to_string(), "/tmp/x".to_string()]),
            1
        );
    }

    #[test]
    fn run_replay_report_missing_target_fails() {
        assert_eq!(run_replay_report(&["report.toml".to_string()]), 1);
    }

    #[test]
    fn run_replay_report_target_flag_missing_value_fails() {
        assert_eq!(
            run_replay_report(&["report.toml".to_string(), "--target".to_string()]),
            1
        );
    }

    #[test]
    fn replay_report_full_reconstructs_sources_and_state() {
        let dir = replay_temp_dir("full");
        let report_path = dir.join("report.toml");
        std::fs::write(
            &report_path,
            r#"
[[config_sources]]
relative_path = "00-bar.toml"
content = "[bar.default]\nthickness = 42\n"

[state_settings]
exists = true
content = "[bar.default]\nthickness = 55\n"

[app_state]
exists = true
content = "last_workspace = 3\n"
"#,
        )
        .unwrap();

        let target = dir.join("target");
        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 0);

        let config_file = target.join("config-home/noctalia/00-bar.toml");
        assert_eq!(
            std::fs::read_to_string(&config_file).unwrap(),
            "[bar.default]\nthickness = 42\n"
        );
        let state_file = target.join("state-home/noctalia/settings.toml");
        assert_eq!(
            std::fs::read_to_string(&state_file).unwrap(),
            "[bar.default]\nthickness = 55\n"
        );
        let app_state_file = target.join("state-home/noctalia/state.toml");
        assert_eq!(
            std::fs::read_to_string(&app_state_file).unwrap(),
            "last_workspace = 3\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_flattened_writes_single_merged_config() {
        let dir = replay_temp_dir("flattened");
        let report_path = dir.join("report.toml");
        std::fs::write(
            &report_path,
            r#"
[merged_config]
content = "[bar.default]\nthickness = 60\n"
"#,
        )
        .unwrap();

        let target = dir.join("target");
        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
            "--flattened".to_string(),
        ]);
        assert_eq!(exit, 0);

        let config_file = target.join("config-home/noctalia/config.toml");
        assert_eq!(
            std::fs::read_to_string(&config_file).unwrap(),
            "[bar.default]\nthickness = 60\n"
        );
        assert!(target.join("state-home/noctalia").is_dir());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_state_settings_exists_false_creates_dir_without_writing_file() {
        let dir = replay_temp_dir("state-exists-false");
        let report_path = dir.join("report.toml");
        std::fs::write(
            &report_path,
            r#"
[state_settings]
exists = false
content = "should_not_be_written = true\n"
"#,
        )
        .unwrap();

        let target = dir.join("target");
        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 0);

        assert!(target.join("state-home/noctalia").is_dir());
        assert!(!target.join("state-home/noctalia/settings.toml").exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_config_source_without_relative_path_uses_fallback_name() {
        let dir = replay_temp_dir("fallback-name");
        let report_path = dir.join("report.toml");
        std::fs::write(
            &report_path,
            r#"
[[config_sources]]
content = "first = true\n"

[[config_sources]]
content = "second = true\n"
"#,
        )
        .unwrap();

        let target = dir.join("target");
        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 0);

        assert_eq!(
            std::fs::read_to_string(target.join("config-home/noctalia/config_0.toml")).unwrap(),
            "first = true\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("config-home/noctalia/config_1.toml")).unwrap(),
            "second = true\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_flattened_without_merged_config_fails() {
        let dir = replay_temp_dir("flattened-missing");
        let report_path = dir.join("report.toml");
        std::fs::write(&report_path, "").unwrap();

        let target = dir.join("target");
        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
            "--flattened".to_string(),
        ]);
        assert_eq!(exit, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_refuses_existing_target_without_force() {
        let dir = replay_temp_dir("existing-target");
        let report_path = dir.join("report.toml");
        std::fs::write(&report_path, "").unwrap();

        let target = dir.join("target");
        std::fs::create_dir_all(&target).unwrap();

        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
            "--flattened".to_string(),
        ]);
        assert_eq!(exit, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_force_overwrites_existing_target() {
        let dir = replay_temp_dir("force-target");
        let report_path = dir.join("report.toml");
        std::fs::write(&report_path, "[merged_config]\ncontent = \"a = 1\\n\"\n").unwrap();

        let target = dir.join("target");
        std::fs::create_dir_all(target.join("config-home/noctalia")).unwrap();
        std::fs::write(
            target.join("config-home/noctalia/stale.toml"),
            "stale = true\n",
        )
        .unwrap();

        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
            "--flattened".to_string(),
            "--force".to_string(),
        ]);
        assert_eq!(exit, 0);

        assert!(!target.join("config-home/noctalia/stale.toml").exists());
        assert_eq!(
            std::fs::read_to_string(target.join("config-home/noctalia/config.toml")).unwrap(),
            "a = 1\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_report_rejects_unsafe_relative_path_in_report() {
        let dir = replay_temp_dir("unsafe-path");
        let report_path = dir.join("report.toml");
        std::fs::write(
            &report_path,
            r#"
[[config_sources]]
relative_path = "../escape.toml"
content = "evil = true\n"
"#,
        )
        .unwrap();

        let target = dir.join("target");
        let exit = run_replay_report(&[
            report_path.to_string_lossy().to_string(),
            "--target".to_string(),
            target.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 1);

        let _ = std::fs::remove_dir_all(dir);
    }
}
