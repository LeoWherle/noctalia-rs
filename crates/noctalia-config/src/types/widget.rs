//! Task 2.2 — widget config. Port of `src/config/widget_config.{cpp,h}`'s three
//! functions: [`read_widget_setting_value`] (`readWidgetSettingValue`),
//! [`seed_builtin_widgets`] (`seedBuiltinWidgets`), and
//! [`read_bar_widget_config`] (`readBarWidgetConfig`).
//!
//! `WidgetSettingValue` itself (`widget_setting_value.h`) and the capsule-group
//! reconciliation helpers this task's own test file also exercises
//! (`resolveWidgetBarCapsuleSpec`) already landed under task 2.1.2 — see
//! `crate::types::widget_setting_value` / `crate::types::bar`.

use std::collections::HashMap;

use toml::Value;

use crate::types::bar::WidgetConfig;
use crate::types::widget_setting_value::{WidgetSettingStringMap, WidgetSettingValue};

/// `ui/style.h`'s `Style::fontSizeBody`, needed by `seed_builtin_widgets`'s
/// `active_window` default. `ui/style.h` has no assigned migration task yet;
/// same "placeholder until the real thing lands" pattern as `bar.rs`'s
/// `style_defaults` module (kept as its own small constant here rather than
/// making that module `pub`, since it's the only value this file needs).
const FONT_SIZE_BODY: f32 = 14.0;

/// Port of `readWidgetSettingValue` (widget_config.cpp:13-54). The C++'s outer
/// dispatch uses toml++'s exact-type accessors (`node.as_string()`/
/// `as_integer()`/`as_floating_point()`/`as_boolean()`), which never coerce
/// across TOML types — each of `Value::as_str`/`as_integer`/`as_float`/
/// `as_bool` here likewise only returns `Some` for a node that's actually that
/// TOML type, matching exactly.
#[must_use]
pub fn read_widget_setting_value(node: &Value) -> Option<WidgetSettingValue> {
    if let Some(s) = node.as_str() {
        return Some(WidgetSettingValue::String(s.to_string()));
    }
    if let Some(i) = node.as_integer() {
        return Some(WidgetSettingValue::Int(i));
    }
    if let Some(f) = node.as_float() {
        return Some(WidgetSettingValue::Float(f));
    }
    if let Some(b) = node.as_bool() {
        return Some(WidgetSettingValue::Bool(b));
    }
    if let Some(array) = node.as_array() {
        // Non-string elements are silently dropped, not rejected (widget_config.cpp:28-32:
        // `item.value<std::string>()` inside an `if`, no `else` branch bailing out).
        let strings: Vec<String> = array
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect();
        return Some(WidgetSettingValue::StringList(strings));
    }
    if let Some(table) = node.as_table() {
        // Unlike the array case, a single non-string value fails the *whole* table
        // (widget_config.cpp:43-45: an early `return std::nullopt`).
        let mut strings = WidgetSettingStringMap::new();
        for (key, value_node) in table {
            let value = value_node.as_str()?;
            strings.insert(key.clone(), value.to_string());
        }
        return Some(WidgetSettingValue::StringMap(strings));
    }
    None
}

