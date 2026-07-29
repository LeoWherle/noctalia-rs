//! Semantic validation of an already-parsed config table.
//! Partial port of `src/config/config_validate.{cpp,h}` — task 2.4.1 only.
//!
//! `config_validate.cpp` pulls in most of the shell's config-consuming surface:
//! the bar/desktop/lockscreen widget-type registries (Phase 13/14/15, ~2100 lines
//! of not-yet-ported C++), the launcher provider list (task 14.1), the plugin
//! registry (`src/scripting/`, out of migration scope per MIGRATION_PLAN.md's
//! ground rules), and `time::isValidTimezone`. None of that exists in Rust yet, so
//! `validateMergedConfig` cannot be ported whole. MIGRATION_PLAN.md's task 2.4 is
//! split into 2.4.1 (this module: the checks whose dependencies are already
//! ported) through 2.4.5 (the rest, gated on their owning phases). See
//! MIGRATION_PLAN.md for the full breakdown.
//!
//! What's here, all direct ports of the correspondingly-named `config_validate.cpp`
//! functions:
//! - [`check_section`] — port of `checkSection`: unknown-key + schema-defaults
//!   check for every section in [`crate::schema::config_sections::sections`].
//! - [`validate_include_shape`] — port of `validateIncludeShape`.
//! - [`validate_calendar_syntax`] — port of `validateCalendarSyntax`.
//! - [`validate_location`] — port of `validateLocation`. Needs
//!   `day_night_schedule::normalizedClock` (`src/system/day_night_schedule.cpp`), a
//!   ~15-line pure HH:MM format check with no scheduling logic attached; pulled
//!   forward as the private [`normalized_clock`] helper below (same "minimal piece"
//!   pattern as 2.1.3's `KeyChord` POD) — the rest of `day_night_schedule.cpp`
//!   (`GeoCoordinates`/`resolveCoordinates`/`evaluate`) is still task 5.6's to port.
//! - [`validate_bars`] — port of `validateBars`: `BarConfig`/`BarMonitorOverride`
//!   *schema* checks only (both already ported, 2.1.2/2.3) — not
//!   `validateBarWidgets`, which needs Phase 13's widget-type registry (task 2.4.2).
//! - [`validate_merged_config`] — port of `validateMergedConfig`, wiring only the
//!   above. Deliberately does **not** wire (see MIGRATION_PLAN.md 2.4.2-2.4.4):
//!   `validateBarWidgets`/`validateDesktopWidgets`/`validateLockscreenWidgets`
//!   (Phase 13/14/15 widget-type registries), `validateLauncherProviders` (task
//!   14.1), `validatePluginSettings` (out of scope; scripting-only).
//!
//! Not ported at all in this task: `validateConfigSources`/`validateConfigFile`
//! (task 2.4.5 — both call `mergeConfigWithIncludes`/`ConfigService::deepMerge`
//! (task 2.5) and `normalizeLegacyConfig` (task 2.6) directly, neither of which
//! exists yet) and the CLI-level `config_validate_cli_test.sh` (needs a real
//! `noctalia config validate` binary and `config export full`, task 2.7).

use crate::schema::config_schema::{bar_fields_schema, bar_monitor_override_schema};
use crate::schema::config_sections::{SectionSpec, is_known_root_key, sections};
use crate::schema::diagnostics::Diagnostics;
use crate::schema::engine::{collect_unknown_keys, read_into};
use crate::types::bar::{BarConfig, BarMonitorOverride};

/// Port of `day_night_schedule::normalizedClock` (`src/system/day_night_schedule.cpp:103-119`).
/// Returns the input back unchanged if it's a valid `HH:MM` clock time (00-23:59),
/// otherwise `None`. See the module doc comment for why only this one function is
/// pulled forward here rather than the whole `day_night_schedule` module.
fn normalized_clock(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    if !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || !bytes[4].is_ascii_digit()
    {
        return None;
    }
    let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
    let minute = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(value)
}

/// Port of `checkSection` (`config_validate.cpp:232-249`).
pub fn check_section(root: &toml::Table, spec: &SectionSpec, diag: &mut Diagnostics) {
    let Some(toml::Value::Table(tbl)) = root.get(spec.name) else {
        return;
    };
    let mut unknown = Vec::new();
    (spec.collect_unknown)(tbl, &mut unknown);
    for path in unknown {
        if !spec.allow_unknown_paths.contains(path.as_str()) {
            diag.warn(path, "unknown setting");
        }
    }
    (spec.check_against_defaults)(tbl, diag);
}

