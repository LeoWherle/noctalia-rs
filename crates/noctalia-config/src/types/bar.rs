//! Task 2.1.2 — bar & widget settings. Port of the `BarCapsuleGroupStyle`/
//! `BarDeadZoneOverride`/`BarMonitorOverride`/`BarDeadZoneConfig`/`BarConfig`,
//! `WidgetBarCapsuleSpec`, `WidgetConfig` (+ accessors), and the capsule-group
//! reconciliation helpers from `src/config/config_types.{h,cpp}` (structs:
//! lines 26-419; bodies: config_types.cpp:245-579's `WidgetConfig::*`,
//! `resolveWidgetBarCapsuleSpec`, `findBarCapsuleGroupStyle`,
//! `capsuleGroupRefsFor{Bar,Monitor}Scope`, `reconcileCapsuleGroups`,
//! `capsuleSpecFromGroup`, `{is,make}CapsuleGroupToken`/`capsuleGroupTokenId`,
//! `resolveWidgetContentScale`, `outputMatchesSelector`).
//!
//! Scope boundary (deliberate, matches this task's plan text): this module
//! covers the **data model** only — struct shapes, in-code defaults matching
//! the C++ member initializers, and the pure helper functions above. It does
//! **not** reproduce `src/config/schema/config_schema.cpp`'s declarative
//! field-mapping engine, which additionally does range clamping (e.g.
//! `kBarThicknessRange`), warn-diagnostics on bad enum strings
//! (`layer`'s top/overlay check), and array keep-predicates (dropping
//! empty-id capsule groups on write) — that engine is task 2.3 (Schema)'s
//! job. The `serde(rename = ...)` keys below were cross-checked against
//! `config_schema.cpp`'s `field(&Struct::member, "toml_key")` registrations
//! (the authoritative source — NOT `example.toml`'s comments) so task 2.3
//! can reuse these struct shapes as its `Deserialize` target rather than
//! re-deriving the key names independently.
//!
//! Every struct here derives `Deserialize`/`Serialize` with a container-level
//! `#[serde(default)]`, backed by a manual `impl Default` matching the C++
//! in-class member initializers exactly — this makes `toml::from_str`
//! succeed on a partial table (as real config files always are) without
//! needing a `default = "fn"` attribute on every single field.
//!
//! `WidgetConfig` and `WidgetBarCapsuleSpec` are the two exceptions: neither
//! derives `Deserialize`. `WidgetConfig::settings`'s value type varies
//! per-key (`WidgetSettingValue`), which is populated by task 2.2's
//! `readBarWidgetConfig`/`readWidgetSettingValue` (explicit code, not
//! `serde`) — not by this crate's derive machinery. `WidgetBarCapsuleSpec` is
//! a computed *output* type (`resolveWidgetBarCapsuleSpec`'s return value),
//! never itself read from a config file.
//!
//! `BarConfig::name` and `BarConfig::monitor_overrides` are excluded from the
//! per-field TOML mapping here (`#[serde(skip)]`): in the C++, `name` comes
//! from special-cased handling around the `[bar.<name>]` table-name key
//! itself (`config_schema.cpp:1694`, outside the generic field loop, since
//! it's the map key, not a value inside the table), and `monitorOverrides`
//! from a separate `[bar.<name>.monitor.<id>]` table-map (mirroring the
//! top-level `[bar.<name>]` -> `Config::bars` map-to-vec assembly) — both are
//! whole-config assembly concerns for a later task (2.1.9 / 2.9), not this
//! struct's own derive. `position` is *not* skipped here despite also being
//! handled outside `config_schema.cpp`'s generic field loop
//! (`config_schema.cpp:1694` groups it with `name`) — unlike `name`, its
//! value genuinely lives at the ordinary `position` TOML key inside
//! `[bar.<name>]` (see `example.toml`'s `position = "top"`), so plain derive
//! parsing already does the right thing; whatever the C++ does specially
//! with it (likely UI-facing bookkeeping) doesn't change where the value
//! comes from.

use std::collections::HashMap;
use std::collections::HashSet;

use noctalia_core::color::{ColorRole, ColorSpec, color_spec_from_role};
use serde::{Deserialize, Serialize};

use crate::types::serde_support::{color_spec_serde, optional_color_spec_serde};
use crate::types::widget_setting_value::{WidgetSettingValue, widget_setting_value_as};

/// Placeholder mirrors of the handful of `ui/style.h` constants this module's
/// defaults need. `ui/style.h` itself has no assigned migration task yet
/// (not referenced anywhere in MIGRATION_PLAN.md) — replace these with a
/// real `ui::style` port when one lands; until then keep them in sync by
/// hand (same "placeholder until the real thing lands" pattern as task 1.7's
/// `INSTALL_PREFIX`/`INSTALL_DATADIR`).
mod style_defaults {
    /// `Style::barThicknessDefault` (ui/style.h:7).
    pub const BAR_THICKNESS_DEFAULT: i32 = 34;
    /// `Style::radiusXl` (ui/style.h:16).
    pub const RADIUS_XL: i32 = 12;
    /// `Style::barCapsulePadding` (ui/style.h:32).
    pub const BAR_CAPSULE_PADDING: f32 = 6.0;
}

/// C's `isspace` byte set (space/\t/\n/\v/\f/\r) — matches
/// `StringUtils::trimLeftView`/`trimRightView`'s `std::isspace` check
/// (string_utils.h:20-32). Small private duplicate of
/// `noctalia_core::color::rgb`'s `is_c_isspace`/`trim_c_whitespace` (which
/// are `pub(crate)` to that crate); a shared `core::string_utils` port would
/// obsolete both copies, same as task 1.6.5's local `generate_uuid_v4`
/// pending `string_utils.h`.
fn is_c_isspace_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

fn trim_c_whitespace(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_ascii() && is_c_isspace_byte(c as u8))
}

/// A capsule group: an ordered set of member widgets sharing one capsule +
/// style. Port of `BarCapsuleGroupStyle` (config_types.h:26-40).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarCapsuleGroupStyle {
    pub id: String,
    /// Ordered member widget references.
    pub members: Vec<String>,
    pub enabled: bool,
    #[serde(with = "color_spec_serde")]
    pub fill: ColorSpec,
    /// True when `border` is explicitly present (empty value = no outline);
    /// mirrors bar/widget border semantics. Not itself a TOML key — the
    /// schema engine's `capsuleBorderField` (task 2.3) derives it from
    /// whether the `border` key was present at all, which a plain
    /// `Option<ColorSpec>` can't distinguish from "absent" on its own.
    #[serde(skip)]
    pub border_specified: bool,
    #[serde(default, with = "optional_color_spec_serde")]
    pub border: Option<ColorSpec>,
    #[serde(default, with = "optional_color_spec_serde")]
    pub foreground: Option<ColorSpec>,
    pub padding: f32,
    pub radius: Option<f32>,
    pub opacity: f32,
}

