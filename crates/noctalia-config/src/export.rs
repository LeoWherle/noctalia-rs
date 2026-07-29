//! Config export / serialization to TOML table.
//! Port of `src/config/config_export.{cpp,h}`.

use std::collections::HashMap;

use crate::schema::config_schema::bar_fields_schema;
use crate::schema::config_sections::sections;
use crate::schema::engine::write_table;
use crate::types::bar::{BarConfig, BarMonitorOverride, WidgetConfig};
use crate::types::config::Config;
use crate::types::desktop_widgets::{
    DesktopWidgetState, DesktopWidgetsConfig, DesktopWidgetsGridState,
};
use crate::types::plugins::PluginsConfig;
use crate::types::widget_setting_value::WidgetSettingValue;

fn string_array(values: &[String]) -> Vec<toml::Value> {
    let mut array = Vec::new();
    for value in values {
        array.push(toml::Value::String(value.clone()));
    }
    array
}

fn string_map_table(values: &HashMap<String, String>) -> toml::Table {
    let mut table = toml::Table::new();
    let mut keys: Vec<&String> = values.keys().collect();
    keys.sort();
    for key in keys {
        if let Some(val) = values.get(key) {
            table.insert(key.clone(), toml::Value::String(val.clone()));
        }
    }
    table
}

fn insert_widget_setting_value(table: &mut toml::Table, key: &str, value: &WidgetSettingValue) {
    let val = match value {
        WidgetSettingValue::Bool(b) => toml::Value::Boolean(*b),
        WidgetSettingValue::Int(i) => toml::Value::Integer(*i),
        WidgetSettingValue::Float(f) => toml::Value::Float(*f),
        WidgetSettingValue::String(s) => toml::Value::String(s.clone()),
        WidgetSettingValue::StringList(list) => toml::Value::Array(string_array(list)),
        WidgetSettingValue::StringMap(map) => toml::Value::Table(string_map_table(map)),
    };
    table.insert(key.to_string(), val);
}

fn widget_config_table(widget: &WidgetConfig) -> toml::Table {
    let mut table = toml::Table::new();
    if !widget.r#type.is_empty() {
        table.insert(
            "type".to_string(),
            toml::Value::String(widget.r#type.clone()),
        );
    }

    let mut keys: Vec<&String> = widget.settings.keys().collect();
    keys.sort();
    for key in keys {
        if let Some(val) = widget.settings.get(key) {
            insert_widget_setting_value(&mut table, key, val);
        }
    }

    let mut table_keys: Vec<&String> = widget.tables.keys().collect();
    table_keys.sort();
    for key in table_keys {
        if let Some(map) = widget.tables.get(key) {
            let mut subtable = toml::Table::new();
            let mut map_keys: Vec<&String> = map.keys().collect();
            map_keys.sort();
            for map_key in map_keys {
                if let Some(val) = map.get(map_key) {
                    subtable.insert(map_key.clone(), toml::Value::String(val.clone()));
                }
            }
            table.insert(key.clone(), toml::Value::Table(subtable));
        }
    }
    table
}

fn plugin_settings_table(plugins: &PluginsConfig) -> toml::Table {
    let mut table = toml::Table::new();
    let mut plugin_ids: Vec<&String> = plugins.plugin_settings.keys().collect();
    plugin_ids.sort();

    for plugin_id in plugin_ids {
        if let Some(settings) = plugins.plugin_settings.get(plugin_id) {
            let mut per_plugin = toml::Table::new();
            let mut keys: Vec<&String> = settings.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(val) = settings.get(key) {
                    insert_widget_setting_value(&mut per_plugin, key, val);
                }
            }
            table.insert(plugin_id.clone(), toml::Value::Table(per_plugin));
        }
    }
    table
}

