//! Config change-set computation. Task 2.5.2: `computeConfigChangeSet` and the
//! equality helpers it and `configEqual` build on, from
//! `src/config/config_overrides.cpp:29-413,712-743`. Pure — operates only on
//! already-ported `Config` and its section types, no live `ConfigService`
//! needed. See MIGRATION_PLAN.md's 2.5 split for the rest of that file's scope
//! (`deep_merge` is 2.5.1, `merge_config_with_includes` is 2.5.3, the
//! remaining `ConfigService::*` override-CRUD methods are folded into 2.9).

use std::collections::HashMap;

use crate::types::bar::{BarConfig, BarMonitorOverride, WidgetConfig};
use crate::types::config::{Config, ConfigChangeSet};
use crate::types::desktop_widgets::{
    DesktopWidgetState, DesktopWidgetsConfig, LockscreenWidgetsConfig,
};
use crate::types::plugins::PluginsConfig;
use crate::types::widget_setting_value::WidgetSettingValue;

/// Port of `vectorEqual` (`config_overrides.cpp:43-54`).
fn vector_equal<T>(a: &[T], b: &[T], equal: impl Fn(&T, &T) -> bool) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| equal(x, y))
}

/// Port of `numericWidgetSetting` (`config_overrides.cpp:56-64`).
fn numeric_widget_setting(value: &WidgetSettingValue) -> Option<f64> {
    match value {
        WidgetSettingValue::Int(i) => Some(*i as f64),
        WidgetSettingValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// Port of `widgetSettingEqual` (`config_overrides.cpp:66-83`): int/double
/// values compare numerically-coerced (`Int(5)` == `Float(5.0)`); every other
/// pair compares by variant + exact value (which `WidgetSettingValue`'s own
/// derived `PartialEq` already does, since it requires the same variant).
fn widget_setting_equal(a: &WidgetSettingValue, b: &WidgetSettingValue) -> bool {
    let a_num = numeric_widget_setting(a);
    let b_num = numeric_widget_setting(b);
    if a_num.is_some() || b_num.is_some() {
        return a_num.is_some() && b_num.is_some() && a_num == b_num;
    }
    a == b
}

/// Port of `widgetSettingsEqual` (`config_overrides.cpp:85-99`).
fn widget_settings_equal(
    a: &HashMap<String, WidgetSettingValue>,
    b: &HashMap<String, WidgetSettingValue>,
) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, v)| b.get(k).is_some_and(|bv| widget_setting_equal(v, bv)))
}

/// Port of `pluginsConfigEqual` (`config_overrides.cpp:103-117`): compares the
/// open-ended `plugin_settings` map with int/double coercion
/// (`widget_settings_equal`) instead of the derived `PartialEq`, same reason
/// as widgets.
fn plugins_config_equal(a: &PluginsConfig, b: &PluginsConfig) -> bool {
    if a.sources != b.sources
        || a.enabled != b.enabled
        || a.auto_update != b.auto_update
        || a.plugin_settings.len() != b.plugin_settings.len()
    {
        return false;
    }
    a.plugin_settings.iter().all(|(id, a_map)| {
        b.plugin_settings
            .get(id)
            .is_some_and(|b_map| widget_settings_equal(a_map, b_map))
    })
}

/// Port of `desktopWidgetEqual` (`config_overrides.cpp:121-134`): like
/// `DesktopWidgetState`'s derived `PartialEq`, but compares `settings` with
/// int/double coercion instead of exact variant equality.
fn desktop_widget_equal(a: &DesktopWidgetState, b: &DesktopWidgetState) -> bool {
    a.id == b.id
        && a.type_ == b.type_
        && a.output_name == b.output_name
        && a.cx == b.cx
        && a.cy == b.cy
        && a.box_width == b.box_width
        && a.box_height == b.box_height
        && a.rotation_rad == b.rotation_rad
        && a.flip_x == b.flip_x
        && a.flip_y == b.flip_y
        && a.enabled == b.enabled
        && widget_settings_equal(&a.settings, &b.settings)
}

/// Port of `desktopWidgetsConfigEqual` (`config_overrides.cpp:136-141`).
fn desktop_widgets_config_equal(a: &DesktopWidgetsConfig, b: &DesktopWidgetsConfig) -> bool {
    a.enabled == b.enabled
        && a.schema_version == b.schema_version
        && a.grid == b.grid
        && vector_equal(&a.widgets, &b.widgets, desktop_widget_equal)
}

