//! Task 2.1.8 — Shell Event Hooks configuration types.
//! Port of `HookKind`, `HooksConfig`, `hookKindFromKey`, and `hookKindKey` from
//! `src/config/config_types.{h,cpp}` (lines 1274-1326; bodies: `config_types.cpp:550-555`).

use serde::{Deserialize, Serialize};

/// Port of `HookKind` (config_types.h:1274-1294).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    Started = 0,
    WallpaperChanged,
    ColorsChanged,
    ThemeModeChanged,
    SessionLocked,
    SessionUnlocked,
    LoggingOut,
    Rebooting,
    ShuttingDown,
    WifiEnabled,
    WifiDisabled,
    BluetoothEnabled,
    BluetoothDisabled,
    BatteryCharging,
    BatteryDischarging,
    BatteryPlugged,
    BatteryPercentageChanged,
    PowerProfileChanged,
}

impl HookKind {
    pub const COUNT: usize = 18;

    pub fn key(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::WallpaperChanged => "wallpaper_changed",
            Self::ColorsChanged => "colors_changed",
            Self::ThemeModeChanged => "theme_mode_changed",
            Self::SessionLocked => "session_locked",
            Self::SessionUnlocked => "session_unlocked",
            Self::LoggingOut => "logging_out",
            Self::Rebooting => "rebooting",
            Self::ShuttingDown => "shutting_down",
            Self::WifiEnabled => "wifi_enabled",
            Self::WifiDisabled => "wifi_disabled",
            Self::BluetoothEnabled => "bluetooth_enabled",
            Self::BluetoothDisabled => "bluetooth_disabled",
            Self::BatteryCharging => "battery_charging",
            Self::BatteryDischarging => "battery_discharging",
            Self::BatteryPlugged => "battery_plugged",
            Self::BatteryPercentageChanged => "battery_percentage_changed",
            Self::PowerProfileChanged => "power_profile_changed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "started" => Some(Self::Started),
            "wallpaper_changed" => Some(Self::WallpaperChanged),
            "colors_changed" => Some(Self::ColorsChanged),
            "theme_mode_changed" => Some(Self::ThemeModeChanged),
            "session_locked" => Some(Self::SessionLocked),
            "session_unlocked" => Some(Self::SessionUnlocked),
            "logging_out" => Some(Self::LoggingOut),
            "rebooting" => Some(Self::Rebooting),
            "shutting_down" => Some(Self::ShuttingDown),
            "wifi_enabled" => Some(Self::WifiEnabled),
            "wifi_disabled" => Some(Self::WifiDisabled),
            "bluetooth_enabled" => Some(Self::BluetoothEnabled),
            "bluetooth_disabled" => Some(Self::BluetoothDisabled),
            "battery_charging" => Some(Self::BatteryCharging),
            "battery_discharging" => Some(Self::BatteryDischarging),
            "battery_plugged" => Some(Self::BatteryPlugged),
            "battery_percentage_changed" => Some(Self::BatteryPercentageChanged),
            "power_profile_changed" => Some(Self::PowerProfileChanged),
            _ => None,
        }
    }
}

/// Port of `hookKindFromKey` (config_types.cpp:550).
#[inline]
pub fn hook_kind_from_key(key: &str) -> Option<HookKind> {
    HookKind::from_key(key)
}

/// Port of `hookKindKey` (config_types.cpp:552-555).
#[inline]
pub fn hook_kind_key(kind: HookKind) -> &'static str {
    kind.key()
}

/// Port of `HooksConfig` (config_types.h:1319-1323).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    pub started: Vec<String>,
    pub wallpaper_changed: Vec<String>,
    pub colors_changed: Vec<String>,
    pub theme_mode_changed: Vec<String>,
    pub session_locked: Vec<String>,
    pub session_unlocked: Vec<String>,
    pub logging_out: Vec<String>,
    pub rebooting: Vec<String>,
    pub shutting_down: Vec<String>,
    pub wifi_enabled: Vec<String>,
    pub wifi_disabled: Vec<String>,
    pub bluetooth_enabled: Vec<String>,
    pub bluetooth_disabled: Vec<String>,
    pub battery_charging: Vec<String>,
    pub battery_discharging: Vec<String>,
    pub battery_plugged: Vec<String>,
    pub battery_percentage_changed: Vec<String>,
    pub power_profile_changed: Vec<String>,
}

impl HooksConfig {
    /// Port of `HooksConfig::commands[static_cast<size_t>(kind)]`'s indexing (the C++ stores an
    /// `std::array<..., HookKind::Count>` member indexed positionally; this struct stores the
    /// same 18 slots as named fields instead, so `HookManager` looks them up by name here).
    pub fn commands(&self, kind: HookKind) -> &[String] {
        match kind {
            HookKind::Started => &self.started,
            HookKind::WallpaperChanged => &self.wallpaper_changed,
            HookKind::ColorsChanged => &self.colors_changed,
            HookKind::ThemeModeChanged => &self.theme_mode_changed,
            HookKind::SessionLocked => &self.session_locked,
            HookKind::SessionUnlocked => &self.session_unlocked,
            HookKind::LoggingOut => &self.logging_out,
            HookKind::Rebooting => &self.rebooting,
            HookKind::ShuttingDown => &self.shutting_down,
            HookKind::WifiEnabled => &self.wifi_enabled,
            HookKind::WifiDisabled => &self.wifi_disabled,
            HookKind::BluetoothEnabled => &self.bluetooth_enabled,
            HookKind::BluetoothDisabled => &self.bluetooth_disabled,
            HookKind::BatteryCharging => &self.battery_charging,
            HookKind::BatteryDischarging => &self.battery_discharging,
            HookKind::BatteryPlugged => &self.battery_plugged,
            HookKind::BatteryPercentageChanged => &self.battery_percentage_changed,
            HookKind::PowerProfileChanged => &self.power_profile_changed,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn hooks_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let hooks_tbl = root
            .get("hooks")
            .expect("[hooks] table present in example.toml");
        let config: HooksConfig = hooks_tbl
            .clone()
            .try_into()
            .expect("[hooks] parses into HooksConfig");

        assert!(config.started.is_empty());
        assert!(config.wallpaper_changed.is_empty());
    }

    #[test]
    fn hook_kind_key_mappings() {
        assert_eq!(hook_kind_from_key("started"), Some(HookKind::Started));
        assert_eq!(
            hook_kind_from_key("power_profile_changed"),
            Some(HookKind::PowerProfileChanged)
        );
        assert_eq!(hook_kind_from_key("unknown"), None);

        assert_eq!(hook_kind_key(HookKind::Started), "started");
        assert_eq!(
            hook_kind_key(HookKind::PowerProfileChanged),
            "power_profile_changed"
        );
    }
}