fn apply_monitor_override(base: &BarConfig, ovr: &BarMonitorOverride) -> BarConfig {
    let mut resolved = base.clone();
    if let Some(pos) = &ovr.position {
        resolved.position = pos.clone();
    }
    if let Some(enabled) = ovr.enabled {
        resolved.enabled = enabled;
    }
    if let Some(auto_hide) = ovr.auto_hide {
        resolved.auto_hide = auto_hide;
    }
    if let Some(smart) = ovr.smart_auto_hide {
        resolved.smart_auto_hide = smart;
    }
    if let Some(show) = ovr.show_on_workspace_switch {
        resolved.show_on_workspace_switch = show;
    }
    if let Some(reserve) = ovr.reserve_space {
        resolved.reserve_space = reserve;
    }
    if let Some(layer) = &ovr.layer {
        resolved.layer = layer.clone();
    }
    if let Some(thickness) = ovr.thickness {
        resolved.thickness = thickness;
    }
    if let Some(bg_op) = ovr.background_opacity {
        resolved.background_opacity = bg_op;
    }
    if let Some(border) = &ovr.border {
        resolved.border = *border;
    }
    if let Some(border_width) = ovr.border_width {
        resolved.border_width = border_width;
    }
    if let Some(radius) = ovr.radius {
        resolved.radius = radius;
        resolved.radius_top_left = radius;
        resolved.radius_top_right = radius;
        resolved.radius_bottom_left = radius;
        resolved.radius_bottom_right = radius;
    }
    if let Some(r) = ovr.radius_top_left {
        resolved.radius_top_left = r;
    }
    if let Some(r) = ovr.radius_top_right {
        resolved.radius_top_right = r;
    }
    if let Some(r) = ovr.radius_bottom_left {
        resolved.radius_bottom_left = r;
    }
    if let Some(r) = ovr.radius_bottom_right {
        resolved.radius_bottom_right = r;
    }
    if let Some(concave) = ovr.concave_edge_corners {
        resolved.concave_edge_corners = concave;
    }
    if let Some(m) = ovr.margin_ends {
        resolved.margin_ends = m;
    }
    if let Some(m) = ovr.margin_edge {
        resolved.margin_edge = m;
    }
    if let Some(m) = ovr.margin_opposite_edge {
        resolved.margin_opposite_edge = m;
    }
    if let Some(p) = ovr.padding {
        resolved.padding = p;
    }
    if let Some(s) = ovr.widget_spacing {
        resolved.widget_spacing = s;
    }
    if let Some(s) = ovr.shadow {
        resolved.shadow = s;
    }
    if let Some(cs) = ovr.contact_shadow {
        resolved.contact_shadow = cs;
    }
    if let Some(po) = ovr.panel_overlap {
        resolved.panel_overlap = po;
    }
    if let Some(ct) = ovr.capsule_thickness {
        resolved.capsule_thickness = ct;
    }
    if let Some(ff) = &ovr.font_family {
        resolved.font_family = Some(ff.clone());
    }
    if let Some(sw) = &ovr.start_widgets {
        resolved.start_widgets = sw.clone();
    }
    if let Some(cw) = &ovr.center_widgets {
        resolved.center_widgets = cw.clone();
    }
    if let Some(ew) = &ovr.end_widgets {
        resolved.end_widgets = ew.clone();
    }
    if let Some(s) = ovr.scale {
        resolved.scale = s;
    }
    if let Some(d) = ovr.widget_capsule_default {
        resolved.widget_capsule_default = d;
    }
    if let Some(f) = &ovr.widget_capsule_fill {
        resolved.widget_capsule_fill = *f;
    }
    if ovr.widget_capsule_border_specified {
        resolved.widget_capsule_border_specified = true;
        resolved.widget_capsule_border = ovr.widget_capsule_border;
    }
    if let Some(fg) = &ovr.widget_capsule_foreground {
        resolved.widget_capsule_foreground = Some(*fg);
    }
    if let Some(c) = &ovr.widget_color {
        resolved.widget_color = Some(*c);
    }
    if let Some(ic) = &ovr.widget_icon_color {
        resolved.widget_icon_color = Some(*ic);
    }
    if let Some(groups) = &ovr.widget_capsule_groups {
        resolved.widget_capsule_groups = groups.clone();
    }
    if let Some(p) = ovr.widget_capsule_padding {
        resolved.widget_capsule_padding = p as f32;
    }
    if ovr.widget_capsule_radius.is_some() {
        resolved.widget_capsule_radius = ovr.widget_capsule_radius;
    }
    if let Some(op) = ovr.widget_capsule_opacity {
        resolved.widget_capsule_opacity = op as f32;
    }
    if let Some(hh) = ovr.hover_highlight {
        resolved.hover_highlight = hh;
    }
    if let Some(actions) = &ovr.dead_zone.actions {
        resolved.dead_zone.actions = actions.clone();
    }
    resolved
}

