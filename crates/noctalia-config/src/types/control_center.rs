//! Task 2.1.8 — Control Center configuration types.
//! Port of `ControlCenterConfig` and `ControlCenterConfig::CalendarTabConfig` from
//! `src/config/config_types.h` (lines 1443-1462) and `config_schema.cpp:478-503`.

use serde::{Deserialize, Serialize};

use crate::types::shell::{ShortcutConfig, default_control_center_shortcuts};

/// Port of `ControlCenterConfig::CalendarTabConfig` (config_types.h:1446-1452).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarTabConfig {
    pub show_events_card: bool,
    pub show_week_numbers: bool,
    pub event_date_format: String,
    pub event_time_format: String,
}

impl Default for CalendarTabConfig {
    fn default() -> Self {
        Self {
            show_events_card: true,
            show_week_numbers: false,
            event_date_format: "%A %e %B".to_string(),
            event_time_format: "%H:%M".to_string(),
        }
    }
}

/// Port of `ControlCenterConfig` (config_types.h:1443-1462).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlCenterConfig {
    /// Populated from `default_control_center_shortcuts()` in C++ defaults;
    /// `#[serde(skip)]` per shortcut task 2.1.3 pattern.
    #[serde(skip)]
    pub shortcuts: Vec<ShortcutConfig>,
    pub hidden_tabs: Vec<String>,
    #[serde(rename = "sidebar")]
    pub sidebar_mode: String,
    #[serde(rename = "sidebar_section")]
    pub sidebar_section_mode: String,
    pub width: i32,
    pub show_shortcut_labels: bool,
    #[serde(rename = "calendar")]
    pub calendar_tab: CalendarTabConfig,
}

impl Default for ControlCenterConfig {
    fn default() -> Self {
        Self {
            shortcuts: default_control_center_shortcuts(),
            hidden_tabs: Vec::new(),
            sidebar_mode: "compact".to_string(),
            sidebar_section_mode: "compact".to_string(),
            width: 700,
            show_shortcut_labels: true,
            calendar_tab: CalendarTabConfig::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn control_center_calendar_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let cc_tbl = root
            .get("control_center")
            .expect("[control_center] table present in example.toml");
        let config: ControlCenterConfig = cc_tbl
            .clone()
            .try_into()
            .expect("[control_center] parses into ControlCenterConfig");

        assert_eq!(config.sidebar_mode, "compact");
        assert_eq!(config.sidebar_section_mode, "compact");
        assert_eq!(config.width, 700);
        assert!(config.show_shortcut_labels);
        assert!(config.calendar_tab.show_events_card);
        assert!(!config.calendar_tab.show_week_numbers);
    }

    #[test]
    fn control_center_config_defaults_match_cpp() {
        let config = ControlCenterConfig::default();
        assert_eq!(config.sidebar_mode, "compact");
        assert_eq!(config.sidebar_section_mode, "compact");
        assert_eq!(config.width, 700);
        assert!(config.show_shortcut_labels);
        assert!(!config.shortcuts.is_empty());
        assert!(config.calendar_tab.show_events_card);
        assert_eq!(config.calendar_tab.event_time_format, "%H:%M");
    }
}
