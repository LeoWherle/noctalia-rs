//! Task 2.1.7 — NightLight configuration types.
//! Port of `NightLightConfig` from `src/config/config_types.h`
//! (lines 1244-1256) and `config_schema.cpp:153-182`.

use serde::{Deserialize, Serialize};

/// Port of `NightLightConfig` (config_types.h:1244-1256).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NightLightConfig {
    pub enabled: bool,
    pub force: bool,
    #[serde(rename = "temperature_day")]
    pub day_temperature: i32,
    #[serde(rename = "temperature_night")]
    pub night_temperature: i32,
}

impl NightLightConfig {
    pub const TEMPERATURE_MIN: i32 = 1000;
    pub const TEMPERATURE_MAX: i32 = 10000;
    pub const TEMPERATURE_GAP: i32 = 100;
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            force: false,
            day_temperature: 6500,
            night_temperature: 4000,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn nightlight_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let nl_tbl = root
            .get("nightlight")
            .expect("[nightlight] table present in example.toml");
        let config: NightLightConfig = nl_tbl
            .clone()
            .try_into()
            .expect("[nightlight] parses into NightLightConfig");

        assert!(!config.enabled);
        assert!(!config.force);
        assert_eq!(config.day_temperature, 6500);
        assert_eq!(config.night_temperature, 4000);
    }

    #[test]
    fn nightlight_config_defaults_match_cpp() {
        let config = NightLightConfig::default();
        assert!(!config.enabled);
        assert!(!config.force);
        assert_eq!(config.day_temperature, 6500);
        assert_eq!(config.night_temperature, 4000);
        assert_eq!(NightLightConfig::TEMPERATURE_MIN, 1000);
        assert_eq!(NightLightConfig::TEMPERATURE_MAX, 10000);
        assert_eq!(NightLightConfig::TEMPERATURE_GAP, 100);
    }
}
