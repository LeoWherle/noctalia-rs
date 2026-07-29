//! Task 2.1.5 — OSD configuration types.
//! Port of `OsdKindsConfig` and `OsdConfig` from `src/config/config_types.h`
//! (lines 650-683) and `config_schema.cpp:42-78`.

use serde::{Deserialize, Serialize};

/// Port of `OsdKindsConfig` (config_types.h:650-667).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdKindsConfig {
    pub volume: bool,
    pub volume_output: bool,
    pub volume_input: bool,
    pub brightness: bool,
    pub wifi: bool,
    pub bluetooth: bool,
    pub power_profile: bool,
    pub caffeine: bool,
    pub nightlight: bool,
    pub dnd: bool,
    pub lock_keys: bool,
    pub keyboard_layout: bool,
    pub media: bool,
    pub privacy: bool,
    pub keyboard_backlight: bool,
}

impl Default for OsdKindsConfig {
    fn default() -> Self {
        Self {
            volume: true,
            volume_output: true,
            volume_input: true,
            brightness: true,
            wifi: true,
            bluetooth: true,
            power_profile: true,
            caffeine: true,
            nightlight: true,
            dnd: true,
            lock_keys: true,
            keyboard_layout: true,
            media: true,
            privacy: true,
            keyboard_backlight: true,
        }
    }
}

/// Port of `OsdConfig` (config_types.h:669-683).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdConfig {
    pub enabled: bool,
    pub position: String,
    pub position_vertical: String,
    pub orientation: String,
    pub scale: f32,
    pub background_opacity: f32,
    pub border: bool,
    pub offset_x: i32,
    pub offset_y: i32,
    pub monitors: Vec<String>,
    pub kinds: OsdKindsConfig,
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            position: "top_center".to_string(),
            position_vertical: "top_center".to_string(),
            orientation: "horizontal".to_string(),
            scale: 1.0,
            background_opacity: 0.97,
            border: true,
            offset_x: 20,
            offset_y: 8,
            monitors: Vec::new(),
            kinds: OsdKindsConfig::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn osd_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let osd_tbl = root
            .get("osd")
            .expect("[osd] table present in example.toml");
        let config: OsdConfig = osd_tbl
            .clone()
            .try_into()
            .expect("[osd] parses into OsdConfig");

        assert!(config.enabled);
        assert_eq!(config.position, "top_right");
        assert_eq!(config.position_vertical, "top_center");
        assert_eq!(config.orientation, "horizontal");
        assert_eq!(config.scale, 1.0);
        assert_eq!(config.background_opacity, 0.97);
        assert_eq!(config.offset_x, 20);
        assert_eq!(config.offset_y, 8);

        assert!(config.kinds.volume);
        assert!(config.kinds.volume_output);
        assert!(config.kinds.volume_input);
        assert!(config.kinds.brightness);
        assert!(config.kinds.wifi);
        assert!(config.kinds.bluetooth);
        assert!(config.kinds.power_profile);
        assert!(config.kinds.caffeine);
        assert!(config.kinds.nightlight);
        assert!(config.kinds.dnd);
        assert!(config.kinds.lock_keys);
        assert!(config.kinds.keyboard_layout);
        assert!(config.kinds.privacy);
    }

    #[test]
    fn osd_config_defaults_match_cpp() {
        let config = OsdConfig::default();
        assert!(config.enabled);
        assert_eq!(config.position, "top_center");
        assert_eq!(config.position_vertical, "top_center");
        assert_eq!(config.orientation, "horizontal");
        assert_eq!(config.scale, 1.0);
        assert_eq!(config.background_opacity, 0.97);
        assert!(config.border);
        assert_eq!(config.offset_x, 20);
        assert_eq!(config.offset_y, 8);
        assert!(config.monitors.is_empty());
        assert!(config.kinds.keyboard_backlight);
    }
}