impl Default for BarCapsuleGroupStyle {
    fn default() -> Self {
        Self {
            id: String::new(),
            members: Vec::new(),
            enabled: true,
            fill: color_spec_from_role(ColorRole::SurfaceVariant, 1.0),
            border_specified: false,
            border: None,
            foreground: None,
            padding: style_defaults::BAR_CAPSULE_PADDING,
            radius: None,
            opacity: 1.0,
        }
    }
}

/// The literal "group:" prefix + a group id. Port of
/// `kCapsuleGroupTokenPrefix` (config_types.h:44).
pub const CAPSULE_GROUP_TOKEN_PREFIX: &str = "group:";

/// Port of `isCapsuleGroupToken` (config_types.cpp:483).
pub fn is_capsule_group_token(lane_entry: &str) -> bool {
    lane_entry.starts_with(CAPSULE_GROUP_TOKEN_PREFIX)
}

/// Port of `capsuleGroupTokenId` (config_types.cpp:485-490).
pub fn capsule_group_token_id(lane_entry: &str) -> String {
    if !is_capsule_group_token(lane_entry) {
        return String::new();
    }
    lane_entry[CAPSULE_GROUP_TOKEN_PREFIX.len()..].to_string()
}

/// Port of `makeCapsuleGroupToken` (config_types.cpp:492-494).
pub fn make_capsule_group_token(group_id: &str) -> String {
    format!("{CAPSULE_GROUP_TOKEN_PREFIX}{group_id}")
}

/// Port of `BarDeadZoneOverride` (config_types.h:49-53).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarDeadZoneOverride {
    pub actions: Option<HashMap<String, String>>,
}

/// Port of `BarDeadZoneConfig` (config_types.h:109-115).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarDeadZoneConfig {
    /// Gesture -> action bindings for the parts of the bar no widget covers.
    pub actions: HashMap<String, String>,
}

/// Port of `BarMonitorOverride` (config_types.h:55-107). Every field's C++
/// default is `std::nullopt`/`false`/empty, so `#[derive(Default)]` already
/// matches — no manual `impl Default` needed here (unlike `BarConfig`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarMonitorOverride {
    #[serde(rename = "match")]
    pub match_: String,
    pub position: Option<String>,
    pub enabled: Option<bool>,
    pub auto_hide: Option<bool>,
    pub smart_auto_hide: Option<bool>,
    pub show_on_workspace_switch: Option<bool>,
    pub reserve_space: Option<bool>,
    /// "top" | "overlay"; string-vs-enum validation is schema-engine territory (task 2.3).
    pub layer: Option<String>,
    pub thickness: Option<i32>,
    pub background_opacity: Option<f32>,
    #[serde(with = "optional_color_spec_serde")]
    pub border: Option<ColorSpec>,
    pub border_width: Option<f32>,
    pub radius: Option<i32>,
    pub radius_top_left: Option<i32>,
    pub radius_top_right: Option<i32>,
    pub radius_bottom_left: Option<i32>,
    pub radius_bottom_right: Option<i32>,
    pub concave_edge_corners: Option<bool>,
    /// Inset from each end of the bar along its main axis.
    pub margin_ends: Option<i32>,
    /// Distance from the nearest screen edge (floats the bar when > 0).
    pub margin_edge: Option<i32>,
    /// Extra reserved space on the inward side of the bar.
    pub margin_opposite_edge: Option<i32>,
    pub padding: Option<i32>,
    pub widget_spacing: Option<i32>,
    pub shadow: Option<bool>,
    pub contact_shadow: Option<bool>,
    pub panel_overlap: Option<i32>,
    pub capsule_thickness: Option<f32>,
    pub font_family: Option<String>,
    pub scale: Option<f32>,
    #[serde(rename = "start")]
    pub start_widgets: Option<Vec<String>>,
    #[serde(rename = "center")]
    pub center_widgets: Option<Vec<String>>,
    #[serde(rename = "end")]
    pub end_widgets: Option<Vec<String>>,
    #[serde(rename = "capsule")]
    pub widget_capsule_default: Option<bool>,
    #[serde(rename = "capsule_fill", with = "optional_color_spec_serde")]
    pub widget_capsule_fill: Option<ColorSpec>,
    #[serde(rename = "capsule_foreground", with = "optional_color_spec_serde")]
    pub widget_capsule_foreground: Option<ColorSpec>,
    #[serde(rename = "color", with = "optional_color_spec_serde")]
    pub widget_color: Option<ColorSpec>,
    #[serde(rename = "icon_color", with = "optional_color_spec_serde")]
    pub widget_icon_color: Option<ColorSpec>,
    /// Read-only here in the C++ (`capsule_group` never serializes back for a
    /// monitor override — it re-derives from the resolved bar instead,
    /// config_schema.cpp:2249); this port doesn't reproduce that write-side
    /// asymmetry (export shape is task 2.7's job).
    #[serde(rename = "capsule_group")]
    pub widget_capsule_groups: Option<Vec<BarCapsuleGroupStyle>>,
    #[serde(rename = "capsule_padding")]
    pub widget_capsule_padding: Option<f64>,
    #[serde(rename = "capsule_radius")]
    pub widget_capsule_radius: Option<f64>,
    #[serde(rename = "capsule_opacity")]
    pub widget_capsule_opacity: Option<f64>,
    /// See `BarCapsuleGroupStyle::border_specified` — not a TOML key.
    #[serde(skip)]
    pub widget_capsule_border_specified: bool,
    #[serde(rename = "capsule_border", with = "optional_color_spec_serde")]
    pub widget_capsule_border: Option<ColorSpec>,
    pub hover_highlight: Option<bool>,
    pub dead_zone: BarDeadZoneOverride,
}

impl BarMonitorOverride {
    /// Port of `BarMonitorOverride::isAutoHideEnabled` (config_types.h:102-104).
    #[must_use]
    pub fn is_auto_hide_enabled(&self, base_auto_hide: bool, base_smart_auto_hide: bool) -> bool {
        self.auto_hide.unwrap_or(base_auto_hide)
            || self.smart_auto_hide.unwrap_or(base_smart_auto_hide)
    }
}