/// Port of `lockscreenWidgetsConfigEqual` (`config_overrides.cpp:143-148`).
fn lockscreen_widgets_config_equal(
    a: &LockscreenWidgetsConfig,
    b: &LockscreenWidgetsConfig,
) -> bool {
    a.enabled == b.enabled
        && a.schema_version == b.schema_version
        && a.grid == b.grid
        && vector_equal(&a.widgets, &b.widgets, desktop_widget_equal)
}

/// Port of `barBaseConfigEqual` (`config_overrides.cpp:150-159`): compares two
/// bars ignoring their monitor-override lists (those are resolved + compared
/// separately by `bar_config_equal`). `BarConfig`'s derived `PartialEq` covers
/// every other field, so new bar fields participate automatically.
fn bar_base_config_equal(a: &BarConfig, b: &BarConfig) -> bool {
    let mut aa = a.clone();
    let mut bb = b.clone();
    aa.monitor_overrides.clear();
    bb.monitor_overrides.clear();
    aa == bb
}

/// Port of `applyMonitorOverrideForComparison` (`config_overrides.cpp:161-296`).
///
/// Divergence note (verified, not silently introduced): the C++ function
/// never applies `ovr.layer` onto `resolved.layer` — `grep -i layer
/// config_overrides.cpp` finds zero hits — even though both `BarMonitorOverride::layer`
/// and `BarConfig::layer` exist. So two monitor overrides differing only in
/// `layer` compare as identical for override-effectiveness purposes in the
/// C++, and this port matches that (not "improving" it per the ground rules;
/// if the omission is itself a C++ bug, that's a separate, explicitly-recorded
/// decision to fix on both sides, not something to silently diverge on here).
fn apply_monitor_override_for_comparison(base: &BarConfig, ovr: &BarMonitorOverride) -> BarConfig {
    let mut resolved = base.clone();
    resolved.monitor_overrides.clear();
    if let Some(ref v) = ovr.position {
        resolved.position = v.clone();
    }
    if let Some(v) = ovr.enabled {
        resolved.enabled = v;
    }
    if let Some(v) = ovr.auto_hide {
        resolved.auto_hide = v;
    }
    if let Some(v) = ovr.smart_auto_hide {
        resolved.smart_auto_hide = v;
    }
    if let Some(v) = ovr.show_on_workspace_switch {
        resolved.show_on_workspace_switch = v;
    }
    if let Some(v) = ovr.reserve_space {
        resolved.reserve_space = v;
    }
    if let Some(v) = ovr.thickness {
        resolved.thickness = v;
    }
    if let Some(v) = ovr.background_opacity {
        resolved.background_opacity = v;
    }
    if let Some(v) = ovr.border {
        resolved.border = v;
    }
    if let Some(v) = ovr.border_width {
        resolved.border_width = v;
    }
    if let Some(v) = ovr.radius {
        resolved.radius = v;
        resolved.radius_top_left = v;
        resolved.radius_top_right = v;
        resolved.radius_bottom_left = v;
        resolved.radius_bottom_right = v;
    }
    if let Some(v) = ovr.radius_top_left {
        resolved.radius_top_left = v;
    }
    if let Some(v) = ovr.radius_top_right {
        resolved.radius_top_right = v;
    }
    if let Some(v) = ovr.radius_bottom_left {
        resolved.radius_bottom_left = v;
    }
    if let Some(v) = ovr.radius_bottom_right {
        resolved.radius_bottom_right = v;
    }
    if let Some(v) = ovr.concave_edge_corners {
        resolved.concave_edge_corners = v;
    }
    if let Some(v) = ovr.margin_ends {
        resolved.margin_ends = v;
    }
    if let Some(v) = ovr.margin_edge {
        resolved.margin_edge = v;
    }
    if let Some(v) = ovr.margin_opposite_edge {
        resolved.margin_opposite_edge = v;
    }
    if let Some(v) = ovr.padding {
        resolved.padding = v;
    }
    if let Some(v) = ovr.widget_spacing {
        resolved.widget_spacing = v;
    }
    if let Some(v) = ovr.shadow {
        resolved.shadow = v;
    }
    if let Some(v) = ovr.contact_shadow {
        resolved.contact_shadow = v;
    }
    if let Some(v) = ovr.panel_overlap {
        resolved.panel_overlap = v;
    }
    if let Some(v) = ovr.capsule_thickness {
        resolved.capsule_thickness = v;
    }
    if let Some(ref v) = ovr.font_family {
        resolved.font_family = Some(v.clone());
    }
    if let Some(ref v) = ovr.scale {
        resolved.scale = *v;
    }
    if let Some(ref v) = ovr.start_widgets {
        resolved.start_widgets = v.clone();
    }
    if let Some(ref v) = ovr.center_widgets {
        resolved.center_widgets = v.clone();
    }
    if let Some(ref v) = ovr.end_widgets {
        resolved.end_widgets = v.clone();
    }
    if let Some(v) = ovr.widget_capsule_default {
        resolved.widget_capsule_default = v;
    }
    if let Some(v) = ovr.widget_capsule_fill {
        resolved.widget_capsule_fill = v;
    }
    if let Some(ref v) = ovr.widget_capsule_foreground {
        resolved.widget_capsule_foreground = Some(*v);
    }
    if let Some(ref v) = ovr.widget_color {
        resolved.widget_color = Some(*v);
    }
    if let Some(ref v) = ovr.widget_icon_color {
        resolved.widget_icon_color = Some(*v);
    }
    if let Some(ref v) = ovr.widget_capsule_groups {
        resolved.widget_capsule_groups = v.clone();
    }
    if let Some(v) = ovr.widget_capsule_padding {
        resolved.widget_capsule_padding = (v as f32).clamp(0.0, 48.0);
    }
    if let Some(v) = ovr.widget_capsule_radius {
        resolved.widget_capsule_radius = Some(v.clamp(0.0, 80.0));
    }
    if let Some(v) = ovr.widget_capsule_opacity {
        resolved.widget_capsule_opacity = (v as f32).clamp(0.0, 1.0);
    }
    if ovr.widget_capsule_border_specified {
        resolved.widget_capsule_border_specified = true;
        resolved.widget_capsule_border = ovr.widget_capsule_border;
    }
    if let Some(v) = ovr.hover_highlight {
        resolved.hover_highlight = v;
    }
    if let Some(ref v) = ovr.dead_zone.actions {
        resolved.dead_zone.actions = v.clone();
    }
    resolved
}