/// Port of `seedBuiltinWidgets` (widget_config.cpp:56-133). Takes
/// `widgets: &mut HashMap<String, WidgetConfig>` directly rather than `&mut
/// Config` (config_types.h's not-yet-ported root aggregate, task 2.1.9): the
/// C++ function only ever touches `config.widgets`, so this follows the same
/// adaptation as `bar.rs`'s `output_matches_selector` — callers pass
/// `&mut config.widgets` once the root `Config` struct lands.
pub fn seed_builtin_widgets(widgets: &mut HashMap<String, WidgetConfig>) {
    fn seed(
        widgets: &mut HashMap<String, WidgetConfig>,
        name: &str,
        r#type: &str,
        settings: Vec<(&str, WidgetSettingValue)>,
    ) {
        let mut wc = WidgetConfig {
            r#type: r#type.to_string(),
            ..WidgetConfig::default()
        };
        for (key, value) in settings {
            wc.settings.insert(key.to_string(), value);
        }
        widgets.insert(name.to_string(), wc);
    }

    seed(
        widgets,
        "cpu",
        "sysmon",
        vec![("stat", WidgetSettingValue::String("cpu_usage".to_string()))],
    );
    seed(
        widgets,
        "temp",
        "sysmon",
        vec![("stat", WidgetSettingValue::String("cpu_temp".to_string()))],
    );
    seed(
        widgets,
        "ram",
        "sysmon",
        vec![("stat", WidgetSettingValue::String("ram_used".to_string()))],
    );
    seed(
        widgets,
        "network_tx",
        "sysmon",
        vec![("stat", WidgetSettingValue::String("net_tx".to_string()))],
    );
    seed(
        widgets,
        "network_rx",
        "sysmon",
        vec![("stat", WidgetSettingValue::String("net_rx".to_string()))],
    );
    seed(
        widgets,
        "output_volume",
        "volume",
        vec![("device", WidgetSettingValue::String("output".to_string()))],
    );
    seed(
        widgets,
        "input_volume",
        "volume",
        vec![("device", WidgetSettingValue::String("input".to_string()))],
    );
    seed(
        widgets,
        "date",
        "clock",
        vec![(
            "format",
            WidgetSettingValue::String("{:%a %d %b}".to_string()),
        )],
    );
    seed(
        widgets,
        "active_window",
        "active_window",
        vec![
            ("max_length", WidgetSettingValue::Float(260.0)),
            ("min_length", WidgetSettingValue::Float(80.0)),
            (
                "icon_size",
                WidgetSettingValue::Float(f64::from(FONT_SIZE_BODY)),
            ),
            (
                "title_scroll",
                WidgetSettingValue::String("none".to_string()),
            ),
        ],
    );
    seed(
        widgets,
        "media",
        "media",
        vec![
            ("max_length", WidgetSettingValue::Float(220.0)),
            ("min_length", WidgetSettingValue::Float(80.0)),
            ("art_size", WidgetSettingValue::Float(16.0)),
            (
                "title_scroll",
                WidgetSettingValue::String("none".to_string()),
            ),
        ],
    );
    seed(
        widgets,
        "keyboard_layout",
        "keyboard_layout",
        vec![("hide_when_single_layout", WidgetSettingValue::Bool(false))],
    );
    seed(
        widgets,
        "lock_keys",
        "lock_keys",
        vec![
            ("show_caps_lock", WidgetSettingValue::Bool(true)),
            ("show_num_lock", WidgetSettingValue::Bool(true)),
            ("show_scroll_lock", WidgetSettingValue::Bool(false)),
            ("hide_when_off", WidgetSettingValue::Bool(false)),
            ("display", WidgetSettingValue::String("short".to_string())),
        ],
    );
    seed(
        widgets,
        "spacer",
        "spacer",
        vec![("interactive", WidgetSettingValue::Bool(false))],
    );
}