/// Port of `BarConfig` (config_types.h:117-190).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarConfig {
    /// Gesture -> action bindings applied to every widget on this bar.
    pub actions: HashMap<String, String>,
    /// Populated from the enclosing `[bar.<name>]` table key, not this
    /// struct's own derive (see module doc comment).
    #[serde(skip)]
    pub name: String,
    pub position: String,
    pub enabled: bool,
    /// Slide out when the pointer leaves; reveal on edge approach.
    pub auto_hide: bool,
    /// Hide while the active workspace has windows; show when it is empty.
    pub smart_auto_hide: bool,
    /// With `auto_hide`: briefly reveal when the active workspace changes.
    pub show_on_workspace_switch: bool,
    /// Reserve compositor exclusive zone; applies with or without auto_hide.
    pub reserve_space: bool,
    /// "top" | "overlay" — attached panels use the same layer.
    pub layer: String,
    pub thickness: i32,
    pub background_opacity: f32,
    #[serde(with = "color_spec_serde")]
    pub border: ColorSpec,
    pub border_width: f32,
    pub radius: i32,
    pub radius_top_left: i32,
    pub radius_top_right: i32,
    pub radius_bottom_left: i32,
    pub radius_bottom_right: i32,
    pub concave_edge_corners: bool,
    pub margin_ends: i32,
    pub margin_edge: i32,
    pub margin_opposite_edge: i32,
    pub padding: i32,
    pub widget_spacing: i32,
    pub shadow: bool,
    pub contact_shadow: bool,
    pub panel_overlap: i32,
    pub capsule_thickness: f32,
    pub scale: f32,
    pub font_weight: i32,
    pub font_family: Option<String>,
    #[serde(rename = "start")]
    pub start_widgets: Vec<String>,
    #[serde(rename = "center")]
    pub center_widgets: Vec<String>,
    #[serde(rename = "end")]
    pub end_widgets: Vec<String>,
    #[serde(rename = "capsule")]
    pub widget_capsule_default: bool,
    #[serde(rename = "capsule_fill", with = "color_spec_serde")]
    pub widget_capsule_fill: ColorSpec,
    #[serde(rename = "capsule_foreground", with = "optional_color_spec_serde")]
    pub widget_capsule_foreground: Option<ColorSpec>,
    #[serde(rename = "color", with = "optional_color_spec_serde")]
    pub widget_color: Option<ColorSpec>,
    #[serde(rename = "icon_color", with = "optional_color_spec_serde")]
    pub widget_icon_color: Option<ColorSpec>,
    #[serde(rename = "capsule_group")]
    pub widget_capsule_groups: Vec<BarCapsuleGroupStyle>,
    #[serde(rename = "capsule_padding")]
    pub widget_capsule_padding: f32,
    #[serde(rename = "capsule_radius")]
    pub widget_capsule_radius: Option<f64>,
    #[serde(rename = "capsule_opacity")]
    pub widget_capsule_opacity: f32,
    #[serde(skip)]
    pub widget_capsule_border_specified: bool,
    #[serde(rename = "capsule_border", with = "optional_color_spec_serde")]
    pub widget_capsule_border: Option<ColorSpec>,
    pub hover_highlight: bool,
    pub dead_zone: BarDeadZoneConfig,
    /// Populated from a separate `[bar.<name>.monitor.<id>]` table-map, not
    /// this struct's own derive (see module doc comment).
    #[serde(skip)]
    pub monitor_overrides: Vec<BarMonitorOverride>,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            actions: HashMap::new(),
            name: "default".to_string(),
            position: "top".to_string(),
            enabled: true,
            auto_hide: false,
            smart_auto_hide: false,
            show_on_workspace_switch: true,
            reserve_space: true,
            layer: "top".to_string(),
            thickness: style_defaults::BAR_THICKNESS_DEFAULT,
            background_opacity: 1.0,
            border: color_spec_from_role(ColorRole::Outline, 1.0),
            border_width: 0.0,
            radius: style_defaults::RADIUS_XL,
            radius_top_left: style_defaults::RADIUS_XL,
            radius_top_right: style_defaults::RADIUS_XL,
            radius_bottom_left: style_defaults::RADIUS_XL,
            radius_bottom_right: style_defaults::RADIUS_XL,
            concave_edge_corners: true,
            margin_ends: 100,
            margin_edge: 0,
            margin_opposite_edge: 0,
            padding: 14,
            widget_spacing: 6,
            shadow: true,
            contact_shadow: false,
            panel_overlap: 1,
            capsule_thickness: 0.76,
            scale: 1.0,
            font_weight: 500,
            font_family: None,
            start_widgets: vec![
                "launcher".to_string(),
                "wallpaper".to_string(),
                "workspaces".to_string(),
            ],
            center_widgets: vec!["clock".to_string()],
            end_widgets: [
                "media",
                "tray",
                "notifications",
                "clipboard",
                "network",
                "bluetooth",
                "volume",
                "brightness",
                "battery",
                "control-center",
                "session",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            widget_capsule_default: false,
            widget_capsule_fill: color_spec_from_role(ColorRole::SurfaceVariant, 1.0),
            widget_capsule_foreground: None,
            widget_color: None,
            widget_icon_color: None,
            widget_capsule_groups: Vec::new(),
            widget_capsule_padding: style_defaults::BAR_CAPSULE_PADDING,
            widget_capsule_radius: None,
            widget_capsule_opacity: 1.0,
            widget_capsule_border_specified: false,
            widget_capsule_border: None,
            hover_highlight: true,
            dead_zone: BarDeadZoneConfig::default(),
            monitor_overrides: Vec::new(),
        }
    }
}

impl BarConfig {
    /// Port of `BarConfig::isAutoHideEnabled` (config_types.h:128).
    #[must_use]
    pub const fn is_auto_hide_enabled(&self) -> bool {
        self.auto_hide || self.smart_auto_hide
    }
}

/// Optional rounded "capsule" behind a bar widget. Port of
/// `WidgetBarCapsuleSpec` (config_types.h:347-366). A computed *output* type
/// (see module doc comment) — never itself deserialized from a config file.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetBarCapsuleSpec {
    pub enabled: bool,
    pub fill: ColorSpec,
    /// Opaque group ID (auto-generated). Adjacent widgets in the same
    /// section with the same non-empty ID share one shell.
    pub group: String,
    pub border: Option<ColorSpec>,
    pub foreground: Option<ColorSpec>,
    pub padding: f32,
    pub radius: Option<f32>,
    pub opacity: f32,
    pub hover_highlight: bool,
}

