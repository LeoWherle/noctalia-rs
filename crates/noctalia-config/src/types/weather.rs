//! Task 2.1.8 — Weather configuration types.
//! Port of `WeatherConfig` from `src/config/config_types.h`
//! (lines 1050-1057) and `config_schema.cpp:32-39`.

use serde::{Deserialize, Serialize};

/// Port of `WeatherConfig` (config_types.h:1050-1057).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherConfig {
    pub enabled: bool,
    pub effects: bool,
    pub refresh_minutes: i32,
    pub unit: String,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            effects: true,
            refresh_minutes: 30,
            unit: "metric".to_string(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn weather_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let weather_tbl = root
            .get("weather")
            .expect("[weather] table present in example.toml");
        let config: WeatherConfig = weather_tbl
            .clone()
            .try_into()
            .expect("[weather] parses into WeatherConfig");

        assert!(!config.enabled);
        assert!(config.effects);
        assert_eq!(config.refresh_minutes, 30);
        assert_eq!(config.unit, "celsius");
    }

    #[test]
    fn weather_config_defaults_match_cpp() {
        let config = WeatherConfig::default();
        assert!(config.enabled);
        assert!(config.effects);
        assert_eq!(config.refresh_minutes, 30);
        assert_eq!(config.unit, "metric");
    }
}
