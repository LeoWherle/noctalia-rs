//! Task 2.1.7 — Storage configuration types.
//! Port of `StorageKeySource` and `StorageConfig` from `src/config/config_types.h`
//! (lines 730-743, 1059-1064).

use serde::{Deserialize, Serialize};

/// Port of `StorageKeySource` (config_types.h:730-743).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageKeySource {
    #[default]
    SecretService = 0,
    File = 1,
}

/// Port of `StorageConfig` (config_types.h:1059-1064).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub key_source: StorageKeySource,
    pub key_file: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn storage_config_defaults_match_cpp() {
        let config = StorageConfig::default();
        assert_eq!(config.key_source, StorageKeySource::SecretService);
        assert!(config.key_file.is_empty());
    }

    #[test]
    fn storage_key_source_serde_roundtrip() {
        assert_eq!(
            serde_json::from_str::<StorageKeySource>("\"secret_service\"").unwrap(),
            StorageKeySource::SecretService
        );
        assert_eq!(
            serde_json::from_str::<StorageKeySource>("\"file\"").unwrap(),
            StorageKeySource::File
        );
    }
}