fn bar_config_table(bar: &BarConfig) -> toml::Table {
    let mut table = write_table(bar, bar_fields_schema());
    table.insert(
        "position".to_string(),
        toml::Value::String(bar.position.clone()),
    );

    if !bar.monitor_overrides.is_empty() {
        let mut monitors = toml::Table::new();
        for ovr in &bar.monitor_overrides {
            if ovr.match_.is_empty() {
                continue;
            }
            let resolved_bar = apply_monitor_override(bar, ovr);
            let mut monitor = write_table(&resolved_bar, bar_fields_schema());
            monitor.insert("match".to_string(), toml::Value::String(ovr.match_.clone()));
            if let Some(pos) = &ovr.position {
                monitor.insert("position".to_string(), toml::Value::String(pos.clone()));
            }
            monitors.insert(ovr.match_.clone(), toml::Value::Table(monitor));
        }
        table.insert("monitor".to_string(), toml::Value::Table(monitors));
    }
    table
}

fn widgets_placement_table(
    enabled: bool,
    schema_version: i32,
    grid: &DesktopWidgetsGridState,
    widgets: &[DesktopWidgetState],
) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert("enabled".to_string(), toml::Value::Boolean(enabled));
    table.insert(
        "schema_version".to_string(),
        toml::Value::Integer(schema_version as i64),
    );

    let mut grid_table = toml::Table::new();
    grid_table.insert("visible".to_string(), toml::Value::Boolean(grid.visible));
    grid_table.insert(
        "cell_size".to_string(),
        toml::Value::Integer(grid.cell_size as i64),
    );
    grid_table.insert(
        "major_interval".to_string(),
        toml::Value::Integer(grid.major_interval as i64),
    );
    table.insert("grid".to_string(), toml::Value::Table(grid_table));

    if !widgets.is_empty() {
        let mut order = Vec::new();
        let mut widget_table = toml::Table::new();
        for widget in widgets {
            if widget.id.is_empty() {
                continue;
            }
            order.push(toml::Value::String(widget.id.clone()));
            let mut item = toml::Table::new();
            item.insert(
                "type".to_string(),
                toml::Value::String(widget.type_.clone()),
            );
            item.insert(
                "output".to_string(),
                toml::Value::String(widget.output_name.clone()),
            );
            item.insert("cx".to_string(), toml::Value::Float(widget.cx as f64));
            item.insert("cy".to_string(), toml::Value::Float(widget.cy as f64));
            item.insert(
                "box_width".to_string(),
                toml::Value::Float(widget.box_width as f64),
            );
            item.insert(
                "box_height".to_string(),
                toml::Value::Float(widget.box_height as f64),
            );
            item.insert(
                "rotation".to_string(),
                toml::Value::Float(widget.rotation_rad as f64),
            );
            if widget.flip_x {
                item.insert("flip_x".to_string(), toml::Value::Boolean(true));
            }
            if widget.flip_y {
                item.insert("flip_y".to_string(), toml::Value::Boolean(true));
            }
            item.insert("enabled".to_string(), toml::Value::Boolean(widget.enabled));

            let mut settings = toml::Table::new();
            let mut keys: Vec<&String> = widget.settings.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(val) = widget.settings.get(key) {
                    insert_widget_setting_value(&mut settings, key, val);
                }
            }
            item.insert("settings".to_string(), toml::Value::Table(settings));
            widget_table.insert(widget.id.clone(), toml::Value::Table(item));
        }
        table.insert("widget_order".to_string(), toml::Value::Array(order));
        table.insert("widget".to_string(), toml::Value::Table(widget_table));
    }
    table
}

