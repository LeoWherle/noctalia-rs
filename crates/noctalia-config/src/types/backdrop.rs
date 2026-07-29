//! Task 2.1.4 — Backdrop configuration type.
//! Port of `BackdropConfig` from `src/config/config_types.h` (lines 484-490)
//! and `config_schema.cpp:80-87`.

use serde::{Deserialize, Serialize};

/// Port of `BackdropConfig` (config_types.h:484-490).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackdropConfig {
    pub enabled: bool,
    pub blur_intensity: f32,
    pub tint_intensity: f32,
}

impl Default for BackdropConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            blur_intensity: 0.5,
            tint_intensity: 0.3,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn backdrop_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let backdrop_tbl = root
            .get("backdrop")
            .expect("[backdrop] table present in example.toml");
        let config: BackdropConfig = backdrop_tbl
            .clone()
            .try_into()
            .expect("[backdrop] parses into BackdropConfig");

        assert!(!config.enabled);
        assert_eq!(config.blur_intensity, 0.5);
        assert_eq!(config.tint_intensity, 0.3);
    }

    #[test]
    fn backdrop_config_defaults_match_cpp() {
        let config = BackdropConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.blur_intensity, 0.5);
        assert_eq!(config.tint_intensity, 0.3);
    }
}
