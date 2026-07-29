//! Task 2.1.5 — Notification daemon & filter configuration types.
//! Port of `NotificationFilterConfig` and `NotificationConfig` from
//! `src/config/config_types.h` (lines 266-282, 685-703) and
//! `config_schema.cpp:197-235, 241-366`.

use serde::{Deserialize, Serialize};

/// Port of `NotificationFilterConfig` (config_types.h:266-282).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationFilterConfig {
    /// Table key in `[notification.filter.<name>]`.
    #[serde(skip)]
    pub name: String,
    pub enabled: bool,
    #[serde(rename = "match")]
    pub match_: String,
    pub match_content: String,
    pub show_toast: bool,
    pub save_history: bool,
    pub play_sound: bool,
    pub allow_permanent: bool,
    pub override_duration: Option<i32>,
    pub allowed_urgencies: Vec<String>,
}

impl Default for NotificationFilterConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            match_: String::new(),
            match_content: String::new(),
            show_toast: true,
            save_history: true,
            play_sound: true,
            allow_permanent: true,
            override_duration: None,
            allowed_urgencies: Vec::new(),
        }
    }
}

/// Port of `NotificationConfig` (config_types.h:685-703).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub enable_daemon: bool,
    pub show_app_name: bool,
    pub show_actions: bool,
    pub position: String,
    pub layer: String,
    pub scale: f32,
    pub background_opacity: f32,
    pub border: bool,
    pub offset_x: i32,
    pub offset_y: i32,
    pub monitors: Vec<String>,
    pub collapse_on_dismiss: bool,
    pub history_retention_hours: i32,
    /// Populated from `[notification.filter.<name>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub filters: Vec<NotificationFilterConfig>,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enable_daemon: true,
            show_app_name: true,
            show_actions: true,
            position: "top_right".to_string(),
            layer: "top".to_string(),
            scale: 1.0,
            background_opacity: 0.97,
            border: true,
            offset_x: 20,
            offset_y: 8,
            monitors: Vec::new(),
            collapse_on_dismiss: true,
            history_retention_hours: 0,
            filters: Vec::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn notification_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let notif_tbl = root
            .get("notification")
            .expect("[notification] table present in example.toml");
        let config: NotificationConfig = notif_tbl
            .clone()
            .try_into()
            .expect("[notification] parses into NotificationConfig");

        assert!(config.enable_daemon);
        assert!(config.show_app_name);
        assert!(config.show_actions);
        assert_eq!(config.layer, "top");
        assert_eq!(config.scale, 1.0);
        assert_eq!(config.background_opacity, 0.97);
        assert_eq!(config.offset_x, 20);
        assert_eq!(config.offset_y, 8);
    }

    #[test]
    fn notification_config_defaults_match_cpp() {
        let config = NotificationConfig::default();
        assert!(config.enable_daemon);
        assert!(config.show_app_name);
        assert!(config.show_actions);
        assert_eq!(config.position, "top_right");
        assert_eq!(config.layer, "top");
        assert_eq!(config.scale, 1.0);
        assert_eq!(config.background_opacity, 0.97);
        assert!(config.border);
        assert_eq!(config.offset_x, 20);
        assert_eq!(config.offset_y, 8);
        assert!(config.monitors.is_empty());
        assert!(config.collapse_on_dismiss);
        assert_eq!(config.history_retention_hours, 0);
        assert!(config.filters.is_empty());
    }

    #[test]
    fn notification_filter_config_defaults_match_cpp() {
        let filter = NotificationFilterConfig::default();
        assert_eq!(filter.name, "");
        assert!(filter.enabled);
        assert_eq!(filter.match_, "");
        assert_eq!(filter.match_content, "");
        assert!(filter.show_toast);
        assert!(filter.save_history);
        assert!(filter.play_sound);
        assert!(filter.allow_permanent);
        assert!(filter.override_duration.is_none());
        assert!(filter.allowed_urgencies.is_empty());
    }
}