/// Resolves one `[widget.<name>]` table against the config parsed so far —
/// the canonical bar-widget instance resolution used by runtime loading and
/// `config validate`. Port of `readBarWidgetConfig` (widget_config.cpp:135-170).
///
/// Takes `base_widgets: &HashMap<String, WidgetConfig>` rather than `&Config`
/// (same adaptation as [`seed_builtin_widgets`], for the same reason: the C++
/// only ever reads `baseConfig.widgets`).
#[must_use]
pub fn read_bar_widget_config(
    widget_name: &str,
    entry_table: &toml::Table,
    base_widgets: &HashMap<String, WidgetConfig>,
) -> WidgetConfig {
    let mut wc = WidgetConfig::default();

    if let Some(type_value) = entry_table.get("type").and_then(Value::as_str) {
        wc.r#type = type_value.to_string();
        if let Some(base) = base_widgets.get(widget_name)
            && base.r#type == wc.r#type
        {
            wc.settings = base.settings.clone();
            wc.tables = base.tables.clone();
        }
    } else if let Some(base) = base_widgets.get(widget_name) {
        wc = base.clone();
    } else {
        wc.r#type = widget_name.to_string();
    }

    for (key, value) in entry_table {
        if key == "type" {
            continue;
        }
        if let Some(table_value) = value.as_table() {
            let mut table = HashMap::new();
            for (table_key, table_node) in table_value {
                if let Some(parsed) = table_node.as_str() {
                    table.insert(table_key.clone(), parsed.to_string());
                }
            }
            wc.tables.insert(key.clone(), table);
        } else if let Some(parsed) = read_widget_setting_value(value) {
            wc.settings.insert(key.clone(), parsed);
        }
    }

    wc
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::bar::{BarConfig, resolve_widget_bar_capsule_spec};

    fn parse_widget_table(toml_str: &str, table_name: &str, widget_name: &str) -> toml::Table {
        let parsed: toml::Table = toml::from_str(toml_str).expect("test TOML parses");
        let root = parsed
            .get(table_name)
            .and_then(Value::as_table)
            .unwrap_or_else(|| panic!("[{table_name}] table"));
        root.get(widget_name)
            .and_then(Value::as_table)
            .unwrap_or_else(|| panic!("[{table_name}.{widget_name}] table"))
            .clone()
    }

    fn string_setting<'a>(widget: &'a WidgetConfig, key: &str) -> &'a str {
        match widget.settings.get(key) {
            Some(WidgetSettingValue::String(s)) => s.as_str(),
            other => panic!("expected string setting '{key}', got {other:?}"),
        }
    }

    fn bool_setting(widget: &WidgetConfig, key: &str) -> bool {
        match widget.settings.get(key) {
            Some(WidgetSettingValue::Bool(b)) => *b,
            other => panic!("expected bool setting '{key}', got {other:?}"),
        }
    }

    /// Port of `config_widget_test.cpp`'s `temp` case: an entry table with no
    /// `type` key resolves against a matching-name builtin, inheriting its
    /// settings and layering the entry's own on top.
    #[test]
    fn temp_resolves_against_builtin_and_layers_overrides() {
        let mut base = HashMap::new();
        seed_builtin_widgets(&mut base);

        let table = parse_widget_table("[widget.temp]\nshow_label = false\n", "widget", "temp");
        let temp = read_bar_widget_config("temp", &table, &base);

        assert_eq!(temp.r#type, "sysmon");
        assert_eq!(string_setting(&temp, "stat"), "cpu_temp");
        assert!(!bool_setting(&temp, "show_label"));
    }

    /// Port of `config_widget_test.cpp`'s `my_clock` case: an unknown widget
    /// name with no builtin match resolves its type to its own entry name.
    #[test]
    fn unknown_widget_name_resolves_to_its_own_type() {
        let base = HashMap::new();
        let table = parse_widget_table(
            "[widget.my_clock]\nformat = \"{:%H:%M}\"\n",
            "widget",
            "my_clock",
        );
        let custom = read_bar_widget_config("my_clock", &table, &base);

        assert_eq!(custom.r#type, "my_clock");
        assert_eq!(string_setting(&custom, "format"), "{:%H:%M}");
    }

    /// Port of `config_widget_test.cpp`'s `keyboard_layout` case: a nested
    /// table setting becomes a `tables` entry, not a `settings` one.
    #[test]
    fn nested_table_setting_becomes_a_string_map_table() {
        let mut base = HashMap::new();
        seed_builtin_widgets(&mut base);

        let table = parse_widget_table(
            "[widget.keyboard_layout]\n\
             show_label = true\n\
             [widget.keyboard_layout.custom_labels]\n\
             \"English (US)\" = \"EN\"\n",
            "widget",
            "keyboard_layout",
        );
        let layout = read_bar_widget_config("keyboard_layout", &table, &base);

        assert_eq!(layout.r#type, "keyboard_layout");
        assert!(!bool_setting(&layout, "hide_when_single_layout"));
        assert!(bool_setting(&layout, "show_label"));
        assert_eq!(
            layout
                .tables
                .get("custom_labels")
                .and_then(|m| m.get("English (US)"))
                .map(String::as_str),
            Some("EN")
        );
    }

    /// Port of `config_widget_test.cpp`'s string-map `readWidgetSettingValue`
    /// case.
    #[test]
    fn read_widget_setting_value_parses_a_string_map_table() {
        let parsed: toml::Table =
            toml::from_str("[output_glyphs]\n\"eDP-1\" = \"laptop\"\n\"DP-1\" = \"monitor\"\n")
                .expect("parses");
        let node = parsed.get("output_glyphs").expect("output_glyphs present");

        let value = read_widget_setting_value(node).expect("string-map setting parses");
        let WidgetSettingValue::StringMap(map) = value else {
            panic!("expected a StringMap value");
        };
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("eDP-1").map(String::as_str), Some("laptop"));
        assert_eq!(map.get("DP-1").map(String::as_str), Some("monitor"));
    }

    /// Port of `config_widget_test.cpp`'s invalid string-map case: a
    /// non-string value anywhere in the table rejects the whole table.
    #[test]
    fn read_widget_setting_value_rejects_a_table_with_a_non_string_value() {
        let parsed: toml::Table =
            toml::from_str("[output_glyphs]\n\"eDP-1\" = 1\n").expect("parses");
        let node = parsed.get("output_glyphs").expect("output_glyphs present");

        assert!(read_widget_setting_value(node).is_none());
    }

    /// No C++ test exercises this directly (`tests/config_widget_test.cpp` has
    /// no array-of-strings/`getStringList` case — confirmed via grep); added
    /// to independently cover the array-vs-table asymmetry documented on
    /// `read_widget_setting_value` itself (non-string array elements are
    /// dropped, not rejecting, unlike the table case).
    #[test]
    fn read_widget_setting_value_drops_non_string_array_elements() {
        let parsed: toml::Table = toml::from_str("v = [\"a\", 1, \"b\"]\n").expect("parses");
        let node = parsed.get("v").expect("v present");

        let value = read_widget_setting_value(node).expect("array setting parses");
        assert_eq!(
            value,
            WidgetSettingValue::StringList(vec!["a".to_string(), "b".to_string()])
        );
    }

    /// Port of `config_widget_test.cpp`'s `resolveWidgetBarCapsuleSpec` cases:
    /// an `"auto"` capsule_radius keeps the bar's radius; a numeric one
    /// overrides it. (`WidgetSettingValue`/`resolveWidgetBarCapsuleSpec`
    /// themselves already have their own dedicated tests under task 2.1.2 —
    /// these two just confirm this task's own settings map feeds them correctly.)
    #[test]
    fn automatic_and_explicit_widget_capsule_radius() {
        let bar = BarConfig {
            widget_capsule_radius: Some(12.0),
            ..Default::default()
        };
        let mut launcher = WidgetConfig::default();
        launcher
            .settings
            .insert("capsule".to_string(), WidgetSettingValue::Bool(true));
        launcher.settings.insert(
            "capsule_radius".to_string(),
            WidgetSettingValue::String("auto".to_string()),
        );

        let automatic = resolve_widget_bar_capsule_spec(&bar, Some(&launcher));
        assert!(automatic.enabled);
        assert_eq!(automatic.radius, Some(12.0));

        launcher
            .settings
            .insert("capsule_radius".to_string(), WidgetSettingValue::Float(7.0));
        let explicit = resolve_widget_bar_capsule_spec(&bar, Some(&launcher));
        assert_eq!(explicit.radius, Some(7.0));
    }
}
