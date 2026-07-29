//! Task 2.1.7 — Location configuration types.
//! Port of `LocationConfig` from `src/config/config_types.h`
//! (lines 1258-1270) and `config_schema.cpp:184-195`.

use serde::{Deserialize, Serialize};

/// Port of `LocationConfig` (config_types.h:1258-1270).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocationConfig {
    pub auto_locate: bool,
    pub address: String,
    pub custom_schedule: bool,
    pub sunset: String,
    pub sunrise: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn location_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let loc_tbl = root
            .get("location")
            .expect("[location] table present in example.toml");
        let config: LocationConfig = loc_tbl
            .clone()
            .try_into()
            .expect("[location] parses into LocationConfig");

        assert!(!config.auto_locate);
        assert!(config.address.is_empty());
        assert!(!config.custom_schedule);
        assert!(config.latitude.is_none());
        assert!(config.longitude.is_none());
    }

    #[test]
    fn location_config_defaults_match_cpp() {
        let config = LocationConfig::default();
        assert!(!config.auto_locate);
        assert!(config.address.is_empty());
        assert!(!config.custom_schedule);
        assert!(config.sunset.is_empty());
        assert!(config.sunrise.is_empty());
        assert!(config.latitude.is_none());
        assert!(config.longitude.is_none());
    }
}
