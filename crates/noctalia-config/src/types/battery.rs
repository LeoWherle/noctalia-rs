//! Task 2.1.7 — Battery configuration types.
//! Port of `BatteryDeviceWarningThreshold` and `BatteryConfig` from
//! `src/config/config_types.h` (lines 1215-1229) and `config_schema.cpp:435-440`.

use serde::{Deserialize, Serialize};

/// Port of `BatteryDeviceWarningThreshold` (config_types.h:1215-1221).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryDeviceWarningThreshold {
    /// Table key in `[battery.device.<selector>]`.
    #[serde(skip)]
    pub selector: String,
    pub warning_threshold: i32,
}

impl Default for BatteryDeviceWarningThreshold {
    fn default() -> Self {
        Self {
            selector: String::new(),
            warning_threshold: 10,
        }
    }
}

/// Port of `BatteryConfig` (config_types.h:1223-1229).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryConfig {
    pub warning_threshold: i32,
    /// Populated from `[battery.device.<selector>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub device_thresholds: Vec<BatteryDeviceWarningThreshold>,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            warning_threshold: 10,
            device_thresholds: Vec::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn battery_config_defaults_match_cpp() {
        let config = BatteryConfig::default();
        assert_eq!(config.warning_threshold, 10);
        assert!(config.device_thresholds.is_empty());
    }
}
