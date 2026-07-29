//! Task 2.1.9 — Top-level assembled `Config` struct & `ConfigChangeSet`.
//! Port of `Config` and `ConfigChangeSet` from `src/config/config_types.h`
//! (lines 1533-1625). Deserializes the whole `example.toml` losslessly!

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::accessibility::AccessibilityConfig;
use crate::types::audio::AudioConfig;
use crate::types::backdrop::BackdropConfig;
use crate::types::bar::{BarConfig, WidgetConfig};
use crate::types::battery::BatteryConfig;
use crate::types::brightness::BrightnessConfig;
use crate::types::calendar::CalendarConfig;
use crate::types::control_center::ControlCenterConfig;
use crate::types::desktop_widgets::{DesktopWidgetsConfig, LockscreenWidgetsConfig};
use crate::types::dock::DockConfig;
use crate::types::hooks::HooksConfig;
use crate::types::hotcorners::HotCornersConfig;
use crate::types::idle::IdleConfig;
use crate::types::keybinds::KeybindsConfig;
use crate::types::location::LocationConfig;
use crate::types::lockscreen::LockscreenConfig;
use crate::types::nightlight::NightLightConfig;
use crate::types::notification::NotificationConfig;
use crate::types::osd::OsdConfig;
use crate::types::plugins::PluginsConfig;
use crate::types::shell::ShellConfig;
use crate::types::storage::StorageConfig;
use crate::types::system::SystemConfig;
use crate::types::theme::ThemeConfig;
use crate::types::wallpaper::WallpaperConfig;
use crate::types::weather::WeatherConfig;

/// Port of `Config` (config_types.h:1533-1562).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Populated from `[bar.<id>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub bars: Vec<BarConfig>,
    /// Populated from `[widget.<id>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub widgets: HashMap<String, WidgetConfig>,
    pub wallpaper: WallpaperConfig,
    pub backdrop: BackdropConfig,
    pub lockscreen: LockscreenConfig,
    pub lockscreen_widgets: LockscreenWidgetsConfig,
    pub dock: DockConfig,
    pub desktop_widgets: DesktopWidgetsConfig,
    pub hot_corners: HotCornersConfig,
    pub storage: StorageConfig,
    pub shell: ShellConfig,
    pub osd: OsdConfig,
    pub notification: NotificationConfig,
    pub weather: WeatherConfig,
    pub calendar: CalendarConfig,
    pub system: SystemConfig,
    pub audio: AudioConfig,
    pub brightness: BrightnessConfig,
    pub battery: BatteryConfig,
    pub keybinds: KeybindsConfig,
    pub nightlight: NightLightConfig,
    pub location: LocationConfig,
    pub idle: IdleConfig,
    pub hooks: HooksConfig,
    pub theme: ThemeConfig,
    pub control_center: ControlCenterConfig,
    pub plugins: PluginsConfig,
    pub accessibility: AccessibilityConfig,
}

/// Port of `ConfigChangeSet` (config_types.h:1566-1625).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChangeSet {
    pub bars: bool,
    pub widgets: bool,
    pub desktop_widgets: bool,
    pub lockscreen_widgets: bool,
    pub wallpaper: bool,
    pub backdrop: bool,
    pub lockscreen: bool,
    pub dock: bool,
    pub shell: bool,
    pub osd: bool,
    pub notification: bool,
    pub weather: bool,
    pub calendar: bool,
    pub system: bool,
    pub audio: bool,
    pub brightness: bool,
    pub battery: bool,
    pub keybinds: bool,
    pub nightlight: bool,
    pub location: bool,
    pub idle: bool,
    pub hooks: bool,
    pub theme: bool,
    pub control_center: bool,
    pub plugins: bool,
    pub hot_corners: bool,
    pub storage: bool,
    pub accessibility: bool,
}

impl Default for ConfigChangeSet {
    fn default() -> Self {
        Self {
            bars: true,
            widgets: true,
            desktop_widgets: true,
            lockscreen_widgets: true,
            wallpaper: true,
            backdrop: true,
            lockscreen: true,
            dock: true,
            shell: true,
            osd: true,
            notification: true,
            weather: true,
            calendar: true,
            system: true,
            audio: true,
            brightness: true,
            battery: true,
            keybinds: true,
            nightlight: true,
            location: true,
            idle: true,
            hooks: true,
            theme: true,
            control_center: true,
            plugins: true,
            hot_corners: true,
            storage: true,
            accessibility: true,
        }
    }
}

impl ConfigChangeSet {
    /// Port of `ConfigChangeSet::any()` (config_types.h:1596-1624).
    pub fn any(self) -> bool {
        self.bars
            || self.widgets
            || self.desktop_widgets
            || self.lockscreen_widgets
            || self.wallpaper
            || self.backdrop
            || self.lockscreen
            || self.dock
            || self.shell
            || self.osd
            || self.notification
            || self.weather
            || self.calendar
            || self.system
            || self.audio
            || self.brightness
            || self.battery
            || self.keybinds
            || self.nightlight
            || self.location
            || self.idle
            || self.hooks
            || self.theme
            || self.control_center
            || self.plugins
            || self.hot_corners
            || self.storage
            || self.accessibility
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn whole_example_toml_deserializes_losslessly_into_config() {
        let config: Config =
            toml::from_str(EXAMPLE_TOML).expect("example.toml parses into whole Config struct!");

        assert_eq!(config.accessibility.ui_scale, 1.0);
        assert!(!config.accessibility.high_contrast);
        assert_eq!(config.shell.corner_radius_scale, 1.0);
        assert!(!config.weather.enabled);
        assert_eq!(config.weather.unit, "celsius");
        assert!(!config.audio.enable_overdrive);
        assert_eq!(config.audio.sound_volume, 0.5);
        assert!(!config.brightness.enable_ddcutil);
        assert!(!config.nightlight.enabled);
        assert_eq!(config.nightlight.day_temperature, 6500);
        assert!(!config.location.auto_locate);
        assert!(config.osd.enabled);
        assert_eq!(config.osd.position, "top_right");
        assert!(config.notification.enable_daemon);
        assert_eq!(config.notification.layer, "top");
        assert_eq!(config.theme.builtin_palette, "Noctalia");
        assert_eq!(config.control_center.width, 700);
    }

    #[test]
    fn config_change_set_default_is_all_true_and_any_returns_true() {
        let cs = ConfigChangeSet::default();
        assert!(cs.any());
    }

    #[test]
    fn config_change_set_all_false_any_returns_false() {
        let cs = ConfigChangeSet {
            bars: false,
            widgets: false,
            desktop_widgets: false,
            lockscreen_widgets: false,
            wallpaper: false,
            backdrop: false,
            lockscreen: false,
            dock: false,
            shell: false,
            osd: false,
            notification: false,
            weather: false,
            calendar: false,
            system: false,
            audio: false,
            brightness: false,
            battery: false,
            keybinds: false,
            nightlight: false,
            location: false,
            idle: false,
            hooks: false,
            theme: false,
            control_center: false,
            plugins: false,
            hot_corners: false,
            storage: false,
            accessibility: false,
        };
        assert!(!cs.any());
    }
}