impl Default for WidgetBarCapsuleSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            fill: color_spec_from_role(ColorRole::SurfaceVariant, 1.0),
            group: String::new(),
            border: None,
            foreground: None,
            padding: style_defaults::BAR_CAPSULE_PADDING,
            radius: None,
            opacity: 1.0,
            hover_highlight: true,
        }
    }
}

/// Port of `WidgetConfig` (config_types.h:368-389). Not itself
/// `Deserialize` — see module doc comment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WidgetConfig {
    /// Widget type (e.g. "clock", "spacer"); defaults to the entry name.
    pub r#type: String,
    pub settings: HashMap<String, WidgetSettingValue>,
    pub tables: HashMap<String, HashMap<String, String>>,
}

impl WidgetConfig {
    /// Port of `WidgetConfig::findSetting` (config_types.cpp:245-248).
    #[must_use]
    pub fn find_setting(&self, key: &str) -> Option<&WidgetSettingValue> {
        self.settings.get(key)
    }

    /// Port of `WidgetConfig::getString` (config_types.cpp:250-254).
    #[must_use]
    pub fn get_string(&self, key: &str, fallback: &str) -> String {
        self.find_setting(key)
            .and_then(|v| widget_setting_value_as::<String>(v, ""))
            .unwrap_or_else(|| fallback.to_string())
    }

    /// Port of `WidgetConfig::getStringList` (config_types.cpp:256-262).
    #[must_use]
    pub fn get_string_list(&self, key: &str, fallback: &[String]) -> Vec<String> {
        self.find_setting(key)
            .and_then(|v| widget_setting_value_as::<Vec<String>>(v, ""))
            .unwrap_or_else(|| fallback.to_vec())
    }

    /// Port of `WidgetConfig::getInt` (config_types.cpp:264-268).
    #[must_use]
    pub fn get_int(&self, key: &str, fallback: i64) -> i64 {
        self.find_setting(key)
            .and_then(|v| widget_setting_value_as::<i64>(v, ""))
            .unwrap_or(fallback)
    }

    /// Port of `WidgetConfig::getDouble` (config_types.cpp:270-274).
    #[must_use]
    pub fn get_double(&self, key: &str, fallback: f64) -> f64 {
        self.find_setting(key)
            .and_then(|v| widget_setting_value_as::<f64>(v, ""))
            .unwrap_or(fallback)
    }

    /// Port of `WidgetConfig::getBool` (config_types.cpp:276-280).
    #[must_use]
    pub fn get_bool(&self, key: &str, fallback: bool) -> bool {
        self.find_setting(key)
            .and_then(|v| widget_setting_value_as::<bool>(v, ""))
            .unwrap_or(fallback)
    }

    /// Port of `WidgetConfig::getColorSpec` (config_types.cpp:282-289).
    #[must_use]
    pub fn get_color_spec(&self, key: &str, fallback: ColorSpec, context: &str) -> ColorSpec {
        let ctx = if context.is_empty() { key } else { context };
        self.find_setting(key)
            .and_then(|v| widget_setting_value_as::<ColorSpec>(v, ctx))
            .unwrap_or(fallback)
    }

    /// Port of `WidgetConfig::getOptionalColorSpec` (config_types.cpp:291-302).
    #[must_use]
    pub fn get_optional_color_spec(&self, key: &str, context: &str) -> Option<ColorSpec> {
        let ctx = if context.is_empty() { key } else { context };
        let value = self.find_setting(key)?;
        if let WidgetSettingValue::String(s) = value
            && trim_c_whitespace(s).is_empty()
        {
            return None;
        }
        widget_setting_value_as::<ColorSpec>(value, ctx)
    }

    /// Port of `WidgetConfig::getStringMap` (config_types.cpp:304-311).
    #[must_use]
    pub fn get_string_map(
        &self,
        key: &str,
        fallback: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        self.tables
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    }

    /// Port of `WidgetConfig::hasSetting` (config_types.cpp:313).
    #[must_use]
    pub fn has_setting(&self, key: &str) -> bool {
        self.find_setting(key).is_some()
    }
}

/// Merges `[bar.*]` capsule defaults with `[widget.*]` overrides. Port of
/// `resolveWidgetBarCapsuleSpec` (config_types.cpp:315-380).
#[must_use]
pub fn resolve_widget_bar_capsule_spec(
    bar: &BarConfig,
    widget: Option<&WidgetConfig>,
) -> WidgetBarCapsuleSpec {
    let mut spec = WidgetBarCapsuleSpec::default();
    let widget_has_capsule_key = widget.is_some_and(|w| w.has_setting("capsule"));
    let widget_has_fill_key = widget.is_some_and(|w| w.has_setting("capsule_fill"));
    let widget_has_border_key = widget.is_some_and(|w| w.has_setting("capsule_border"));

    spec.enabled = if widget_has_capsule_key {
        widget.is_some_and(|w| w.get_bool("capsule", false))
    } else {
        bar.widget_capsule_default
    };

    spec.padding = bar.widget_capsule_padding;
    if let Some(w) = widget
        && w.has_setting("capsule_padding")
    {
        spec.padding =
            (w.get_double("capsule_padding", f64::from(spec.padding)) as f32).clamp(0.0, 48.0);
    }
    if let Some(radius) = bar.widget_capsule_radius {
        spec.radius = Some((radius as f32).clamp(0.0, 80.0));
    }
    if let Some(w) = widget
        && let Some(value) = w.settings.get("capsule_radius")
        && matches!(
            value,
            WidgetSettingValue::Float(_) | WidgetSettingValue::Int(_)
        )
    {
        let fallback = f64::from(spec.radius.unwrap_or(0.0));
        spec.radius = Some((w.get_double("capsule_radius", fallback) as f32).clamp(0.0, 80.0));
    }
    spec.opacity = bar.widget_capsule_opacity;
    if let Some(w) = widget
        && w.has_setting("capsule_opacity")
    {
        spec.opacity =
            (w.get_double("capsule_opacity", f64::from(spec.opacity)) as f32).clamp(0.0, 1.0);
    }
    spec.hover_highlight = bar.hover_highlight;

    if !spec.enabled {
        return spec;
    }

    spec.fill = if widget_has_fill_key {
        widget.map_or(bar.widget_capsule_fill, |w| {
            w.get_color_spec(
                "capsule_fill",
                bar.widget_capsule_fill,
                "widget.capsule_fill",
            )
        })
    } else {
        bar.widget_capsule_fill
    };

    spec.border = if widget_has_border_key {
        widget.and_then(|w| w.get_optional_color_spec("capsule_border", "widget.capsule_border"))
    } else if bar.widget_capsule_border_specified {
        bar.widget_capsule_border
    } else {
        None
    };

    spec.foreground = if widget.is_some_and(|w| w.has_setting("capsule_foreground")) {
        widget.and_then(|w| {
            w.get_optional_color_spec("capsule_foreground", "widget.capsule_foreground")
        })
    } else {
        bar.widget_capsule_foreground
    };
    spec
}

