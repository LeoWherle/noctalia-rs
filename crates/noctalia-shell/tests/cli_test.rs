//! Subprocess-level test of the compiled `noctalia-shell` binary's CLI dispatch
//! (`crates/noctalia-shell/src/main.rs`), added per task 4.2.2's pre-done review: the unit
//! tests in `noctalia-config::cli` exercise `run_validate`/`run_cli` directly, which never
//! touches `main.rs`'s own `clap` parsing/dispatch (`Command::Config { args } =>
//! noctalia_config::cli::run_cli(&args)`) — this covers that wiring specifically, against the
//! real fixtures `tests/config_validate_cli_test.sh` uses. Extended in task 4.2.3 with
//! `config export merged` coverage.

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

/// `config export merged`/`validate` both fall back to real `$XDG_CONFIG_HOME`/
/// `$XDG_STATE_HOME` when given no explicit path, so these tests point those at an isolated
/// temp dir rather than touching whatever config this machine actually has.
fn isolated_xdg_dirs(label: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "noctalia-shell-cli-test-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let config_home = root.join("config");
    let state_home = root.join("state");
    std::fs::create_dir_all(config_home.join("noctalia")).expect("create config-home/noctalia");
    std::fs::create_dir_all(&state_home).expect("create state-home");
    (root, config_home, state_home)
}

#[test]
fn config_export_merged_prints_toml_from_an_empty_config_dir() {
    let (root, config_home, state_home) = isolated_xdg_dirs("export-merged-empty");
    let output = noctalia_shell()
        .args(["config", "export", "merged"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .expect("run noctalia-shell");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_export_merged_strips_config_version_and_keeps_bar_settings() {
    let (root, config_home, state_home) = isolated_xdg_dirs("export-merged-content");
    std::fs::write(
        config_home.join("noctalia/00-bar.toml"),
        "config_version = 3\n[bar.default]\nthickness = 42\n",
    )
    .expect("write fixture config");

    let output = noctalia_shell()
        .args(["config", "export", "merged"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .expect("run noctalia-shell");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("config_version"), "stdout: {stdout}");
    assert!(stdout.contains("thickness = 42"), "stdout: {stdout}");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_export_full_reports_not_implemented_yet() {
    let output = noctalia_shell()
        .args(["config", "export", "full"])
        .output()
        .expect("run noctalia-shell");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not implemented yet"), "stderr: {stderr}");
}