/// Port of `validateIncludeShape` (`config_validate.cpp:197-228`).
pub fn validate_include_shape(tbl: &toml::Table, diag: &mut Diagnostics) {
    let Some(inc_value) = tbl.get("include") else {
        return;
    };
    let Some(inc) = inc_value.as_table() else {
        diag.fatal_with_code(
            "include",
            "[include] must be a table",
            "config.include.type",
        );
        return;
    };
    for (key, node) in inc {
        match key.as_str() {
            "autoload" => {
                if !matches!(node, toml::Value::Boolean(_)) {
                    diag.fatal_with_code(
                        "include.autoload",
                        "must be a boolean",
                        "config.include.type",
                    );
                }
            }
            "files" => match node.as_array() {
                None => {
                    diag.fatal_with_code(
                        "include.files",
                        "must be an array of strings",
                        "config.include.type",
                    );
                }
                Some(arr) => {
                    for el in arr {
                        if !matches!(el, toml::Value::String(_)) {
                            diag.fatal_with_code(
                                "include.files",
                                "every entry must be a string",
                                "config.include.type",
                            );
                            break;
                        }
                    }
                }
            },
            other => {
                diag.warn(format!("include.{other}"), "unknown setting");
            }
        }
    }
}

/// Port of `validateCalendarSyntax` (`config_validate.cpp:406-428`).
pub fn validate_calendar_syntax(root: &toml::Table, diag: &mut Diagnostics) {
    let Some(calendar) = root.get("calendar").and_then(|v| v.as_table()) else {
        return;
    };
    if calendar
        .get("accounts")
        .and_then(|v| v.as_array())
        .is_some()
    {
        diag.error(
            "calendar.accounts",
            "calendar accounts now use [calendar.account.<id>] named tables",
        );
    }
    let Some(accounts) = calendar.get("account").and_then(|v| v.as_table()) else {
        return;
    };
    for (id, node) in accounts {
        let Some(account) = node.as_table() else {
            continue;
        };
        if !account.contains_key("url") {
            continue;
        }
        diag.error(
            format!("calendar.account.{id}.url"),
            "CalDAV collection url was removed; use provider/server_url discovery syntax instead",
        );
    }
}

/// Port of `validateLocation` (`config_validate.cpp:91-115`).
pub fn validate_location(merged: &toml::Table, diag: &mut Diagnostics) {
    let Some(location) = merged.get("location").and_then(|v| v.as_table()) else {
        return;
    };
    let custom_schedule = location
        .get("custom_schedule")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    for key in ["sunset", "sunrise"] {
        let path = format!("location.{key}");
        let value = location.get(key).and_then(|v| v.as_str());
        let set = value.is_some_and(|v| !v.is_empty());
        if set {
            // SAFETY-equivalent invariant, not memory safety: `set` guarantees `value` is `Some`.
            let value = value.unwrap_or_default();
            if normalized_clock(value).is_none() {
                let message = format!("\"{value}\" is not a time of day in HH:MM form");
                if custom_schedule {
                    diag.error_with_code(path, message, "location.clock.invalid");
                } else {
                    diag.warn(path, message);
                }
            }
        } else if custom_schedule {
            diag.error_with_code(
                path,
                format!("custom_schedule needs a {key} time in HH:MM form"),
                "location.clock.missing",
            );
        }
    }
}

/// Port of `validateBars` (`config_validate.cpp:674-724`). Schema-only: does not
/// port `validateBarWidgets` (task 2.4.2, needs Phase 13's widget-type registry).
pub fn validate_bars(root: &toml::Table, diag: &mut Diagnostics) {
    let Some(bars) = root.get("bar").and_then(|v| v.as_table()) else {
        return;
    };
    for (name, node) in bars {
        if name == "order" {
            continue;
        }
        let Some(bar_tbl) = node.as_table() else {
            continue;
        };
        let base = format!("bar.{name}");
        let mut unknown = Vec::new();
        collect_unknown_keys(bar_tbl, bar_fields_schema(), &base, &mut unknown);
        for path in unknown {
            // position + the monitor override map are handled outside barFieldsSchema.
            if path == format!("{base}.position") || path == format!("{base}.monitor") {
                continue;
            }
            diag.warn(path, "unknown setting");
        }
        let mut tmp_bar = BarConfig::default();
        read_into(bar_tbl, &mut tmp_bar, bar_fields_schema(), &base, diag);

        if let Some(monitors) = bar_tbl.get("monitor").and_then(|v| v.as_table()) {
            for (match_key, mon_node) in monitors {
                let Some(mon_tbl) = mon_node.as_table() else {
                    continue;
                };
                let mon_base = format!("{base}.monitor.{match_key}");
                let mut mon_unknown = Vec::new();
                collect_unknown_keys(
                    mon_tbl,
                    bar_monitor_override_schema(),
                    &mon_base,
                    &mut mon_unknown,
                );
                for path in mon_unknown {
                    diag.warn(path, "unknown setting");
                }
                let mut tmp_ovr = BarMonitorOverride::default();
                read_into(
                    mon_tbl,
                    &mut tmp_ovr,
                    bar_monitor_override_schema(),
                    &mon_base,
                    diag,
                );
            }
        }
    }
}

