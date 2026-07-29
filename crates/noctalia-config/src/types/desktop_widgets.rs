//! Task 2.1.5 — Desktop & Lockscreen Widgets configuration types.
//! Port of `DesktopWidgetsGridState`, `DesktopWidgetState`, `DesktopWidgetsConfig`,
//! and `LockscreenWidgetsConfig` from `src/config/config_types.h` (lines 605-648)
//! and `config_schema.cpp:369-395`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::widget_setting_value::WidgetSettingValue;

/// Port of `DesktopWidgetsGridState` (config_types.h:605-611).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopWidgetsGridState {
    pub visible: bool,
    pub cell_size: i32,
    pub major_interval: i32,
}

impl Default for DesktopWidgetsGridState {
    fn default() -> Self {
        Self {
            visible: true,
            cell_size: 16,
            major_interval: 4,
        }
    }
}

/// Port of `DesktopWidgetState` (config_types.h:613-630).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopWidgetState {
    /// Canonical identifier; table key in `[desktop_widgets.widget.<id>]`.
    #[serde(skip)]
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "output")]
    pub output_name: String,
    pub cx: f32,
    pub cy: f32,
    pub box_width: f32,
    pub box_height: f32,
    #[serde(rename = "rotation")]
    pub rotation_rad: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub enabled: bool,
    #[serde(skip)]
    pub settings: HashMap<String, WidgetSettingValue>,
}

impl Default for DesktopWidgetState {
    fn default() -> Self {
        Self {
            id: String::new(),
            type_: "clock".to_string(),
            output_name: String::new(),
            cx: 0.0,
            cy: 0.0,
            box_width: 0.0,
            box_height: 0.0,
            rotation_rad: 0.0,
            flip_x: false,
            flip_y: false,
            enabled: true,
            settings: HashMap::new(),
        }
    }
}

/// Port of `DesktopWidgetsConfig` (config_types.h:632-639).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopWidgetsConfig {
    pub enabled: bool,
    pub schema_version: i32,
    pub grid: DesktopWidgetsGridState,
    /// Populated from `[desktop_widgets.widget.<id>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub widgets: Vec<DesktopWidgetState>,
}

impl Default for DesktopWidgetsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            schema_version: 2,
            grid: DesktopWidgetsGridState::default(),
            widgets: Vec::new(),
        }
    }
}

/// Port of `LockscreenWidgetsConfig` (config_types.h:641-648).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockscreenWidgetsConfig {
    pub enabled: bool,
    pub schema_version: i32,
    pub grid: DesktopWidgetsGridState,
    /// Populated from `[lockscreen_widgets.widget.<id>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub widgets: Vec<DesktopWidgetState>,
}

impl Default for LockscreenWidgetsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            schema_version: 2,
            grid: DesktopWidgetsGridState::default(),
            widgets: Vec::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn desktop_widgets_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let dw_tbl = root
            .get("desktop_widgets")
            .expect("[desktop_widgets] table present in example.toml");
        let config: DesktopWidgetsConfig = dw_tbl
            .clone()
            .try_into()
            .expect("[desktop_widgets] parses into DesktopWidgetsConfig");

        assert!(!config.enabled);
        assert_eq!(config.schema_version, 2);
        assert!(config.grid.visible);
        assert_eq!(config.grid.cell_size, 16);
        assert_eq!(config.grid.major_interval, 4);
    }

    #[test]
    fn desktop_widgets_config_defaults_match_cpp() {
        let config = DesktopWidgetsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.schema_version, 2);
        assert!(config.grid.visible);
        assert_eq!(config.grid.cell_size, 16);
        assert_eq!(config.grid.major_interval, 4);
        assert!(config.widgets.is_empty());
    }

    #[test]
    fn lockscreen_widgets_config_defaults_match_cpp() {
        let config = LockscreenWidgetsConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.schema_version, 2);
        assert!(config.grid.visible);
        assert_eq!(config.grid.cell_size, 16);
        assert_eq!(config.grid.major_interval, 4);
        assert!(config.widgets.is_empty());
    }
}
