//! Task 2.1.9 — Plugins configuration types & plugin source helpers.
//! Port of `PluginSourceKind`, `PluginSourceConfig`, `PluginsConfig`,
//! `defaultPluginSources`, `isDefaultPluginSourceName`, and `isValidPluginSourceName`
//! from `src/config/config_types.{h,cpp}` (structs: lines 1464-1506; bodies:
//! `config_types.cpp:59-90`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::widget_setting_value::WidgetSettingValue;

/// Port of `PluginSourceKind` (config_types.h:1468-1476).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSourceKind {
    #[default]
    Git = 0,
    Path = 1,
}

/// Port of `PluginSourceConfig` (config_types.h:1478-1484).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginSourceConfig {
    pub kind: PluginSourceKind,
    pub name: String,
    pub location: String,
    pub enabled: bool,
}

impl Default for PluginSourceConfig {
    fn default() -> Self {
        Self {
            kind: PluginSourceKind::Git,
            name: String::new(),
            location: String::new(),
            enabled: true,
        }
    }
}

/// Port of `PluginsConfig` (config_types.h:1488-1498).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    #[serde(rename = "source")]
    pub sources: Vec<PluginSourceConfig>,
    pub enabled: Vec<String>,
    #[serde(rename = "auto_update")]
    pub auto_update: bool,
    /// Keyed by plugin id ("author/plugin") then setting key.
    #[serde(skip)]
    pub plugin_settings: HashMap<String, HashMap<String, WidgetSettingValue>>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            sources: default_plugin_sources(),
            enabled: Vec::new(),
            auto_update: true,
            plugin_settings: HashMap::new(),
        }
    }
}

/// Port of `defaultPluginSources` (config_types.cpp:59-68).
pub fn default_plugin_sources() -> Vec<PluginSourceConfig> {
    vec![
        PluginSourceConfig {
            kind: PluginSourceKind::Git,
            name: "official".to_string(),
            location: "https://github.com/noctalia-dev/official-plugins".to_string(),
            enabled: true,
        },
        PluginSourceConfig {
            kind: PluginSourceKind::Git,
            name: "community".to_string(),
            location: "https://github.com/noctalia-dev/community-plugins".to_string(),
            enabled: true,
        },
    ]
}

/// Port of `isDefaultPluginSourceName` (config_types.cpp:70-73).
pub fn is_default_plugin_source_name(name: &str) -> bool {
    name == "official" || name == "community"
}

/// Port of `isValidPluginSourceName` (config_types.cpp:75-91).
pub fn is_valid_plugin_source_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn plugins_deserializes_from_toml() {
        let toml_str = r#"
            auto_update = false
            enabled = ["author/plugin1"]
        "#;
        let config: PluginsConfig = toml::from_str(toml_str).expect("parses into PluginsConfig");

        assert!(!config.auto_update);
        assert_eq!(config.enabled, vec!["author/plugin1"]);
    }

    #[test]
    fn default_plugin_sources_matches_cpp() {
        let sources = default_plugin_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "official");
        assert_eq!(sources[1].name, "community");

        assert!(is_default_plugin_source_name("official"));
        assert!(is_default_plugin_source_name("community"));
        assert!(!is_default_plugin_source_name("custom"));
    }

    #[test]
    fn is_valid_plugin_source_name_matches_cpp_rules() {
        assert!(is_valid_plugin_source_name("official"));
        assert!(is_valid_plugin_source_name("my-repo_1.0"));
        assert!(!is_valid_plugin_source_name(""));
        assert!(!is_valid_plugin_source_name("."));
        assert!(!is_valid_plugin_source_name(".."));
        assert!(!is_valid_plugin_source_name("-invalid"));
        assert!(!is_valid_plugin_source_name("invalid/slash"));
    }
}