/// Partial port of `validateMergedConfig` (`config_validate.cpp:806-810`, via
/// `appendMergedConfigDiagnostics`, `config_validate.cpp:726-765`). See the module
/// doc comment for exactly what's wired vs. deferred.
pub fn validate_merged_config(merged: &toml::Table) -> Diagnostics {
    let mut diag = Diagnostics::default();
    for spec in sections() {
        check_section(merged, spec, &mut diag);
    }
    validate_calendar_syntax(merged, &mut diag);
    validate_location(merged, &mut diag);
    validate_bars(merged, &mut diag);
    validate_include_shape(merged, &mut diag);

    for (key, _) in merged {
        if !is_known_root_key(key) {
            diag.warn(key.clone(), "unknown section");
        }
    }
    diag
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::schema::diagnostics::{RecoveryScope, Severity};

    fn parse(toml_str: &str) -> toml::Table {
        toml::from_str(toml_str).expect("test fixture must be valid TOML")
    }

    fn find<'a>(
        diag: &'a Diagnostics,
        path: &str,
    ) -> Option<&'a crate::schema::diagnostics::DiagnosticEntry> {
        diag.entries.iter().find(|e| e.path == path)
    }

    #[test]
    fn unknown_key_in_registered_section_warns() {
        let tbl = parse("[accessibility]\nui_scale = 1.0\nui_scl = 1.25\n");
        let diag = validate_merged_config(&tbl);
        let entry = find(&diag, "accessibility.ui_scl").expect("unknown key should be reported");
        assert_eq!(entry.severity, Severity::Warning);
        assert_eq!(entry.message, "unknown setting");
    }

    #[test]
    fn unknown_top_level_key_warns() {
        let tbl = parse("[not_a_real_section]\nfoo = 1\n");
        let diag = validate_merged_config(&tbl);
        let entry = find(&diag, "not_a_real_section").expect("unknown section should be reported");
        assert_eq!(entry.message, "unknown section");
    }

    #[test]
    fn known_custom_root_keys_are_not_flagged_unknown() {
        let tbl = parse("[widget.temp]\ntype = \"temp\"\n[bar.main]\nenabled = true\n");
        let diag = validate_merged_config(&tbl);
        assert!(find(&diag, "widget").is_none());
        assert!(find(&diag, "bar").is_none());
    }

    #[test]
    fn enum_field_unknown_value_warns_via_check_against_defaults() {
        let tbl = parse("[wallpaper]\nfill_mode = \"bogus\"\n");
        let diag = validate_merged_config(&tbl);
        let entry = find(&diag, "wallpaper.fill_mode").expect("bad enum value should warn");
        assert_eq!(entry.severity, Severity::Warning);
        assert!(entry.message.contains("bogus"));
    }

    #[test]
    fn include_shape_valid_produces_no_diagnostics() {
        let tbl = parse("[include]\nautoload = true\nfiles = [\"a.toml\", \"b.toml\"]\n");
        let mut diag = Diagnostics::default();
        validate_include_shape(&tbl, &mut diag);
        assert!(diag.entries.is_empty());
    }

    #[test]
    fn include_autoload_wrong_type_is_fatal() {
        let tbl = parse("[include]\nautoload = \"yes\"\n");
        let mut diag = Diagnostics::default();
        validate_include_shape(&tbl, &mut diag);
        let entry = find(&diag, "include.autoload").expect("wrong type should be fatal");
        assert_eq!(entry.recovery_scope, RecoveryScope::Document);
    }

    #[test]
    fn include_files_non_string_entry_is_fatal() {
        let tbl = parse("[include]\nfiles = [\"a.toml\", 5]\n");
        let mut diag = Diagnostics::default();
        validate_include_shape(&tbl, &mut diag);
        let entry = find(&diag, "include.files").expect("non-string entry should be fatal");
        assert_eq!(entry.recovery_scope, RecoveryScope::Document);
    }

    #[test]
    fn include_unknown_key_warns() {
        let tbl = parse("[include]\nbogus = true\n");
        let mut diag = Diagnostics::default();
        validate_include_shape(&tbl, &mut diag);
        let entry = find(&diag, "include.bogus").expect("unknown include key should warn");
        assert_eq!(entry.severity, Severity::Warning);
    }

    #[test]
    fn calendar_old_array_syntax_errors() {
        let tbl = parse("[calendar]\naccounts = [1, 2]\n");
        let mut diag = Diagnostics::default();
        validate_calendar_syntax(&tbl, &mut diag);
        let entry = find(&diag, "calendar.accounts").expect("old array syntax should error");
        assert_eq!(entry.severity, Severity::Error);
    }

    #[test]
    fn calendar_account_url_key_errors() {
        let tbl = parse("[calendar.account.work]\nurl = \"https://example.com/cal\"\n");
        let mut diag = Diagnostics::default();
        validate_calendar_syntax(&tbl, &mut diag);
        let entry =
            find(&diag, "calendar.account.work.url").expect("removed url syntax should error");
        assert_eq!(entry.severity, Severity::Error);
    }

    #[test]
    fn calendar_account_without_url_is_clean() {
        let tbl = parse("[calendar.account.work]\nprovider = \"caldav\"\n");
        let mut diag = Diagnostics::default();
        validate_calendar_syntax(&tbl, &mut diag);
        assert!(diag.entries.is_empty());
    }

    #[test]
    fn location_valid_clock_times_are_clean() {
        let tbl =
            parse("[location]\ncustom_schedule = true\nsunset = \"20:15\"\nsunrise = \"06:30\"\n");
        let mut diag = Diagnostics::default();
        validate_location(&tbl, &mut diag);
        assert!(diag.entries.is_empty());
    }

    #[test]
    fn location_invalid_clock_with_custom_schedule_errors() {
        let tbl =
            parse("[location]\ncustom_schedule = true\nsunset = \"25:99\"\nsunrise = \"06:30\"\n");
        let mut diag = Diagnostics::default();
        validate_location(&tbl, &mut diag);
        let entry = find(&diag, "location.sunset").expect("bad clock should be reported");
        assert_eq!(entry.severity, Severity::Error);
        assert_eq!(entry.recovery_scope, RecoveryScope::Value);
    }

    #[test]
    fn location_invalid_clock_without_custom_schedule_warns() {
        let tbl = parse("[location]\nsunset = \"25:99\"\n");
        let mut diag = Diagnostics::default();
        validate_location(&tbl, &mut diag);
        let entry = find(&diag, "location.sunset").expect("bad clock should be reported");
        assert_eq!(entry.severity, Severity::Warning);
    }

    #[test]
    fn location_missing_clock_with_custom_schedule_errors() {
        let tbl = parse("[location]\ncustom_schedule = true\nsunrise = \"06:30\"\n");
        let mut diag = Diagnostics::default();
        validate_location(&tbl, &mut diag);
        let entry = find(&diag, "location.sunset").expect("missing clock should be reported");
        assert_eq!(entry.severity, Severity::Error);
        assert!(entry.message.contains("sunset"));
    }

    #[test]
    fn location_missing_clock_without_custom_schedule_is_clean() {
        let tbl = parse("[location]\n");
        let mut diag = Diagnostics::default();
        validate_location(&tbl, &mut diag);
        assert!(diag.entries.is_empty());
    }

    #[test]
    fn bar_unknown_key_warns_but_position_and_monitor_are_exempt() {
        let tbl = parse(
            "[bar.main]\nposition = \"top\"\nenabled = true\nbogus = 1\n[bar.main.monitor.eDP-1]\nposition = \"bottom\"\n",
        );
        let mut diag = Diagnostics::default();
        validate_bars(&tbl, &mut diag);
        assert!(find(&diag, "bar.main.position").is_none());
        assert!(find(&diag, "bar.main.monitor").is_none());
        let entry = find(&diag, "bar.main.bogus").expect("truly unknown bar key should warn");
        assert_eq!(entry.severity, Severity::Warning);
    }

    #[test]
    fn bar_order_key_is_skipped_entirely() {
        let tbl = parse("[bar]\norder = [\"main\"]\n");
        let mut diag = Diagnostics::default();
        validate_bars(&tbl, &mut diag);
        assert!(diag.entries.is_empty());
    }

    #[test]
    fn bar_monitor_override_unknown_key_warns() {
        let tbl = parse("[bar.main.monitor.eDP-1]\nbogus_override = true\n");
        let mut diag = Diagnostics::default();
        validate_bars(&tbl, &mut diag);
        let entry = find(&diag, "bar.main.monitor.eDP-1.bogus_override")
            .expect("unknown monitor override key should warn");
        assert_eq!(entry.severity, Severity::Warning);
    }

    #[test]
    fn normalized_clock_accepts_valid_and_rejects_invalid() {
        assert_eq!(normalized_clock("06:30"), Some("06:30"));
        assert_eq!(normalized_clock("23:59"), Some("23:59"));
        assert_eq!(normalized_clock("24:00"), None);
        assert_eq!(normalized_clock("06:60"), None);
        assert_eq!(normalized_clock("6:30"), None);
        assert_eq!(normalized_clock("06-30"), None);
        assert_eq!(normalized_clock(""), None);
    }

    #[test]
    fn shell_launcher_providers_are_populated_via_named_map_not_array_of() {
        // Regression test for a bug found (not by the review, independently) while
        // fixing the false positives above: `shell_launcher_schema`'s `providers`
        // field used `array_of`, which only ever matches a TOML *array* — but
        // `[shell.launcher.providers.<name>]` is a table of named sub-tables (see
        // `tests/config_validate/warn-only.toml`), so real provider configs were
        // silently never read into `LauncherConfig::providers` at all. Now uses
        // `named_map`, matching `config_schema.cpp`'s `shellLauncherSchema`.
        use crate::schema::config_schema::shell_schema;
        use crate::schema::engine::read_into;
        use crate::types::shell::ShellConfig;

        let tbl: toml::Table = toml::from_str(
            "[launcher.providers.emoji]\nprefix = \"em\"\n[launcher.providers.wallpaper]\nprefix = \"wp\"\nbogus_key = 1\n",
        )
        .expect("test fixture must be valid TOML");
        // shell_schema reads a ShellConfig's own fields directly, so nest under a
        // synthetic "launcher" key matching what a bare LauncherConfig sub-table read expects.
        let launcher_tbl = tbl
            .get("launcher")
            .and_then(|v| v.as_table())
            .expect("launcher table");
        let mut wrapper = toml::Table::new();
        wrapper.insert(
            "launcher".to_string(),
            toml::Value::Table(launcher_tbl.clone()),
        );
        let mut out = ShellConfig::default();
        let mut diag = Diagnostics::default();
        read_into(&wrapper, &mut out, shell_schema(), "shell", &mut diag);

        let mut names: Vec<&str> = out
            .launcher
            .providers
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["emoji", "wallpaper"]);
        let emoji = out
            .launcher
            .providers
            .iter()
            .find(|p| p.name == "emoji")
            .expect("emoji provider populated");
        assert_eq!(emoji.prefix, "em");

        let mut unknown = Vec::new();
        collect_unknown_keys(&wrapper, shell_schema(), "shell", &mut unknown);
        assert!(
            unknown.contains(&"shell.launcher.providers.wallpaper.bogus_key".to_string()),
            "unknown keys within a named-map element should still be reported: {unknown:?}"
        );
    }

    #[test]
    fn whole_example_toml_produces_zero_diagnostics() {
        // The real repo-root `example.toml`, run through every check this task
        // wires. This is the regression guard for the false positives found by
        // this task's own fresh-context review: `validate_merged_config` calls
        // `checkSection` over every registered section, which depends on every
        // section schema in `config_schema.rs` being *complete* (every real TOML
        // key registered, not just present-but-mistyped). The review's first pass
        // found ~30 spurious "unknown setting" warnings here, all traced to gaps
        // in already-committed task-2.3 schemas (missing fields in
        // `shell_animation_schema`/`shell_shadow_schema`/`shell_panel_schema`/
        // `shell_launcher_schema`/`wallpaper_automation_schema`/`templates_schema`/
        // `control_center_schema`, an empty `keybinds_schema`, and a
        // `shell_launcher_schema` bug where `providers` was read via `array_of`
        // instead of `named_map` and so was never populated from real
        // `[shell.launcher.providers.*]` tables at all) — all fixed in this same
        // commit. If this test starts failing again, the fix is almost always in
        // `config_schema.rs`, not here.
        let example = include_str!("../../../example.toml");
        let tbl: toml::Table = toml::from_str(example).expect("example.toml is valid TOML");
        let diag = validate_merged_config(&tbl);
        assert!(
            diag.entries.is_empty(),
            "example.toml should validate cleanly: {:?}",
            diag.entries
        );
    }
}