/// Port of `barMonitorOverrideEqual` (`config_overrides.cpp:298-301`).
fn bar_monitor_override_equal(
    base: &BarConfig,
    a: &BarMonitorOverride,
    b: &BarMonitorOverride,
) -> bool {
    a.match_ == b.match_
        && bar_base_config_equal(
            &apply_monitor_override_for_comparison(base, a),
            &apply_monitor_override_for_comparison(base, b),
        )
}

/// Port of `barConfigEqual` (`config_overrides.cpp:303-311`).
fn bar_config_equal(a: &BarConfig, b: &BarConfig) -> bool {
    bar_base_config_equal(a, b)
        && vector_equal(&a.monitor_overrides, &b.monitor_overrides, |lhs, rhs| {
            bar_monitor_override_equal(a, lhs, rhs)
        })
}

/// Port of `widgetConfigEqual` (`config_overrides.cpp:313-315`).
fn widget_config_equal(a: &WidgetConfig, b: &WidgetConfig) -> bool {
    a.r#type == b.r#type && widget_settings_equal(&a.settings, &b.settings) && a.tables == b.tables
}

/// Port of `widgetMapEqual` (`config_overrides.cpp:317-330`).
fn widget_map_equal(a: &HashMap<String, WidgetConfig>, b: &HashMap<String, WidgetConfig>) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, v)| b.get(k).is_some_and(|bv| widget_config_equal(v, bv)))
}