/// Returns the group for `id` on this bar, or `None` if `id` is empty or
/// unregistered. Port of `findBarCapsuleGroupStyle` (config_types.cpp:382-392).
#[must_use]
pub fn find_bar_capsule_group_style<'a>(
    bar: &'a BarConfig,
    id: &str,
) -> Option<&'a BarCapsuleGroupStyle> {
    if id.is_empty() {
        return None;
    }
    bar.widget_capsule_groups
        .iter()
        .find(|group| group.id == id)
}

fn collect_capsule_group_refs(lane: &[String], out: &mut HashSet<String>) {
    for entry in lane {
        if is_capsule_group_token(entry) {
            out.insert(capsule_group_token_id(entry));
        }
    }
}

fn effective_lane<'a>(monitor_lane: Option<&'a [String]>, bar_lane: &'a [String]) -> &'a [String] {
    monitor_lane.unwrap_or(bar_lane)
}

/// Group ids the bar scope's lanes reference (its own lanes, plus any
/// monitor override that doesn't carry its own `capsule_group` array). Port
/// of `capsuleGroupRefsForBarScope` (config_types.cpp:409-423).
#[must_use]
pub fn capsule_group_refs_for_bar_scope(bar: &BarConfig) -> HashSet<String> {
    let mut refs = HashSet::new();
    collect_capsule_group_refs(&bar.start_widgets, &mut refs);
    collect_capsule_group_refs(&bar.center_widgets, &mut refs);
    collect_capsule_group_refs(&bar.end_widgets, &mut refs);
    for ovr in &bar.monitor_overrides {
        if ovr.widget_capsule_groups.is_some() {
            continue;
        }
        collect_capsule_group_refs(
            effective_lane(ovr.start_widgets.as_deref(), &bar.start_widgets),
            &mut refs,
        );
        collect_capsule_group_refs(
            effective_lane(ovr.center_widgets.as_deref(), &bar.center_widgets),
            &mut refs,
        );
        collect_capsule_group_refs(
            effective_lane(ovr.end_widgets.as_deref(), &bar.end_widgets),
            &mut refs,
        );
    }
    refs
}

/// Port of `capsuleGroupRefsForMonitorScope` (config_types.cpp:425-431).
#[must_use]
pub fn capsule_group_refs_for_monitor_scope(
    bar: &BarConfig,
    monitor_override: &BarMonitorOverride,
) -> HashSet<String> {
    let mut refs = HashSet::new();
    collect_capsule_group_refs(
        effective_lane(
            monitor_override.start_widgets.as_deref(),
            &bar.start_widgets,
        ),
        &mut refs,
    );
    collect_capsule_group_refs(
        effective_lane(
            monitor_override.center_widgets.as_deref(),
            &bar.center_widgets,
        ),
        &mut refs,
    );
    collect_capsule_group_refs(
        effective_lane(monitor_override.end_widgets.as_deref(), &bar.end_widgets),
        &mut refs,
    );
    refs
}

/// Rebuilds an overriding `capsule_group` array against the config-file
/// array, in file order. Port of `reconcileCapsuleGroups`
/// (config_types.cpp:433-460).
#[must_use]
pub fn reconcile_capsule_groups(
    current: &[BarCapsuleGroupStyle],
    base: &[BarCapsuleGroupStyle],
    referenced: &HashSet<String>,
) -> Vec<BarCapsuleGroupStyle> {
    let mut out = Vec::with_capacity(current.len() + base.len());
    let mut consumed = vec![false; current.len()];
    for base_group in base {
        let mut matched = false;
        for (i, cur) in current.iter().enumerate() {
            if !consumed[i] && cur.id == base_group.id {
                out.push(cur.clone());
                consumed[i] = true;
                matched = true;
                break;
            }
        }
        if !matched && referenced.contains(&base_group.id) {
            out.push(base_group.clone());
        }
    }
    for (i, cur) in current.iter().enumerate() {
        if !consumed[i] && referenced.contains(&cur.id) {
            out.push(cur.clone());
        }
    }
    out
}

/// Builds the capsule spec a group's member widgets render with. Port of
/// `capsuleSpecFromGroup` (config_types.cpp:462-481).
#[must_use]
pub fn capsule_spec_from_group(
    bar: &BarConfig,
    group: &BarCapsuleGroupStyle,
) -> WidgetBarCapsuleSpec {
    let radius = if let Some(r) = group.radius {
        Some(r)
    } else {
        bar.widget_capsule_radius
            .map(|r| (r as f32).clamp(0.0, 80.0))
    };
    WidgetBarCapsuleSpec {
        enabled: true,
        fill: group.fill,
        group: group.id.clone(),
        border: if group.border_specified {
            group.border
        } else {
            None
        },
        foreground: group.foreground,
        padding: group.padding,
        radius,
        opacity: group.opacity,
        hover_highlight: bar.hover_highlight,
    }
}

/// Error raised when a widget's `scale` setting isn't a finite number. Port
/// of the `std::runtime_error`s in `resolveWidgetContentScale`
/// (config_types.cpp:509,515).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}: expected finite number")]
pub struct WidgetContentScaleError(String);

/// Port of `resolveWidgetContentScale` (config_types.cpp:496-519).
pub fn resolve_widget_content_scale(
    bar_scale: f32,
    widget: Option<&WidgetConfig>,
    context: &str,
) -> Result<f32, WidgetContentScaleError> {
    let Some(widget) = widget else {
        return Ok(bar_scale);
    };
    let Some(value) = widget.settings.get("scale") else {
        return Ok(bar_scale);
    };
    let widget_scale = match value {
        WidgetSettingValue::Float(f) => {
            if !f.is_finite() {
                return Err(WidgetContentScaleError(context.to_string()));
            }
            *f
        }
        WidgetSettingValue::Int(i) => *i as f64,
        _ => return Err(WidgetContentScaleError(context.to_string())),
    };
    Ok(bar_scale * (widget_scale as f32).clamp(0.2, 2.5))
}

