//! Task 2.1.8 — Calendar configuration types.
//! Port of `CalendarCredentialSource`, `CalendarConfig::Account`, and `CalendarConfig`
//! from `src/config/config_types.h` (lines 1066-1094).

use serde::{Deserialize, Serialize};

/// Port of `CalendarCredentialSource` (config_types.h:1066-1069).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarCredentialSource {
    #[default]
    SecretService = 0,
    File = 1,
}

/// Port of `CalendarConfig::Account` (config_types.h:1074-1087).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarAccountConfig {
    /// Table key in `[calendar.account.<id>]`.
    #[serde(skip)]
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub display_name: String,
    pub color: String,
    pub provider: String,
    pub server_url: String,
    pub username: String,
    pub calendars: Vec<String>,
    pub credential_source: CalendarCredentialSource,
    pub password_file: String,
}

/// Port of `CalendarConfig` (config_types.h:1071-1094).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarConfig {
    pub enabled: bool,
    pub refresh_minutes: i32,
    /// Populated from `[calendar.account.<id>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub accounts: Vec<CalendarAccountConfig>,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            refresh_minutes: 15,
            accounts: Vec::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn calendar_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let cal_tbl = root
            .get("calendar")
            .expect("[calendar] table present in example.toml");
        let config: CalendarConfig = cal_tbl
            .clone()
            .try_into()
            .expect("[calendar] parses into CalendarConfig");

        assert!(!config.enabled);
        assert_eq!(config.refresh_minutes, 15);
    }

    #[test]
    fn calendar_config_defaults_match_cpp() {
        let config = CalendarConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.refresh_minutes, 15);
        assert!(config.accounts.is_empty());
    }

    #[test]
    fn calendar_credential_source_serde_roundtrip() {
        assert_eq!(
            serde_json::from_str::<CalendarCredentialSource>("\"secret_service\"").unwrap(),
            CalendarCredentialSource::SecretService
        );
        assert_eq!(
            serde_json::from_str::<CalendarCredentialSource>("\"file\"").unwrap(),
            CalendarCredentialSource::File
        );
    }
}
