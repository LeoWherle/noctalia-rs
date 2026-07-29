//! Task 2.1.9 — Theme configuration types.
//! Port of `TemplateColorConfig`, `TemplateInputPathModesConfig`,
//! `TemplateCompareColorConfig`, `UserTemplateConfig`, `TemplatesConfig`, and `ThemeConfig`
//! from `src/config/config_types.h` (lines 1378-1441) and `config_schema.cpp:522-680`.

use serde::{Deserialize, Serialize};

use crate::types::wallpaper::{PaletteSource, ThemeMode};

/// Port of `ThemeConfig::TemplateColorConfig` (config_types.h:1379-1387).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateColorConfig {
    /// Table key in `[theme.templates.custom_color.<name>]`.
    #[serde(skip)]
    pub name: String,
    pub color: String,
    pub color_dark: String,
    pub color_light: String,
    pub blend: bool,
}

impl Default for TemplateColorConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            color: String::new(),
            color_dark: String::new(),
            color_light: String::new(),
            blend: true,
        }
    }
}

/// Port of `ThemeConfig::TemplateInputPathModesConfig` (config_types.h:1389-1394).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateInputPathModesConfig {
    pub dark: String,
    pub light: String,
}

/// Port of `ThemeConfig::TemplateCompareColorConfig` (config_types.h:1396-1401).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateCompareColorConfig {
    pub name: String,
    pub color: String,
}

/// Port of `ThemeConfig::UserTemplateConfig` (config_types.h:1403-1418).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserTemplateConfig {
    /// Table key in `[theme.templates.template.<id>]`.
    #[serde(skip)]
    pub id: String,
    pub enabled: bool,
    pub input_path: String,
    pub input_path_modes: Option<TemplateInputPathModesConfig>,
    pub output_paths: Vec<String>,
    pub output_path_dynamic: String,
    pub compare_to: String,
    pub colors_to_compare: Vec<TemplateCompareColorConfig>,
    pub pre_hook: String,
    pub post_hook: String,
    pub post_action: String,
    pub index: i32,
}

impl Default for UserTemplateConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: true,
            input_path: String::new(),
            input_path_modes: None,
            output_paths: Vec::new(),
            output_path_dynamic: String::new(),
            compare_to: String::new(),
            colors_to_compare: Vec::new(),
            pre_hook: String::new(),
            post_hook: String::new(),
            post_action: String::new(),
            index: 0,
        }
    }
}

/// Port of `ThemeConfig::TemplatesConfig` (config_types.h:1420-1429).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplatesConfig {
    pub enable_builtin_templates: bool,
    pub builtin_ids: Vec<String>,
    pub enable_community_templates: bool,
    pub community_ids: Vec<String>,
    /// Populated from `[theme.templates.custom_color.<name>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub custom_colors: Vec<TemplateColorConfig>,
    /// Populated from `[theme.templates.template.<id>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub user_templates: Vec<UserTemplateConfig>,
}

impl Default for TemplatesConfig {
    fn default() -> Self {
        Self {
            enable_builtin_templates: true,
            builtin_ids: Vec::new(),
            enable_community_templates: true,
            community_ids: Vec::new(),
            custom_colors: Vec::new(),
            user_templates: Vec::new(),
        }
    }
}

/// Port of `ThemeConfig` (config_types.h:1378-1441).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub source: PaletteSource,
    pub builtin_palette: String,
    pub community_palette: String,
    pub custom_palette: String,
    pub wallpaper_scheme: String,
    pub mode: ThemeMode,
    pub pure_black_dark: bool,
    pub templates: TemplatesConfig,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            source: PaletteSource::Builtin,
            builtin_palette: "Noctalia".to_string(),
            community_palette: "Oxocarbon".to_string(),
            custom_palette: String::new(),
            wallpaper_scheme: "m3-content".to_string(),
            mode: ThemeMode::Dark,
            pure_black_dark: false,
            templates: TemplatesConfig::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn theme_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let theme_tbl = root
            .get("theme")
            .expect("[theme] table present in example.toml");
        let config: ThemeConfig = theme_tbl
            .clone()
            .try_into()
            .expect("[theme] parses into ThemeConfig");

        assert_eq!(config.source, PaletteSource::Builtin);
        assert_eq!(config.builtin_palette, "Noctalia");
        assert_eq!(config.mode, ThemeMode::Dark);
        assert!(config.templates.enable_builtin_templates);
    }

    #[test]
    fn theme_config_defaults_match_cpp() {
        let config = ThemeConfig::default();
        assert_eq!(config.source, PaletteSource::Builtin);
        assert_eq!(config.builtin_palette, "Noctalia");
        assert_eq!(config.community_palette, "Oxocarbon");
        assert_eq!(config.wallpaper_scheme, "m3-content");
        assert_eq!(config.mode, ThemeMode::Dark);
        assert!(!config.pure_black_dark);
        assert!(config.templates.enable_builtin_templates);
        assert!(config.templates.enable_community_templates);
    }
}