/// Override-effectiveness equality. Every config section uses its derived
/// `PartialEq` (exact member-wise compare) so that adding a field cannot
/// silently break override persistence — the only exceptions are the
/// sections whose comparison carries semantics `PartialEq` can't express:
/// bars (monitor overrides resolved + clamped before comparing), widgets /
/// desktop widgets (settings compared with int/double coercion).
/// Port of `configEqual` (`config_overrides.cpp:346-374`).
#[must_use]
pub fn config_equal(a: &Config, b: &Config) -> bool {
    vector_equal(&a.bars, &b.bars, bar_config_equal)
        && widget_map_equal(&a.widgets, &b.widgets)
        && desktop_widgets_config_equal(&a.desktop_widgets, &b.desktop_widgets)
        && a.hot_corners == b.hot_corners
        && lockscreen_widgets_config_equal(&a.lockscreen_widgets, &b.lockscreen_widgets)
        && a.wallpaper == b.wallpaper
        && a.backdrop == b.backdrop
        && a.lockscreen == b.lockscreen
        && a.dock == b.dock
        && a.shell == b.shell
        && a.osd == b.osd
        && a.notification == b.notification
        && a.weather == b.weather
        && a.calendar == b.calendar
        && a.system == b.system
        && a.audio == b.audio
        && a.brightness == b.brightness
        && a.battery == b.battery
        && a.keybinds == b.keybinds
        && a.nightlight == b.nightlight
        && a.location == b.location
        && a.idle == b.idle
        && a.hooks == b.hooks
        && a.theme == b.theme
        && a.accessibility == b.accessibility
        && a.control_center == b.control_center
        && plugins_config_equal(&a.plugins, &b.plugins)
}

