//! Subprocess-level test of the compiled `noctalia-shell` binary's CLI dispatch
//! (`crates/noctalia-shell/src/main.rs`), added per task 4.2.2's pre-done review: the unit
//! tests in `noctalia-config::cli` exercise `run_validate`/`run_cli` directly, which never
//! touches `main.rs`'s own `clap` parsing/dispatch (`Command::Config { args } =>
//! noctalia_config::cli::run_cli(&args)`) — this covers that wiring specifically, against the
//! real fixtures `tests/config_validate_cli_test.sh` uses.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

fn noctalia_shell() -> Command {
    Command::new(env!("CARGO_BIN_EXE_noctalia-shell"))
}

#[test]
fn config_validate_empty_dir_succeeds() {
    let fixture = repo_root().join("tests/config_validate/generated-config");
    let output = noctalia_shell()
        .args(["config", "validate", fixture.to_str().unwrap()])
        .output()
        .expect("run noctalia-shell");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Config is valid"), "stdout: {stdout}");
    assert!(!stdout.contains("WARN"), "stdout: {stdout}");
}

#[test]
fn config_validate_syntax_error_fails_with_expected_message() {
    let fixture = repo_root().join("tests/config_validate/syntax-error.toml");
    let output = noctalia_shell()
        .args(["config", "validate", fixture.to_str().unwrap()])
        .output()
        .expect("run noctalia-shell");

    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!(
        "ERROR syntax: {}:",
        fixture.to_str().expect("fixture path is valid UTF-8")
    );
    assert!(combined.contains(&expected), "output: {combined}");
}

#[test]
fn config_help_lists_documented_subcommands() {
    let output = noctalia_shell()
        .args(["config", "--help"])
        .output()
        .expect("run noctalia-shell");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in ["validate", "export", "settings-count", "replay-report"] {
        assert!(
            stdout.contains(expected),
            "stdout missing {expected}: {stdout}"
        );
    }
}

#[test]
fn config_export_falls_through_to_unknown_command_until_task_4_2_3() {
    let output = noctalia_shell()
        .args(["config", "export"])
        .output()
        .expect("run noctalia-shell");

    assert!(!output.status.success());
}
