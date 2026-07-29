//! Task 2.3 / 2.1.10 — Integration tests for declarative config schema and validation rules.
//! Port of relevant assertions from `tests/config_schema_roundtrip_test.cpp`.

use noctalia_config::schema::config_schema::is_known_config_path;
use noctalia_config::schema::config_sections::{find_section, sections};
use noctalia_config::types::plugins::is_valid_plugin_source_name;

#[test]
fn all_sections_are_registered_and_findable() {
    let all = sections();
    assert_eq!(all.len(), 24);
    for spec in all {
        let found = find_section(spec.name);
        assert!(found.is_some(), "Section {} should be findable", spec.name);
        assert_eq!(found.map(|s| s.name), Some(spec.name));
    }
}

fn check_path(p: &str) -> bool {
    let parts: Vec<String> = p.split('.').map(|s| s.to_string()).collect();
    is_known_config_path(&parts)
}

#[test]
fn is_known_config_path_identifies_valid_and_invalid_paths() {
    assert!(check_path("bar.default"));
    assert!(check_path("theme.mode"));
    assert!(check_path("wallpaper.enabled"));
    assert!(check_path("audio.sound_volume"));
    assert!(check_path("weather.unit"));
    assert!(check_path("osd.scale"));
    assert!(check_path("backdrop.blur_intensity"));
    assert!(check_path("lockscreen.blur_intensity"));
    assert!(check_path("system.monitor.cpu_poll_seconds"));
    assert!(check_path("nightlight.enabled"));
    assert!(check_path("location.latitude"));
    assert!(check_path("notification.border"));
    assert!(check_path("dock.enabled"));
    assert!(check_path("hot_corners.top_left.action"));
    assert!(check_path("brightness.enable_ddcutil"));
    assert!(check_path("battery.warning_threshold"));
    assert!(check_path("control_center.sidebar_mode"));
    assert!(check_path("plugins.auto_update"));
    assert!(check_path("calendar.enabled"));
    assert!(check_path("hooks.started"));
    assert!(check_path("idle.pre_action_fade_seconds"));
    assert!(check_path("shell.button_borders"));
    assert!(check_path("accessibility.ui_scale"));
    assert!(check_path("storage.key_source"));

    assert!(!check_path("theme")); // bare section name is not a setting path
    assert!(!check_path("nonexistent_section"));
    assert!(!check_path("bar.foo.bar.baz"));
}

#[test]
fn plugin_source_name_validation_matches_cpp() {
    let valid = ["official", "my-repo", "team.plugins", "repo_2", "A1"];
    for name in valid {
        assert!(
            is_valid_plugin_source_name(name),
            "Rejected valid source name {name}"
        );
    }

    let invalid = [
        "",
        ".",
        "..",
        "../repo",
        "repo/name",
        "repo name",
        "-repo",
        "_repo",
    ];
    for name in invalid {
        assert!(
            !is_valid_plugin_source_name(name),
            "Accepted invalid source name {name}"
        );
    }
}
