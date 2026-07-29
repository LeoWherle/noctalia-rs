//! Task 2.1.4 — Wallpaper configuration types.
//! Port of `WallpaperMonitorOverride`, `WallpaperAutomationConfig`,
//! `WallpaperConfig`, `WallpaperFillMode`, `WallpaperTransition`,
//! `WallpaperFavorite`, `PaletteSource`, and `ThemeMode` from
//! `src/config/config_types.{h,cpp}` (structs: lines 421-482, 1328-1364;
//! schema: `config_schema.cpp:722-756, 1495-1514`).

use noctalia_core::color::ColorSpec;
use serde::{Deserialize, Serialize};

/// Port of `WallpaperFillMode` (config_types.h:421-428).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperFillMode {
    Center = 0,
    #[default]
    Crop = 1,
    Fit = 2,
    Stretch = 3,
    Repeat = 4,
    Span = 5,
}

/// Port of `WallpaperTransition` (config_types.h:430-437).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperTransition {
    #[default]
    Fade = 0,
    Wipe = 1,
    Disc = 2,
    Stripes = 3,
    Zoom = 4,
    Honeycomb = 5,
}

/// Port of `WallpaperAutomationConfig::Order` (config_types.h:451-454).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperAutomationOrder {
    #[default]
    Random = 0,
    Alphabetical = 1,
}

/// Port of `PaletteSource` (config_types.h:1328-1333).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaletteSource {
    #[default]
    Builtin = 0,
    Wallpaper = 1,
    Community = 2,
    Custom = 3,
}

/// Port of `ThemeMode` (config_types.h:1342-1346).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Dark = 0,
    Light = 1,
    #[default]
    Auto = 2,
}

/// Port of `WallpaperMonitorOverride` (config_types.h:439-448).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperMonitorOverride {
    #[serde(rename = "match")]
    pub match_: String,
    pub enabled: Option<bool>,
    #[serde(with = "crate::types::serde_support::optional_color_spec_serde")]
    pub fill_color: Option<ColorSpec>,
    pub directory: Option<String>,
    pub directory_light: Option<String>,
    pub directory_dark: Option<String>,
}

/// Port of `WallpaperAutomationConfig` (config_types.h:450-462).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperAutomationConfig {
    pub enabled: bool,
    pub interval_seconds: i32,
    pub order: WallpaperAutomationOrder,
    pub recursive: bool,
}

impl Default for WallpaperAutomationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 1800,
            order: WallpaperAutomationOrder::Random,
            recursive: true,
        }
    }
}

pub fn default_wallpaper_transitions() -> Vec<WallpaperTransition> {
    vec![
        WallpaperTransition::Fade,
        WallpaperTransition::Wipe,
        WallpaperTransition::Disc,
        WallpaperTransition::Stripes,
        WallpaperTransition::Zoom,
        WallpaperTransition::Honeycomb,
    ]
}

/// Port of `WallpaperConfig` (config_types.h:464-482).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    pub enabled: bool,
    pub fill_mode: WallpaperFillMode,
    #[serde(with = "crate::types::serde_support::optional_color_spec_serde")]
    pub fill_color: Option<ColorSpec>,
    #[serde(rename = "transition")]
    pub transitions: Vec<WallpaperTransition>,
    #[serde(rename = "transition_duration")]
    pub transition_duration_ms: f32,
    pub edge_smoothness: f32,
    pub transition_on_startup: bool,
    pub directory: String,
    pub directory_light: String,
    pub directory_dark: String,
    pub per_monitor_directories: bool,
    pub automation: WallpaperAutomationConfig,
    /// Populated from `[wallpaper.monitor.<name>]` table map during whole-config assembly (task 2.3).
    #[serde(skip)]
    pub monitor_overrides: Vec<WallpaperMonitorOverride>,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fill_mode: WallpaperFillMode::Crop,
            fill_color: None,
            transitions: default_wallpaper_transitions(),
            transition_duration_ms: 1500.0,
            edge_smoothness: 0.3,
            transition_on_startup: false,
            directory: String::new(),
            directory_light: String::new(),
            directory_dark: String::new(),
            per_monitor_directories: false,
            automation: WallpaperAutomationConfig::default(),
            monitor_overrides: Vec::new(),
        }
    }
}

/// Port of `WallpaperFavorite` (config_types.h:1354-1364).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperFavorite {
    pub path: String,
    pub theme_mode: ThemeMode,
    pub palette_source: Option<PaletteSource>,
    pub builtin_palette: String,
    pub community_palette: String,
    pub custom_palette: String,
    pub wallpaper_scheme: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn wallpaper_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let wallpaper_tbl = root
            .get("wallpaper")
            .expect("[wallpaper] table present in example.toml");
        let config: WallpaperConfig = wallpaper_tbl
            .clone()
            .try_into()
            .expect("[wallpaper] parses into WallpaperConfig");

        assert!(config.enabled);
        assert_eq!(config.fill_mode, WallpaperFillMode::Crop);
        assert!(config.fill_color.is_none());
        assert_eq!(
            config.transitions,
            vec![
                WallpaperTransition::Fade,
                WallpaperTransition::Wipe,
                WallpaperTransition::Disc,
                WallpaperTransition::Stripes,
                WallpaperTransition::Zoom,
                WallpaperTransition::Honeycomb,
            ]
        );
        assert_eq!(config.transition_duration_ms, 1500.0);
        assert_eq!(config.edge_smoothness, 0.3);
        assert!(!config.transition_on_startup);
        assert_eq!(config.directory, "~/Pictures/Wallpapers");

        assert!(!config.automation.enabled);
        assert_eq!(config.automation.interval_seconds, 1800);
        assert_eq!(config.automation.order, WallpaperAutomationOrder::Random);
        assert!(config.automation.recursive);
    }

    #[test]
    fn wallpaper_config_defaults_match_cpp() {
        let config = WallpaperConfig::default();
        assert!(config.enabled);
        assert_eq!(config.fill_mode, WallpaperFillMode::Crop);
        assert!(config.fill_color.is_none());
        assert_eq!(config.transitions, default_wallpaper_transitions());
        assert_eq!(config.transition_duration_ms, 1500.0);
        assert_eq!(config.edge_smoothness, 0.3);
        assert!(!config.transition_on_startup);
        assert_eq!(config.directory, "");
        assert_eq!(config.directory_light, "");
        assert_eq!(config.directory_dark, "");
        assert!(!config.per_monitor_directories);
        assert!(!config.automation.enabled);
        assert_eq!(config.automation.interval_seconds, 1800);
        assert_eq!(config.automation.order, WallpaperAutomationOrder::Random);
        assert!(config.automation.recursive);
        assert!(config.monitor_overrides.is_empty());
    }

    #[test]
    fn wallpaper_enums_serde_roundtrip() {
        assert_eq!(
            serde_json::from_str::<WallpaperFillMode>("\"crop\"").unwrap(),
            WallpaperFillMode::Crop
        );
        assert_eq!(
            serde_json::from_str::<WallpaperTransition>("\"fade\"").unwrap(),
            WallpaperTransition::Fade
        );
        assert_eq!(
            serde_json::from_str::<WallpaperAutomationOrder>("\"random\"").unwrap(),
            WallpaperAutomationOrder::Random
        );
        assert_eq!(
            serde_json::from_str::<PaletteSource>("\"builtin\"").unwrap(),
            PaletteSource::Builtin
        );
        assert_eq!(
            serde_json::from_str::<ThemeMode>("\"auto\"").unwrap(),
            ThemeMode::Auto
        );
    }
}
