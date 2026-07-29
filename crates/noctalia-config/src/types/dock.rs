//! Task 2.1.4 — Dock configuration types.
//! Port of `DockConfig`, `DockEdge`, `DockLauncherPosition`, and `isAutoHideEnabled`
//! from `src/config/config_types.h` (lines 534-605) and `config_schema.cpp:1761-1832`.

use noctalia_core::color::{ColorRole, ColorSpec, color_spec_from_role};
use serde::{Deserialize, Serialize};

/// Port of `DockEdge` (config_types.h:534-539).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    Top = 0,
    #[default]
    Bottom = 1,
    Left = 2,
    Right = 3,
}

/// Port of `DockLauncherPosition` (config_types.h:548-552).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockLauncherPosition {
    #[default]
    None = 0,
    Start = 1,
    End = 2,
}

/// Port of `DockConfig` (config_types.h:560-605).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub enabled: bool,
    pub position: DockEdge,
    pub active_monitor_only: bool,
    pub icon_size: i32,
    pub main_axis_padding: i32,
    pub cross_axis_padding: i32,
    pub item_spacing: i32,
    pub background_opacity: f32,
    #[serde(with = "crate::types::serde_support::color_spec_serde")]
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
    pub shadow: bool,
    pub show_running: bool,
    pub auto_hide: bool,
    pub smart_auto_hide: bool,
    pub layer: String,
    pub reserve_space: bool,
    pub active_scale: f32,
    pub inactive_scale: f32,
    pub magnification: bool,
    pub magnification_scale: f32,
    pub active_opacity: f32,
    pub inactive_opacity: f32,
    pub show_dots: bool,
    pub show_instance_count: bool,
    pub launcher_position: DockLauncherPosition,
    pub launcher_icon: String,
    pub launcher_custom_image: String,
    pub launcher_custom_image_colorize: bool,
    pub pinned: Vec<String>,
    pub monitors: Vec<String>,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            position: DockEdge::Bottom,
            active_monitor_only: false,
            icon_size: 48,
            main_axis_padding: 16,
            cross_axis_padding: 8,
            item_spacing: 6,
            background_opacity: 0.88,
            border: color_spec_from_role(ColorRole::Outline, 1.0),
            border_width: 0.0,
            radius: 16,
            radius_top_left: 16,
            radius_top_right: 16,
            radius_bottom_left: 16,
            radius_bottom_right: 16,
            concave_edge_corners: true,
            margin_ends: 0,
            margin_edge: 0,
            shadow: true,
            show_running: true,
            auto_hide: false,
            smart_auto_hide: false,
            layer: "top".to_string(),
            reserve_space: true,
            active_scale: 1.0,
            inactive_scale: 0.85,
            magnification: true,
            magnification_scale: 1.45,
            active_opacity: 1.0,
            inactive_opacity: 0.85,
            show_dots: false,
            show_instance_count: true,
            launcher_position: DockLauncherPosition::None,
            launcher_icon: "grid-dots".to_string(),
            launcher_custom_image: String::new(),
            launcher_custom_image_colorize: false,
            pinned: Vec::new(),
            monitors: Vec::new(),
        }
    }
}

impl DockConfig {
    /// Port of `DockConfig::isAutoHideEnabled` (config_types.h:586).
    #[inline]
    pub fn is_auto_hide_enabled(&self) -> bool {
        self.auto_hide || self.smart_auto_hide
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn dock_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let dock_tbl = root
            .get("dock")
            .expect("[dock] table present in example.toml");
        let config: DockConfig = dock_tbl
            .clone()
            .try_into()
            .expect("[dock] parses into DockConfig");

        assert!(!config.enabled);
        assert_eq!(config.position, DockEdge::Bottom);
        assert_eq!(config.icon_size, 48);
        assert_eq!(config.main_axis_padding, 16);
        assert_eq!(config.cross_axis_padding, 8);
        assert_eq!(config.item_spacing, 6);
        assert_eq!(config.background_opacity, 0.88);
        assert_eq!(config.radius, 16);
        assert_eq!(config.radius_top_left, 16);
        assert_eq!(config.radius_top_right, 16);
        assert_eq!(config.radius_bottom_left, 16);
        assert_eq!(config.radius_bottom_right, 16);
        assert_eq!(config.margin_ends, 0);
        assert_eq!(config.margin_edge, 8);
        assert!(config.shadow);
        assert!(config.show_running);
        assert!(!config.auto_hide);
        assert!(config.reserve_space);
        assert_eq!(config.layer, "top");
        assert_eq!(config.active_scale, 1.0);
        assert_eq!(config.inactive_scale, 0.85);
        assert!(config.magnification);
        assert_eq!(config.magnification_scale, 1.45);
        assert_eq!(config.active_opacity, 1.0);
        assert_eq!(config.inactive_opacity, 0.85);
        assert!(!config.show_dots);
        assert!(config.show_instance_count);
        assert_eq!(config.launcher_position, DockLauncherPosition::None);
        assert_eq!(config.launcher_icon, "grid-dots");
        assert!(!config.active_monitor_only);
        assert!(config.pinned.is_empty());
    }

    #[test]
    fn dock_config_defaults_match_cpp() {
        let config = DockConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.position, DockEdge::Bottom);
        assert!(!config.active_monitor_only);
        assert_eq!(config.icon_size, 48);
        assert_eq!(config.main_axis_padding, 16);
        assert_eq!(config.cross_axis_padding, 8);
        assert_eq!(config.item_spacing, 6);
        assert_eq!(config.background_opacity, 0.88);
        assert_eq!(config.border, color_spec_from_role(ColorRole::Outline, 1.0));
        assert_eq!(config.border_width, 0.0);
        assert_eq!(config.radius, 16);
        assert_eq!(config.radius_top_left, 16);
        assert_eq!(config.radius_top_right, 16);
        assert_eq!(config.radius_bottom_left, 16);
        assert_eq!(config.radius_bottom_right, 16);
        assert!(config.concave_edge_corners);
        assert_eq!(config.margin_ends, 0);
        assert_eq!(config.margin_edge, 0);
        assert!(config.shadow);
        assert!(config.show_running);
        assert!(!config.auto_hide);
        assert!(!config.smart_auto_hide);
        assert_eq!(config.layer, "top");
        assert!(config.reserve_space);
        assert_eq!(config.active_scale, 1.0);
        assert_eq!(config.inactive_scale, 0.85);
        assert!(config.magnification);
        assert_eq!(config.magnification_scale, 1.45);
        assert_eq!(config.active_opacity, 1.0);
        assert_eq!(config.inactive_opacity, 0.85);
        assert!(!config.show_dots);
        assert!(config.show_instance_count);
        assert_eq!(config.launcher_position, DockLauncherPosition::None);
        assert_eq!(config.launcher_icon, "grid-dots");
        assert_eq!(config.launcher_custom_image, "");
        assert!(!config.launcher_custom_image_colorize);
        assert!(config.pinned.is_empty());
        assert!(config.monitors.is_empty());
    }

    #[test]
    fn dock_is_auto_hide_enabled() {
        let mut config = DockConfig::default();
        assert!(!config.is_auto_hide_enabled());

        config.auto_hide = true;
        assert!(config.is_auto_hide_enabled());

        config.auto_hide = false;
        config.smart_auto_hide = true;
        assert!(config.is_auto_hide_enabled());

        config.auto_hide = true;
        assert!(config.is_auto_hide_enabled());
    }
}