/// Port of `computeConfigChangeSet` (`config_overrides.cpp:712-743`).
#[must_use]
pub fn compute_config_change_set(prev: &Config, next: &Config) -> ConfigChangeSet {
    ConfigChangeSet {
        bars: !vector_equal(&prev.bars, &next.bars, bar_config_equal),
        widgets: !widget_map_equal(&prev.widgets, &next.widgets),
        desktop_widgets: !desktop_widgets_config_equal(
            &prev.desktop_widgets,
            &next.desktop_widgets,
        ),
        lockscreen_widgets: !lockscreen_widgets_config_equal(
            &prev.lockscreen_widgets,
            &next.lockscreen_widgets,
        ),
        wallpaper: prev.wallpaper != next.wallpaper,
        backdrop: prev.backdrop != next.backdrop,
        lockscreen: prev.lockscreen != next.lockscreen,
        dock: prev.dock != next.dock,
        shell: prev.shell != next.shell,
        osd: prev.osd != next.osd,
        notification: prev.notification != next.notification,
        weather: prev.weather != next.weather,
        calendar: prev.calendar != next.calendar,
        system: prev.system != next.system,
        audio: prev.audio != next.audio,
        brightness: prev.brightness != next.brightness,
        battery: prev.battery != next.battery,
        keybinds: prev.keybinds != next.keybinds,
        nightlight: prev.nightlight != next.nightlight,
        location: prev.location != next.location,
        idle: prev.idle != next.idle,
        hooks: prev.hooks != next.hooks,
        theme: prev.theme != next.theme,
        control_center: prev.control_center != next.control_center,
        plugins: !plugins_config_equal(&prev.plugins, &next.plugins),
        hot_corners: prev.hot_corners != next.hot_corners,
        storage: prev.storage != next.storage,
        accessibility: prev.accessibility != next.accessibility,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::bar::BarDeadZoneOverride;

    #[test]
    fn widget_setting_equal_coerces_int_and_float() {
        assert!(widget_setting_equal(
            &WidgetSettingValue::Int(5),
            &WidgetSettingValue::Float(5.0)
        ));
        assert!(!widget_setting_equal(
            &WidgetSettingValue::Int(5),
            &WidgetSettingValue::Float(5.5)
        ));
    }

    #[test]
    fn widget_setting_equal_rejects_mismatched_non_numeric_variants() {
        assert!(!widget_setting_equal(
            &WidgetSettingValue::Bool(true),
            &WidgetSettingValue::String("true".to_string())
        ));
    }

    #[test]
    fn widget_setting_equal_string_variant_exact_match() {
        assert!(widget_setting_equal(
            &WidgetSettingValue::String("a".to_string()),
            &WidgetSettingValue::String("a".to_string())
        ));
        assert!(!widget_setting_equal(
            &WidgetSettingValue::String("a".to_string()),
            &WidgetSettingValue::String("b".to_string())
        ));
    }

    #[test]
    fn widget_settings_equal_size_mismatch_is_unequal() {
        let a = HashMap::from([("x".to_string(), WidgetSettingValue::Bool(true))]);
        let b = HashMap::new();
        assert!(!widget_settings_equal(&a, &b));
    }

    #[test]
    fn plugins_config_equal_coerces_plugin_setting_numbers() {
        let mut a = PluginsConfig::default();
        let mut b = PluginsConfig::default();
        a.plugin_settings.insert(
            "author/plugin".to_string(),
            HashMap::from([("threshold".to_string(), WidgetSettingValue::Int(5))]),
        );
        b.plugin_settings.insert(
            "author/plugin".to_string(),
            HashMap::from([("threshold".to_string(), WidgetSettingValue::Float(5.0))]),
        );
        assert!(plugins_config_equal(&a, &b));
    }

    #[test]
    fn plugins_config_equal_detects_source_list_change() {
        let a = PluginsConfig::default();
        let mut b = PluginsConfig::default();
        b.enabled.push("author/plugin".to_string());
        assert!(!plugins_config_equal(&a, &b));
    }

    #[test]
    fn desktop_widget_equal_coerces_setting_numbers() {
        let mut a = DesktopWidgetState::default();
        let mut b = DesktopWidgetState::default();
        a.settings
            .insert("scale".to_string(), WidgetSettingValue::Int(2));
        b.settings
            .insert("scale".to_string(), WidgetSettingValue::Float(2.0));
        assert!(desktop_widget_equal(&a, &b));
    }

    #[test]
    fn bar_base_config_equal_ignores_monitor_overrides() {
        let mut a = BarConfig::default();
        let mut b = BarConfig::default();
        a.monitor_overrides.push(BarMonitorOverride::default());
        assert!(bar_base_config_equal(&a, &b));
        b.thickness = a.thickness + 1;
        assert!(!bar_base_config_equal(&a, &b));
    }

    #[test]
    fn apply_monitor_override_resolves_thickness_and_radius_group() {
        let base = BarConfig::default();
        let ovr = BarMonitorOverride {
            thickness: Some(99),
            radius: Some(5),
            ..Default::default()
        };
        let resolved = apply_monitor_override_for_comparison(&base, &ovr);
        assert_eq!(resolved.thickness, 99);
        assert_eq!(resolved.radius, 5);
        assert_eq!(resolved.radius_top_left, 5);
        assert_eq!(resolved.radius_top_right, 5);
        assert_eq!(resolved.radius_bottom_left, 5);
        assert_eq!(resolved.radius_bottom_right, 5);
        assert!(resolved.monitor_overrides.is_empty());
    }

    #[test]
    fn apply_monitor_override_individual_corner_wins_over_group_radius() {
        let base = BarConfig::default();
        let ovr = BarMonitorOverride {
            radius: Some(5),
            radius_top_left: Some(9),
            ..Default::default()
        };
        let resolved = apply_monitor_override_for_comparison(&base, &ovr);
        assert_eq!(resolved.radius_top_left, 9);
        assert_eq!(resolved.radius_top_right, 5);
    }

    #[test]
    fn apply_monitor_override_clamps_capsule_padding_radius_opacity() {
        let base = BarConfig::default();
        let ovr = BarMonitorOverride {
            widget_capsule_padding: Some(999.0),
            widget_capsule_radius: Some(999.0),
            widget_capsule_opacity: Some(999.0),
            ..Default::default()
        };
        let resolved = apply_monitor_override_for_comparison(&base, &ovr);
        assert_eq!(resolved.widget_capsule_padding, 48.0);
        assert_eq!(resolved.widget_capsule_radius, Some(80.0));
        assert_eq!(resolved.widget_capsule_opacity, 1.0);
    }

    #[test]
    fn apply_monitor_override_dead_zone_actions_replace_wholesale() {
        let base = BarConfig::default();
        let ovr = BarMonitorOverride {
            dead_zone: BarDeadZoneOverride {
                actions: Some(HashMap::from([(
                    "swipe_up".to_string(),
                    "toggle".to_string(),
                )])),
            },
            ..Default::default()
        };
        let resolved = apply_monitor_override_for_comparison(&base, &ovr);
        assert_eq!(
            resolved
                .dead_zone
                .actions
                .get("swipe_up")
                .map(String::as_str),
            Some("toggle")
        );
    }

    #[test]
    fn bar_monitor_override_equal_compares_resolved_bars_not_raw_overrides() {
        let base = BarConfig::default();
        // Two differently-shaped overrides that resolve to the same effective bar.
        let a = BarMonitorOverride {
            match_: "DP-1".to_string(),
            thickness: Some(40),
            ..Default::default()
        };
        let b = BarMonitorOverride {
            match_: "DP-1".to_string(),
            thickness: Some(40),
            enabled: Some(base.enabled),
            ..Default::default()
        };
        assert!(bar_monitor_override_equal(&base, &a, &b));
    }

    #[test]
    fn bar_monitor_override_equal_detects_match_difference() {
        let base = BarConfig::default();
        let a = BarMonitorOverride {
            match_: "DP-1".to_string(),
            ..Default::default()
        };
        let b = BarMonitorOverride {
            match_: "DP-2".to_string(),
            ..Default::default()
        };
        assert!(!bar_monitor_override_equal(&base, &a, &b));
    }

    #[test]
    fn apply_monitor_override_never_applies_layer_matching_the_cpp_omission() {
        // Regression test (caught by fresh-context review): a first draft of
        // this port added `layer` handling that the real C++
        // `applyMonitorOverrideForComparison` (config_overrides.cpp:161-296)
        // does not have — confirmed via `grep -i layer config_overrides.cpp`
        // finding zero hits. `resolved.layer` must stay at the base bar's
        // value regardless of what the override specifies.
        let base = BarConfig {
            layer: "top".to_string(),
            ..Default::default()
        };
        let ovr = BarMonitorOverride {
            layer: Some("overlay".to_string()),
            ..Default::default()
        };
        let resolved = apply_monitor_override_for_comparison(&base, &ovr);
        assert_eq!(resolved.layer, "top");
    }

    #[test]
    fn bar_config_equal_true_for_identical_bars() {
        let a = BarConfig::default();
        let b = BarConfig::default();
        assert!(bar_config_equal(&a, &b));
    }

    #[test]
    fn compute_config_change_set_flags_only_the_changed_section() {
        let prev = Config::default();
        let mut next = Config::default();
        next.audio.sound_volume = prev.audio.sound_volume + 0.1;
        let cs = compute_config_change_set(&prev, &next);
        assert!(cs.audio);
        assert!(!cs.shell);
        assert!(!cs.bars);
        assert!(!cs.widgets);
    }

    #[test]
    fn compute_config_change_set_bars_flag_uses_bar_config_equal() {
        let mut prev = Config::default();
        let mut next = Config::default();
        let base = BarConfig::default();
        let mut prev_bar = base.clone();
        prev_bar.monitor_overrides.push(BarMonitorOverride {
            match_: "DP-1".to_string(),
            ..Default::default()
        });
        prev.bars.push(prev_bar);
        let mut next_bar = base.clone();
        next_bar.monitor_overrides.push(BarMonitorOverride {
            match_: "DP-1".to_string(),
            // Explicitly setting a field to the value it would already
            // resolve to shouldn't count as a change (per bar_config_equal's
            // resolved-comparison semantics), unlike a raw field-by-field diff.
            enabled: Some(base.enabled),
            ..Default::default()
        });
        next.bars.push(next_bar);
        let cs = compute_config_change_set(&prev, &next);
        assert!(!cs.bars);
    }

    #[test]
    fn compute_config_change_set_plugins_flag_ignores_numeric_coercion_noise() {
        let mut prev = Config::default();
        let mut next = Config::default();
        prev.plugins.plugin_settings.insert(
            "author/plugin".to_string(),
            HashMap::from([("threshold".to_string(), WidgetSettingValue::Int(5))]),
        );
        next.plugins.plugin_settings.insert(
            "author/plugin".to_string(),
            HashMap::from([("threshold".to_string(), WidgetSettingValue::Float(5.0))]),
        );
        let cs = compute_config_change_set(&prev, &next);
        assert!(!cs.plugins);
    }

    #[test]
    fn config_equal_matches_compute_config_change_set_any() {
        let prev = Config::default();
        let mut next = Config::default();
        assert!(config_equal(&prev, &next));
        next.theme.builtin_palette = "Nord".to_string();
        assert!(!config_equal(&prev, &next));
        assert!(compute_config_change_set(&prev, &next).theme);
    }
}