fn desktop_widgets_table(desktop_widgets: &DesktopWidgetsConfig) -> toml::Table {
    widgets_placement_table(
        desktop_widgets.enabled,
        desktop_widgets.schema_version,
        &desktop_widgets.grid,
        &desktop_widgets.widgets,
    )
}

/// Serializes a full `Config` into a `toml::Table`.
/// Port of `config_export::serialize` (`config_export.cpp:325`).
pub fn serialize(config: &Config) -> toml::Table {
    let mut root = toml::Table::new();

    for spec in sections() {
        root.insert(
            spec.name.to_string(),
            toml::Value::Table((spec.write)(config)),
        );
    }

    root.insert(
        "lockscreen_widgets".to_string(),
        toml::Value::Table(widgets_placement_table(
            config.lockscreen_widgets.enabled,
            config.lockscreen_widgets.schema_version,
            &config.lockscreen_widgets.grid,
            &config.lockscreen_widgets.widgets,
        )),
    );
    root.insert(
        "desktop_widgets".to_string(),
        toml::Value::Table(desktop_widgets_table(&config.desktop_widgets)),
    );

    let mut bar_root = toml::Table::new();
    let mut bar_order = Vec::new();
    for bar in &config.bars {
        if bar.name.is_empty() {
            continue;
        }
        bar_order.push(toml::Value::String(bar.name.clone()));
        bar_root.insert(bar.name.clone(), toml::Value::Table(bar_config_table(bar)));
    }
    bar_root.insert("order".to_string(), toml::Value::Array(bar_order));
    root.insert("bar".to_string(), toml::Value::Table(bar_root));

    let mut widget_root = toml::Table::new();
    let mut widget_names: Vec<&String> = config.widgets.keys().collect();
    widget_names.sort();
    for name in widget_names {
        if let Some(widget) = config.widgets.get(name) {
            widget_root.insert(
                name.clone(),
                toml::Value::Table(widget_config_table(widget)),
            );
        }
    }
    root.insert("widget".to_string(), toml::Value::Table(widget_root));
    root.insert(
        "plugin_settings".to_string(),
        toml::Value::Table(plugin_settings_table(&config.plugins)),
    );

    root
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::schema::diagnostics::Diagnostics;
    use crate::types::bar::{BarCapsuleGroupStyle, BarConfig, BarMonitorOverride};
    use crate::types::config::*;
    use noctalia_core::color::color_spec_from_config_string;

    fn make_probe_bar() -> BarConfig {
        let mut bar = BarConfig {
            name: "default".to_string(),
            position: "bottom".to_string(),
            enabled: false,
            auto_hide: true,
            smart_auto_hide: false,
            show_on_workspace_switch: true,
            reserve_space: false,
            layer: "overlay".to_string(),
            thickness: 44,
            background_opacity: 0.85,
            border: color_spec_from_config_string("#123456", "").unwrap(),
            border_width: 2.0,
            radius: 18,
            radius_top_left: 4,
            radius_top_right: 6,
            radius_bottom_left: 8,
            radius_bottom_right: 10,
            concave_edge_corners: true,
            margin_ends: 100,
            margin_edge: 5,
            margin_opposite_edge: 12,
            padding: 12,
            widget_spacing: 8,
            shadow: false,
            contact_shadow: true,
            panel_overlap: 2,
            capsule_thickness: 0.5,
            scale: 2.0,
            font_weight: 600,
            font_family: Some("Inter".to_string()),
            start_widgets: vec!["launcher".to_string()],
            center_widgets: vec!["clock".to_string(), "weather".to_string()],
            end_widgets: vec!["battery".to_string()],
            widget_capsule_default: true,
            widget_capsule_fill: color_spec_from_config_string("#abcdef", "").unwrap(),
            widget_capsule_foreground: Some(color_spec_from_config_string("#fedcba", "").unwrap()),
            widget_color: Some(color_spec_from_config_string("#0a0b0c", "").unwrap()),
            widget_icon_color: Some(color_spec_from_config_string("#0c0b0a", "").unwrap()),
            widget_capsule_padding: 16.0,
            widget_capsule_radius: Some(12.0),
            widget_capsule_opacity: 0.9,
            widget_capsule_border_specified: true,
            widget_capsule_border: Some(color_spec_from_config_string("#111213", "").unwrap()),
            hover_highlight: false,
            ..Default::default()
        };
        bar.actions.insert("middle".to_string(), "none".to_string());
        bar.actions
            .insert("right".to_string(), "media toggle".to_string());

        let mut dead_zone_actions = HashMap::new();
        dead_zone_actions.insert("left".to_string(), "exec notify-send bar-left".to_string());
        dead_zone_actions.insert(
            "right".to_string(),
            "exec notify-send bar-right".to_string(),
        );
        dead_zone_actions.insert(
            "middle".to_string(),
            "exec notify-send bar-middle".to_string(),
        );
        dead_zone_actions.insert(
            "scroll_up".to_string(),
            "exec notify-send bar-scroll-up".to_string(),
        );
        dead_zone_actions.insert(
            "scroll_down".to_string(),
            "exec notify-send bar-scroll-down".to_string(),
        );
        dead_zone_actions.insert("back".to_string(), "media previous".to_string());
        dead_zone_actions.insert("forward".to_string(), "media next".to_string());
        bar.dead_zone.actions = dead_zone_actions;

        let group = BarCapsuleGroupStyle {
            id: "grp1".to_string(),
            members: vec!["clock".to_string(), "weather".to_string()],
            fill: color_spec_from_config_string("#222324", "").unwrap(),
            border_specified: true,
            border: Some(color_spec_from_config_string("#333435", "").unwrap()),
            foreground: Some(color_spec_from_config_string("#444546", "").unwrap()),
            padding: 20.0,
            radius: Some(14.0),
            opacity: 0.8,
            ..Default::default()
        };
        bar.widget_capsule_groups = vec![group];

        let mut ovr_dead_zone = HashMap::new();
        ovr_dead_zone.insert("left".to_string(), "exec notify-send bar-left".to_string());
        ovr_dead_zone.insert(
            "right".to_string(),
            "exec notify-send bar-right".to_string(),
        );

        let ogroup = BarCapsuleGroupStyle {
            id: "ogrp".to_string(),
            members: vec!["volume".to_string()],
            fill: color_spec_from_config_string("#f1f2f3", "").unwrap(),
            border_specified: true,
            border: Some(color_spec_from_config_string("#0f0e0d", "").unwrap()),
            foreground: Some(color_spec_from_config_string("#0c0b0a", "").unwrap()),
            padding: 18.0,
            radius: Some(9.0),
            opacity: 0.6,
            ..Default::default()
        };

        let ovr = BarMonitorOverride {
            match_: "DP-1".to_string(),
            position: Some("top".to_string()),
            enabled: Some(true),
            auto_hide: Some(false),
            smart_auto_hide: Some(false),
            show_on_workspace_switch: Some(true),
            reserve_space: Some(true),
            layer: Some("top".to_string()),
            thickness: Some(50),
            background_opacity: Some(0.7),
            border: Some(color_spec_from_config_string("#a1a2a3", "").unwrap()),
            border_width: Some(3.0),
            radius: Some(22),
            radius_top_left: Some(1),
            radius_top_right: Some(2),
            radius_bottom_left: Some(3),
            radius_bottom_right: Some(4),
            concave_edge_corners: Some(false),
            margin_ends: Some(70),
            margin_edge: Some(9),
            margin_opposite_edge: Some(4),
            padding: Some(11),
            widget_spacing: Some(7),
            shadow: Some(true),
            contact_shadow: Some(false),
            panel_overlap: Some(-1),
            capsule_thickness: Some(0.25),
            scale: Some(1.5),
            font_family: Some("Fira Sans".to_string()),
            start_widgets: Some(vec!["tray".to_string()]),
            center_widgets: Some(vec!["media".to_string()]),
            end_widgets: Some(vec!["volume".to_string()]),
            widget_capsule_default: Some(false),
            widget_capsule_fill: Some(color_spec_from_config_string("#b1b2b3", "").unwrap()),
            widget_capsule_border_specified: true,
            widget_capsule_border: Some(color_spec_from_config_string("#c1c2c3", "").unwrap()),
            widget_capsule_foreground: Some(color_spec_from_config_string("#d1d2d3", "").unwrap()),
            widget_color: Some(color_spec_from_config_string("#e1e2e3", "").unwrap()),
            widget_icon_color: Some(color_spec_from_config_string("#e3e2e1", "").unwrap()),
            hover_highlight: Some(true),
            widget_capsule_groups: Some(vec![ogroup]),
            widget_capsule_padding: Some(24.0),
            widget_capsule_radius: Some(30.0),
            widget_capsule_opacity: Some(0.5),
            dead_zone: crate::types::bar::BarDeadZoneOverride {
                actions: Some(ovr_dead_zone),
            },
        };
        bar.monitor_overrides = vec![ovr];

        bar
    }

    fn make_probe() -> Config {
        Config {
            bars: vec![make_probe_bar()],
            ..Default::default()
        }
    }

    #[test]
    fn serialize_produces_table_for_all_sections() {
        let probe = make_probe();
        let table = serialize(&probe);

        assert!(table.contains_key("bar"));
        assert!(table.contains_key("widget"));
        assert!(table.contains_key("plugin_settings"));
        assert!(table.contains_key("desktop_widgets"));
        assert!(table.contains_key("lockscreen_widgets"));

        for spec in sections() {
            assert!(
                table.contains_key(spec.name),
                "export table missing section {}",
                spec.name
            );
        }
    }

    #[test]
    fn serialize_plugin_settings_string_map() {
        let mut probe = Config::default();
        let mut string_map = HashMap::new();
        string_map.insert("eDP-1".to_string(), "laptop".to_string());
        string_map.insert("DP-1".to_string(), "monitor".to_string());

        let mut settings = HashMap::new();
        settings.insert(
            "output_glyphs".to_string(),
            WidgetSettingValue::StringMap(string_map),
        );
        probe
            .plugins
            .plugin_settings
            .insert("me/display-output".to_string(), settings);

        let table = serialize(&probe);
        let plugin_settings = table
            .get("plugin_settings")
            .and_then(|v| v.as_table())
            .expect("plugin_settings is table");
        let display_output = plugin_settings
            .get("me/display-output")
            .and_then(|v| v.as_table())
            .expect("per-plugin table");
        let output_glyphs = display_output
            .get("output_glyphs")
            .and_then(|v| v.as_table())
            .expect("string map table");

        assert_eq!(
            output_glyphs.get("eDP-1").and_then(|v| v.as_str()),
            Some("laptop")
        );
        assert_eq!(
            output_glyphs.get("DP-1").and_then(|v| v.as_str()),
            Some("monitor")
        );
    }

    #[test]
    fn bar_roundtrip_via_schema() {
        let probe = make_probe();
        let table = serialize(&probe);
        let bar_table = table
            .get("bar")
            .and_then(|v| v.as_table())
            .and_then(|v| v.get("default"))
            .and_then(|v| v.as_table())
            .expect("bar default table");

        let mut rt = BarConfig {
            name: "default".to_string(),
            ..Default::default()
        };
        let mut diag = Diagnostics::default();
        if let Some(pos) = bar_table.get("position").and_then(|v| v.as_str()) {
            rt.position = pos.to_string();
        }
        crate::schema::engine::read_into(
            bar_table,
            &mut rt,
            bar_fields_schema(),
            "bar.default",
            &mut diag,
        );

        assert_eq!(rt.position, probe.bars[0].position);
        assert_eq!(rt.thickness, probe.bars[0].thickness);
        assert_eq!(rt.font_family, probe.bars[0].font_family);
        assert_eq!(rt.radius, probe.bars[0].radius);
    }
}