/// Shared output selector matching used by monitor-scoped config and IPC
/// selectors. Port of `outputMatchesSelector` (config_types.cpp:557-579).
///
/// Takes `connector_name`/`description` directly rather than a
/// `WaylandOutput` (config_types.h:21's forward-declared struct): that type
/// belongs to the not-yet-ported compositor layer (a later phase). Callers
/// can adapt this signature once `WaylandOutput` lands.
#[must_use]
pub fn output_matches_selector(selector: &str, connector_name: &str, description: &str) -> bool {
    if !connector_name.is_empty() && selector == connector_name {
        return true;
    }
    if !description.is_empty() {
        let desc = description.as_bytes();
        let needle = selector.as_bytes();
        let mut pos = 0usize;
        while let Some(found) = find_bytes(desc, needle, pos) {
            let start_ok = found == 0 || is_c_isspace_byte(desc[found - 1]);
            let end = found + needle.len();
            let end_ok = end == desc.len() || is_c_isspace_byte(desc[end]);
            if start_ok && end_ok {
                return true;
            }
            pos = found + 1;
        }
    }
    false
}

/// Byte-level analogue of `std::string::find(needle, from)`: returns the
/// first index `>= from` at which `needle` occurs in `haystack`, matching
/// `std::string::npos`'s "empty needle matches at `from` itself" behavior.
fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return (from <= haystack.len()).then_some(from);
    }
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::widget_setting_value::IntoWidgetSettingValue;

    fn example_toml() -> &'static str {
        include_str!("../../../../example.toml")
    }

    fn parse_bar_main() -> BarConfig {
        let root: toml::Table = toml::from_str(example_toml()).expect("example.toml parses");
        let bar = root
            .get("bar")
            .and_then(toml::Value::as_table)
            .expect("[bar] table");
        let main = bar.get("main").expect("[bar.main] table").clone();
        main.try_into()
            .expect("[bar.main] deserializes into BarConfig")
    }

    #[test]
    fn bar_main_widget_lanes_deserialize_losslessly_from_example_toml() {
        let bar = parse_bar_main();
        assert_eq!(
            bar.start_widgets,
            vec!["launcher", "wallpaper", "workspaces"]
        );
        assert_eq!(bar.center_widgets, vec!["clock"]);
        assert_eq!(
            bar.end_widgets,
            vec![
                "media",
                "tray",
                "notifications",
                "clipboard",
                "network",
                "bluetooth",
                "volume",
                "brightness",
                "battery",
                "control-center",
                "session",
            ]
        );
    }

    #[test]
    fn bar_main_scalar_fields_deserialize_from_example_toml() {
        let bar = parse_bar_main();
        assert_eq!(bar.position, "top");
        assert_eq!(bar.thickness, 34);
        assert_eq!(bar.margin_ends, 180);
        assert_eq!(bar.margin_edge, 10);
        assert_eq!(bar.padding, 14);
        assert_eq!(bar.widget_spacing, 6);
        assert!(bar.shadow);
        assert!(!bar.auto_hide);
        assert!(bar.reserve_space);
        assert!(!bar.widget_capsule_default);
        // `name`/`monitor_overrides` are `#[serde(skip)]` — they keep their
        // struct defaults, not anything from the TOML table (see module doc
        // comment: these are assembled by the not-yet-written enclosing
        // `[bar.<name>]`-map reader, task 2.1.9/2.9).
        assert_eq!(bar.name, "default");
        assert!(bar.monitor_overrides.is_empty());
    }

    #[test]
    fn bar_config_missing_fields_use_cpp_defaults() {
        let bar: BarConfig =
            toml::from_str("").expect("empty table deserializes via container default");
        assert_eq!(bar, BarConfig::default());
    }

    #[test]
    fn bar_config_color_spec_round_trips_through_toml() {
        let toml_str = "border = \"#112233\"\ncapsule_fill = \"primary\"\n";
        let bar: BarConfig = toml::from_str(toml_str).expect("color fields parse");
        assert_eq!(bar.border.role, None);
        assert_eq!(bar.widget_capsule_fill.role, Some(ColorRole::Primary));

        let serialized = toml::to_string(&bar).expect("serializes back");
        assert!(serialized.contains("border = \"#112233\""));
        assert!(serialized.contains("capsule_fill = \"primary\""));
    }

    #[test]
    fn bar_config_capsule_padding_radius_opacity_border_use_short_toml_keys() {
        // Regression test (caught by pre-done review): these four fields were
        // missing `serde(rename = ...)`, so they silently never deserialized
        // from real config files despite the surrounding `widget_capsule_*`
        // fields all doing so correctly.
        let toml_str = "capsule_padding = 9.5\ncapsule_radius = 4.0\ncapsule_opacity = 0.5\ncapsule_border = \"error\"\n";
        let bar: BarConfig = toml::from_str(toml_str).expect("short capsule keys parse");
        assert_eq!(bar.widget_capsule_padding, 9.5);
        assert_eq!(bar.widget_capsule_radius, Some(4.0));
        assert_eq!(bar.widget_capsule_opacity, 0.5);
        assert_eq!(
            bar.widget_capsule_border.map(|s| s.role),
            Some(Some(ColorRole::Error))
        );
    }

    #[test]
    fn bar_monitor_override_capsule_padding_radius_opacity_border_use_short_toml_keys() {
        let toml_str = "capsule_padding = 9.5\ncapsule_radius = 4.0\ncapsule_opacity = 0.5\ncapsule_border = \"error\"\n";
        let ovr: BarMonitorOverride = toml::from_str(toml_str).expect("short capsule keys parse");
        assert_eq!(ovr.widget_capsule_padding, Some(9.5));
        assert_eq!(ovr.widget_capsule_radius, Some(4.0));
        assert_eq!(ovr.widget_capsule_opacity, Some(0.5));
        assert_eq!(
            ovr.widget_capsule_border.map(|s| s.role),
            Some(Some(ColorRole::Error))
        );
    }

    #[test]
    fn bar_monitor_override_match_keyword_field_round_trips() {
        let toml_str = "match = \"DP-1\"\nthickness = 44\n";
        let ovr: BarMonitorOverride = toml::from_str(toml_str).expect("parses");
        assert_eq!(ovr.match_, "DP-1");
        assert_eq!(ovr.thickness, Some(44));
        assert_eq!(ovr.position, None);
    }

    #[test]
    fn capsule_group_token_helpers_round_trip() {
        let token = make_capsule_group_token("abc123");
        assert_eq!(token, "group:abc123");
        assert!(is_capsule_group_token(&token));
        assert_eq!(capsule_group_token_id(&token), "abc123");
        assert!(!is_capsule_group_token("abc123"));
        assert_eq!(capsule_group_token_id("abc123"), "");
    }

    #[test]
    fn find_bar_capsule_group_style_finds_by_id_or_returns_none() {
        let mut bar = BarConfig::default();
        bar.widget_capsule_groups.push(BarCapsuleGroupStyle {
            id: "g1".to_string(),
            ..Default::default()
        });
        assert!(find_bar_capsule_group_style(&bar, "g1").is_some());
        assert!(find_bar_capsule_group_style(&bar, "missing").is_none());
        assert!(find_bar_capsule_group_style(&bar, "").is_none());
    }

    #[test]
    fn capsule_group_refs_for_bar_scope_collects_lane_tokens() {
        let bar = BarConfig {
            start_widgets: vec!["group:a".to_string(), "launcher".to_string()],
            center_widgets: vec!["group:b".to_string()],
            ..Default::default()
        };
        let refs = capsule_group_refs_for_bar_scope(&bar);
        assert_eq!(refs, HashSet::from(["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn capsule_group_refs_for_bar_scope_skips_monitor_overrides_with_their_own_groups() {
        let mut bar = BarConfig {
            start_widgets: vec!["group:a".to_string()],
            ..Default::default()
        };
        bar.monitor_overrides.push(BarMonitorOverride {
            start_widgets: Some(vec!["group:b".to_string()]),
            widget_capsule_groups: Some(vec![]),
            ..Default::default()
        });
        bar.monitor_overrides.push(BarMonitorOverride {
            center_widgets: Some(vec!["group:c".to_string()]),
            ..Default::default()
        });
        let refs = capsule_group_refs_for_bar_scope(&bar);
        // "b" is excluded: that override carries its own capsule_group array.
        // "c" is included: that override has no capsule_group of its own, so
        // its lanes fall back into the bar scope's reference set.
        assert_eq!(refs, HashSet::from(["a".to_string(), "c".to_string()]));
    }

    #[test]
    fn capsule_group_refs_for_monitor_scope_uses_override_lane_or_falls_back_to_bar_lane() {
        let bar = BarConfig {
            center_widgets: vec!["group:bar-fallback".to_string()],
            ..Default::default()
        };
        let ovr = BarMonitorOverride {
            start_widgets: Some(vec!["group:override".to_string()]),
            ..Default::default()
        };
        let refs = capsule_group_refs_for_monitor_scope(&bar, &ovr);
        assert_eq!(
            refs,
            HashSet::from(["override".to_string(), "bar-fallback".to_string()])
        );
    }

    #[test]
    fn reconcile_capsule_groups_keeps_edited_style_reintroduces_file_group_and_prunes_unreferenced()
    {
        let base = vec![
            BarCapsuleGroupStyle {
                id: "kept".to_string(),
                ..Default::default()
            },
            BarCapsuleGroupStyle {
                id: "reintroduced".to_string(),
                ..Default::default()
            },
        ];
        let current = vec![
            BarCapsuleGroupStyle {
                id: "kept".to_string(),
                opacity: 0.5,
                ..Default::default()
            },
            BarCapsuleGroupStyle {
                id: "gui-created".to_string(),
                ..Default::default()
            },
            BarCapsuleGroupStyle {
                id: "gui-orphaned".to_string(),
                ..Default::default()
            },
        ];
        let referenced = HashSet::from([
            "kept".to_string(),
            "reintroduced".to_string(),
            "gui-created".to_string(),
        ]);
        let result = reconcile_capsule_groups(&current, &base, &referenced);
        let ids: Vec<&str> = result.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["kept", "reintroduced", "gui-created"]);
        // The edited style survives, not the base's.
        assert_eq!(result[0].opacity, 0.5);
    }

    #[test]
    fn capsule_spec_from_group_inherits_bar_radius_when_group_radius_is_unset() {
        let bar = BarConfig {
            widget_capsule_radius: Some(20.0),
            ..Default::default()
        };
        let group = BarCapsuleGroupStyle {
            id: "g".to_string(),
            ..Default::default()
        };
        let spec = capsule_spec_from_group(&bar, &group);
        assert_eq!(spec.radius, Some(20.0));
        assert!(spec.enabled);
        assert_eq!(spec.group, "g");
    }

    #[test]
    fn capsule_spec_from_group_uses_own_radius_when_set() {
        let bar = BarConfig::default();
        let group = BarCapsuleGroupStyle {
            id: "g".to_string(),
            radius: Some(9.0),
            ..Default::default()
        };
        let spec = capsule_spec_from_group(&bar, &group);
        assert_eq!(spec.radius, Some(9.0));
    }

    #[test]
    fn capsule_spec_from_group_drops_border_when_not_specified() {
        let bar = BarConfig::default();
        let group = BarCapsuleGroupStyle {
            id: "g".to_string(),
            border_specified: false,
            border: Some(color_spec_from_role(ColorRole::Error, 1.0)),
            ..Default::default()
        };
        let spec = capsule_spec_from_group(&bar, &group);
        assert_eq!(spec.border, None);
    }

    #[test]
    fn resolve_widget_bar_capsule_spec_disabled_bar_default_ignores_widget_overrides_until_enabled()
    {
        let bar = BarConfig::default();
        assert!(!bar.widget_capsule_default);
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "capsule_fill".to_string(),
            WidgetSettingValue::String("error".to_string()),
        );
        let spec = resolve_widget_bar_capsule_spec(&bar, Some(&widget));
        assert!(!spec.enabled);
        // fill/border/foreground are left at their (irrelevant, since disabled) defaults.
        assert_eq!(
            spec.fill,
            color_spec_from_role(ColorRole::SurfaceVariant, 1.0)
        );
    }

    #[test]
    fn resolve_widget_bar_capsule_spec_widget_capsule_key_enables_it_even_if_bar_default_is_off() {
        let bar = BarConfig::default();
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "capsule".to_string(),
            true.into_widget_setting_value().unwrap(),
        );
        let spec = resolve_widget_bar_capsule_spec(&bar, Some(&widget));
        assert!(spec.enabled);
        assert_eq!(spec.fill, bar.widget_capsule_fill);
    }

    #[test]
    fn resolve_widget_bar_capsule_spec_widget_fill_override_takes_priority() {
        let bar = BarConfig::default();
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "capsule".to_string(),
            true.into_widget_setting_value().unwrap(),
        );
        widget.settings.insert(
            "capsule_fill".to_string(),
            "error".to_string().into_widget_setting_value().unwrap(),
        );
        let spec = resolve_widget_bar_capsule_spec(&bar, Some(&widget));
        assert_eq!(spec.fill.role, Some(ColorRole::Error));
    }

    #[test]
    fn resolve_widget_bar_capsule_spec_no_widget_uses_bar_defaults_only() {
        let bar = BarConfig::default();
        let spec = resolve_widget_bar_capsule_spec(&bar, None);
        assert_eq!(spec.enabled, bar.widget_capsule_default);
        assert_eq!(spec.padding, bar.widget_capsule_padding);
        assert_eq!(spec.opacity, bar.widget_capsule_opacity);
    }

    #[test]
    fn resolve_widget_bar_capsule_spec_numeric_capsule_radius_is_clamped() {
        let bar = BarConfig::default();
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "capsule".to_string(),
            true.into_widget_setting_value().unwrap(),
        );
        widget.settings.insert(
            "capsule_radius".to_string(),
            999.0f64.into_widget_setting_value().unwrap(),
        );
        let spec = resolve_widget_bar_capsule_spec(&bar, Some(&widget));
        assert_eq!(spec.radius, Some(80.0));
    }

    #[test]
    fn resolve_widget_bar_capsule_spec_non_numeric_capsule_radius_is_ignored() {
        let bar = BarConfig {
            widget_capsule_radius: Some(30.0),
            ..Default::default()
        };
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "capsule".to_string(),
            true.into_widget_setting_value().unwrap(),
        );
        widget.settings.insert(
            "capsule_radius".to_string(),
            "auto".to_string().into_widget_setting_value().unwrap(),
        );
        let spec = resolve_widget_bar_capsule_spec(&bar, Some(&widget));
        assert_eq!(spec.radius, Some(30.0));
    }

    #[test]
    fn resolve_widget_content_scale_no_widget_returns_bar_scale() {
        assert_eq!(
            resolve_widget_content_scale(1.5, None, "widget.scale"),
            Ok(1.5)
        );
    }

    #[test]
    fn resolve_widget_content_scale_missing_setting_returns_bar_scale() {
        let widget = WidgetConfig::default();
        assert_eq!(
            resolve_widget_content_scale(1.5, Some(&widget), "widget.scale"),
            Ok(1.5)
        );
    }

    #[test]
    fn resolve_widget_content_scale_clamps_and_multiplies() {
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "scale".to_string(),
            3.0f64.into_widget_setting_value().unwrap(),
        );
        // 3.0 clamps to 2.5.
        assert_eq!(
            resolve_widget_content_scale(2.0, Some(&widget), "widget.scale"),
            Ok(5.0)
        );
    }

    #[test]
    fn resolve_widget_content_scale_accepts_integer_setting() {
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "scale".to_string(),
            1i64.into_widget_setting_value().unwrap(),
        );
        assert_eq!(
            resolve_widget_content_scale(2.0, Some(&widget), "widget.scale"),
            Ok(2.0)
        );
    }

    #[test]
    fn resolve_widget_content_scale_rejects_non_finite() {
        let mut widget = WidgetConfig::default();
        widget
            .settings
            .insert("scale".to_string(), WidgetSettingValue::Float(f64::NAN));
        let err = resolve_widget_content_scale(2.0, Some(&widget), "widget.scale").unwrap_err();
        assert_eq!(err.to_string(), "widget.scale: expected finite number");
    }

    #[test]
    fn resolve_widget_content_scale_rejects_non_numeric_setting() {
        let mut widget = WidgetConfig::default();
        widget
            .settings
            .insert("scale".to_string(), WidgetSettingValue::Bool(true));
        assert!(resolve_widget_content_scale(2.0, Some(&widget), "widget.scale").is_err());
    }

    #[test]
    fn output_matches_selector_exact_connector_match() {
        assert!(output_matches_selector("DP-1", "DP-1", ""));
        assert!(!output_matches_selector("DP-1", "DP-2", ""));
    }

    #[test]
    fn output_matches_selector_word_boundary_in_description() {
        assert!(output_matches_selector("DP-1", "", "BOE 0x0BCA DP-1 panel"));
        // "DP-1" must not match inside "eDP-1" (no boundary before it).
        assert!(!output_matches_selector("DP-1", "", "BOE 0x0BCA eDP-1"));
    }

    #[test]
    fn output_matches_selector_no_match_returns_false() {
        assert!(!output_matches_selector(
            "HDMI-1",
            "DP-1",
            "some description"
        ));
    }

    #[test]
    fn widget_config_accessors_use_typed_fallbacks() {
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "label".to_string(),
            WidgetSettingValue::String("hi".to_string()),
        );
        widget
            .settings
            .insert("count".to_string(), WidgetSettingValue::Int(3));
        widget
            .settings
            .insert("ratio".to_string(), WidgetSettingValue::Float(1.5));
        widget
            .settings
            .insert("on".to_string(), WidgetSettingValue::Bool(true));
        widget.tables.insert(
            "labels".to_string(),
            HashMap::from([("k".to_string(), "v".to_string())]),
        );

        assert_eq!(widget.get_string("label", "fallback"), "hi");
        assert_eq!(widget.get_string("missing", "fallback"), "fallback");
        assert_eq!(widget.get_int("count", 0), 3);
        assert_eq!(widget.get_double("ratio", 0.0), 1.5);
        assert!(widget.get_bool("on", false));
        assert_eq!(
            widget
                .get_string_map("labels", &HashMap::new())
                .get("k")
                .unwrap(),
            "v"
        );
        assert_eq!(
            widget.get_string_map("missing", &HashMap::new()),
            HashMap::new()
        );
        assert!(widget.has_setting("on"));
        assert!(!widget.has_setting("nope"));
    }

    #[test]
    fn widget_config_get_optional_color_spec_treats_blank_string_as_absent() {
        let mut widget = WidgetConfig::default();
        widget.settings.insert(
            "border".to_string(),
            WidgetSettingValue::String("   ".to_string()),
        );
        assert_eq!(widget.get_optional_color_spec("border", ""), None);

        widget.settings.insert(
            "border2".to_string(),
            WidgetSettingValue::String("primary".to_string()),
        );
        assert_eq!(
            widget
                .get_optional_color_spec("border2", "")
                .map(|s| s.role),
            Some(Some(ColorRole::Primary))
        );
    }

    #[test]
    fn find_bytes_handles_empty_needle_like_std_string_find() {
        assert_eq!(find_bytes(b"abc", b"", 0), Some(0));
        assert_eq!(find_bytes(b"abc", b"", 3), Some(3));
        assert_eq!(find_bytes(b"abc", b"", 4), None);
        assert_eq!(find_bytes(b"abc", b"b", 0), Some(1));
        assert_eq!(find_bytes(b"abc", b"z", 0), None);
    }
}
