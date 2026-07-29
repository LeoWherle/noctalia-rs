//! Task 2.1.6 — Accessibility configuration types.
//! Port of `AccessibilityConfig` from `src/config/config_types.h`
//! (lines 1508-1512) and `config_schema.cpp:2256-2262`.

use serde::{Deserialize, Serialize};

/// Port of `AccessibilityConfig` (config_types.h:1508-1512).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessibilityConfig {
    pub ui_scale: f32,
    pub high_contrast: bool,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            high_contrast: false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn accessibility_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let acc_tbl = root
            .get("accessibility")
            .expect("[accessibility] table present in example.toml");
        let config: AccessibilityConfig = acc_tbl
            .clone()
            .try_into()
            .expect("[accessibility] parses into AccessibilityConfig");

        assert_eq!(config.ui_scale, 1.0);
        assert!(!config.high_contrast);
    }

    #[test]
    fn accessibility_config_defaults_match_cpp() {
        let config = AccessibilityConfig::default();
        assert_eq!(config.ui_scale, 1.0);
        assert!(!config.high_contrast);
    }
}
