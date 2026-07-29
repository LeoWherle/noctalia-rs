//! Task 2.1.7 — Brightness configuration types.
//! Port of `BrightnessBackendPreference`, `BrightnessMonitorOverride`, and `BrightnessConfig`
//! from `src/config/config_types.h` (lines 1183-1213) and `config_schema.cpp:414-452`.

use serde::{Deserialize, Serialize};

/// Port of `BrightnessBackendPreference` (config_types.h:1183-1188).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrightnessBackendPreference {
    #[default]
    Auto = 0,
    None = 1,
    Backlight = 2,
    Ddcutil = 3,
}

/// Port of `BrightnessMonitorOverride` (config_types.h:1197-1203).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BrightnessMonitorOverride {
    /// Table key in `[brightness.monitor.<match>]`.
    #[serde(skip)]
    pub match_: String,
    pub backend: Option<BrightnessBackendPreference>,
    pub backlight_device: Option<String>,
}

/// Port of `BrightnessConfig` (config_types.h:1205-1213).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrightnessConfig {
    pub enable_ddcutil: bool,
    pub sync_all_monitors: bool,
    #[serde(rename = "ignore_mmids")]
    pub ddcutil_ignore_mmids: Vec<String>,
    pub minimum_brightness: f32,
    /// Populated from `[brightness.monitor.<match>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub monitor_overrides: Vec<BrightnessMonitorOverride>,
}

impl Default for BrightnessConfig {
    fn default() -> Self {
        Self {
            enable_ddcutil: false,
            sync_all_monitors: false,
            ddcutil_ignore_mmids: Vec::new(),
            minimum_brightness: 0.0,
            monitor_overrides: Vec::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn brightness_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let bright_tbl = root
            .get("brightness")
            .expect("[brightness] table present in example.toml");
        let config: BrightnessConfig = bright_tbl
            .clone()
            .try_into()
            .expect("[brightness] parses into BrightnessConfig");

        assert!(!config.enable_ddcutil);
        assert!(!config.sync_all_monitors);
        assert_eq!(config.minimum_brightness, 0.0);
    }

    #[test]
    fn brightness_config_defaults_match_cpp() {
        let config = BrightnessConfig::default();
        assert!(!config.enable_ddcutil);
        assert!(!config.sync_all_monitors);
        assert!(config.ddcutil_ignore_mmids.is_empty());
        assert_eq!(config.minimum_brightness, 0.0);
        assert!(config.monitor_overrides.is_empty());
    }
}
