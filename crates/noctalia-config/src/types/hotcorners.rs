//! Task 2.1.6 — HotCorners configuration types.
//! Port of `HotCornersConfig` and `Corner` from `src/config/config_types.h`
//! (lines 1514-1531) and `config_schema.cpp:397-411`.

use serde::{Deserialize, Serialize};

/// Port of `HotCornersConfig::Corner` (config_types.h:1519-1523).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotCornerConfig {
    pub action: String,
    pub command: String,
}

impl Default for HotCornerConfig {
    fn default() -> Self {
        Self {
            action: "none".to_string(),
            command: String::new(),
        }
    }
}

/// Port of `HotCornersConfig` (config_types.h:1514-1531).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HotCornersConfig {
    pub enabled: bool,
    pub delay_ms: i32,
    pub top_left: HotCornerConfig,
    pub top_right: HotCornerConfig,
    pub bottom_left: HotCornerConfig,
    pub bottom_right: HotCornerConfig,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hotcorners_deserializes_from_toml() {
        let toml_str = r#"
            enabled = true
            delay_ms = 150
            [top_left]
            action = "command"
            command = "wofi --show drun"
        "#;
        let config: HotCornersConfig =
            toml::from_str(toml_str).expect("parses into HotCornersConfig");

        assert!(config.enabled);
        assert_eq!(config.delay_ms, 150);
        assert_eq!(config.top_left.action, "command");
        assert_eq!(config.top_left.command, "wofi --show drun");
        assert_eq!(config.top_right.action, "none");
    }

    #[test]
    fn hotcorners_config_defaults_match_cpp() {
        let config = HotCornersConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.delay_ms, 0);
        assert_eq!(config.top_left.action, "none");
        assert_eq!(config.top_right.action, "none");
        assert_eq!(config.bottom_left.action, "none");
        assert_eq!(config.bottom_right.action, "none");
    }
}
