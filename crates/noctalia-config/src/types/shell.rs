//! Task 2.1.3 — shell config. Port of `ShellConfig` and its nested
//! `AnimationConfig`/`ShadowConfig`/`PanelConfig`/`LauncherConfig`
//! (+`DmenuConfig`)/`ScreenCornersConfig`/`MprisConfig`/`ScreenshotConfig`/
//! `PrivacyConfig`, `ShellSessionConfig`+`ShellSessionPowerConfig`,
//! `ShellGreeterSyncConfig`, `ShortcutConfig`, `SessionPanelActionConfig`,
//! `DmenuEntryConfig`, `LauncherProviderConfig`, `defaultSessionPanelActions`,
//! `defaultControlCenterShortcuts`, from `src/config/config_types.{h,cpp}`.
//!
//! Same scope boundary as task 2.1.2 (`bar.rs`): data model only — struct
//! shapes, in-code defaults matching the C++ member initializers, and the
//! two default-list free functions. Enum-like C++ fields
//! (`PasswordMaskStyle`/`ClipboardAutoPasteMode`/`ShadowDirection`/
//! `PanelTransparencyMode`/`PanelPlacement`/`SessionActionButtonVariant`) are
//! kept as plain `String`s holding the exact `config_schema.cpp` `enumField`
//! key (not the C++ enum's own variant name) — same "string-vs-enum
//! validation is schema-engine territory" call `bar.rs` already made for
//! `BarConfig::layer`/`position`; a real Rust enum + warn-diagnostics on a
//! bad value is task 2.3's job.
//!
//! `panelCardOpacityForTransparencyMode`/
//! `detachedPanelBackgroundOpacityForTransparencyMode`
//! (config_types.cpp:174-197) and `ShadowDirectionOffset`/
//! `shadowDirectionOffset` (config_types.h:769-796) are deliberately not
//! ported here despite living right next to `PanelTransparencyMode`/
//! `ShadowDirection` in the C++: neither is listed in this task's plan text,
//! both need a real enum type to switch over (not a `String`), and their
//! only consumer (`src/shell/panel/panel_surface_style.h`) is UI/rendering,
//! a later phase — left for whichever task ports that.
//!
//! `SessionPanelActionConfig::shortcut` is `#[serde(skip)]`: converting the
//! TOML string form needs `parseKeyChordSpec`/`keyChordToString`
//! (`core/input/key_chord.cpp`), which need `xkbcommon` FFI and stay task
//! 10.2's job (see `noctalia_core::input`'s own doc comment — its `KeyChord`
//! POD was pulled forward from that task for exactly this field).
//! `default_session_panel_actions` still constructs real `KeyChord` values
//! directly (bypassing string parsing entirely, same as the C++'s own
//! `KeyChord{.sym = XKB_KEY_1}` literals) — the five keysym values used
//! there are verified against this host's installed `xkbcommon-keysyms.h`
//! (`XKB_KEY_1`..`XKB_KEY_5` = `0x0031`..`0x0035`, the ASCII digit
//! codepoints) rather than pulled in via an `xkbcommon` crate dependency for
//! five constants.
//!
//! `DmenuEntryConfig::id` and `LauncherProviderConfig::name` are
//! `#[serde(skip)]`, same "populated from the enclosing table-map key, not
//! this struct's own derive" reasoning as `BarConfig::name` — both are
//! assembled by `config_schema.cpp`'s `namedMap`
//! (`[shell.launcher.dmenu.entry.<id>]` / `[shell.launcher.providers.<name>]`),
//! which is schema-engine territory (task 2.3), not this task's. Likewise
//! `LauncherConfig::providers` and `DmenuConfig::entries` themselves are
//! `#[serde(skip)]` — the not-yet-derivable *result* of that same
//! map assembly, same pattern as `BarConfig::monitor_overrides`.

use serde::{Deserialize, Serialize};

use noctalia_core::color::ColorSpec;
use noctalia_core::input::KeyChord;
use noctalia_core::limits::CLIPBOARD_HISTORY_DEFAULT_ENTRIES;

use crate::types::serde_support::optional_color_spec_serde;

/// Port of `ShortcutConfig` (config_types.h:192-195).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShortcutConfig {
    #[serde(rename = "type")]
    pub r#type: String,
}

/// Port of `defaultControlCenterShortcuts` (config_types.cpp:53-57).
#[must_use]
pub fn default_control_center_shortcuts() -> Vec<ShortcutConfig> {
    [
        "wifi",
        "bluetooth",
        "caffeine",
        "nightlight",
        "notification",
        "power_profile",
    ]
    .into_iter()
    .map(|kind| ShortcutConfig {
        r#type: kind.to_string(),
    })
    .collect()
}

/// Port of `SessionPanelActionConfig` (config_types.h:206-220).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SessionPanelActionConfig {
    pub action: String,
    pub enabled: bool,
    pub command: Option<String>,
    pub label: Option<String>,
    pub glyph: Option<String>,
    /// "default" | "primary" | "secondary" | "destructive" | "outline" | "ghost".
    pub variant: String,
    /// See module doc comment: not read from/written to TOML here.
    #[serde(skip)]
    pub shortcut: Option<KeyChord>,
    pub countdown_seconds: f64,
}

impl Default for SessionPanelActionConfig {
    fn default() -> Self {
        Self {
            action: String::new(),
            enabled: true,
            command: None,
            label: None,
            glyph: None,
            variant: "default".to_string(),
            shortcut: None,
            countdown_seconds: 0.0,
        }
    }
}

/// The `XKB_KEY_1`..`XKB_KEY_5` keysym values `defaultSessionPanelActions`
/// assigns (config_types.cpp:93-117), verified against this host's installed
/// `xkbcommon-keysyms.h`: the ASCII digit codepoints `'1'`..`'5'`
/// (`0x0031`..`0x0035`).
mod default_action_keysyms {
    pub const KEY_1: u32 = 0x0031;
    pub const KEY_2: u32 = 0x0032;
    pub const KEY_3: u32 = 0x0033;
    pub const KEY_4: u32 = 0x0034;
    pub const KEY_5: u32 = 0x0035;
}

/// Port of `defaultSessionPanelActions` (config_types.cpp:93-117).
#[must_use]
pub fn default_session_panel_actions() -> Vec<SessionPanelActionConfig> {
    use default_action_keysyms::{KEY_1, KEY_2, KEY_3, KEY_4, KEY_5};

    vec![
        SessionPanelActionConfig {
            action: "lock".to_string(),
            shortcut: Some(KeyChord {
                sym: KEY_1,
                modifiers: 0,
            }),
            ..Default::default()
        },
        SessionPanelActionConfig {
            action: "logout".to_string(),
            shortcut: Some(KeyChord {
                sym: KEY_2,
                modifiers: 0,
            }),
            ..Default::default()
        },
        SessionPanelActionConfig {
            action: "lock_and_suspend".to_string(),
            shortcut: Some(KeyChord {
                sym: KEY_3,
                modifiers: 0,
            }),
            ..Default::default()
        },
        SessionPanelActionConfig {
            action: "reboot".to_string(),
            shortcut: Some(KeyChord {
                sym: KEY_4,
                modifiers: 0,
            }),
            ..Default::default()
        },
        SessionPanelActionConfig {
            action: "shutdown".to_string(),
            variant: "destructive".to_string(),
            shortcut: Some(KeyChord {
                sym: KEY_5,
                modifiers: 0,
            }),
            ..Default::default()
        },
    ]
}

/// Port of `ShellSessionConfig::ShellSessionPowerConfig` (config_types.h:230-238).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShellSessionPowerConfig {
    pub suspend: Option<String>,
    pub reboot: Option<String>,
    pub shutdown: Option<String>,
}

/// Port of `ShellSessionConfig` (config_types.h:222-241).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShellSessionConfig {
    pub actions: Vec<SessionPanelActionConfig>,
    pub grid: bool,
    pub grid_columns: i32,
    pub show_shortcuts: bool,
    pub power: ShellSessionPowerConfig,
}

impl Default for ShellSessionConfig {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            grid: false,
            grid_columns: 3,
            show_shortcuts: true,
            power: ShellSessionPowerConfig::default(),
        }
    }
}

/// Port of `ShellGreeterSyncConfig` (config_types.h:243-250).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShellGreeterSyncConfig {
    pub privilege_command: String,
    pub auto_sync: bool,
}

/// Port of `DmenuEntryConfig` (config_types.h:858-878).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct DmenuEntryConfig {
    /// Populated from the enclosing `[shell.launcher.dmenu.entry.<id>]` table
    /// key, not this struct's own derive (see module doc comment).
    #[serde(skip)]
    pub id: String,
    pub command: String,
    pub exec: Option<String>,
    pub prefix: Option<String>,
    pub label: Option<String>,
    pub glyph: Option<String>,
    pub global: bool,
    pub freeform: bool,
}

/// Port of `LauncherProviderConfig` (config_types.h:880-886).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct LauncherProviderConfig {
    /// Populated from the enclosing `[shell.launcher.providers.<name>]` table
    /// key, not this struct's own derive (see module doc comment).
    #[serde(skip)]
    pub name: String,
    pub prefix: String,
    pub global: Option<bool>,
}

/// Port of `ShellConfig::AnimationConfig` (config_types.h:889-894).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AnimationConfig {
    pub enabled: bool,
    pub speed: f32,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
        }
    }
}

/// Port of `ShellConfig::ShadowConfig` (config_types.h:896-901).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShadowConfig {
    /// "center" | "down" | "up" | "left" | "right" | "down_left" | "down_right"
    /// | "up_left" | "up_right".
    pub direction: String,
    pub alpha: f32,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            direction: "down".to_string(),
            alpha: 0.55,
        }
    }
}

/// Port of `ShellConfig::PanelConfig` (config_types.h:903-930).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PanelConfig {
    /// "solid" | "soft" | "glass".
    pub transparency_mode: String,
    pub borders: bool,
    pub shadow: bool,
    pub list_item_background: bool,
    /// "attached" | "floating".
    pub launcher_placement: String,
    pub clipboard_placement: String,
    pub control_center_placement: String,
    pub wallpaper_placement: String,
    pub session_placement: String,
    pub polkit_placement: String,
    pub launcher_position: String,
    pub clipboard_position: String,
    pub control_center_position: String,
    pub wallpaper_position: String,
    pub session_position: String,
    pub polkit_position: String,
    pub floating_offset: i32,
    pub open_near_click_control_center: bool,
    pub open_near_click_launcher: bool,
    pub open_near_click_clipboard: bool,
    pub open_near_click_wallpaper: bool,
    pub open_near_click_session: bool,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            transparency_mode: "solid".to_string(),
            borders: true,
            shadow: true,
            list_item_background: false,
            launcher_placement: "floating".to_string(),
            clipboard_placement: "floating".to_string(),
            control_center_placement: "attached".to_string(),
            wallpaper_placement: "attached".to_string(),
            session_placement: "attached".to_string(),
            polkit_placement: "floating".to_string(),
            launcher_position: "center".to_string(),
            clipboard_position: "center".to_string(),
            control_center_position: "auto".to_string(),
            wallpaper_position: "auto".to_string(),
            session_position: "auto".to_string(),
            polkit_position: "center".to_string(),
            floating_offset: 8,
            open_near_click_control_center: false,
            open_near_click_launcher: false,
            open_near_click_clipboard: false,
            open_near_click_wallpaper: false,
            open_near_click_session: false,
        }
    }
}

/// Port of `ShellConfig::LauncherConfig::DmenuConfig` (config_types.h:947-951).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct DmenuConfig {
    /// See module doc comment: assembled by the schema engine's `namedMap`,
    /// not this struct's own derive.
    #[serde(skip)]
    pub entries: Vec<DmenuEntryConfig>,
}

/// Port of `ShellConfig::LauncherConfig` (config_types.h:935-956).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub categories: bool,
    pub show_icons: bool,
    pub compact: bool,
    pub app_grid: bool,
    pub sort_by_usage: bool,
    pub fetch_exchange_rates: bool,
    pub provider_prefix: String,
    /// "off" | "auto" | "ctrl_v" | "ctrl_shift_v" | "shift_insert".
    pub auto_paste: String,
    pub dmenu: DmenuConfig,
    /// See module doc comment: assembled by the schema engine's `namedMap`,
    /// not this struct's own derive.
    #[serde(skip)]
    pub providers: Vec<LauncherProviderConfig>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            categories: true,
            show_icons: true,
            compact: false,
            app_grid: false,
            sort_by_usage: true,
            fetch_exchange_rates: true,
            provider_prefix: "/".to_string(),
            auto_paste: "auto".to_string(),
            dmenu: DmenuConfig::default(),
            providers: Vec::new(),
        }
    }
}

/// Port of `ShellConfig::ScreenCornersConfig` (config_types.h:958-963).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ScreenCornersConfig {
    pub enabled: bool,
    pub size: i32,
}

impl Default for ScreenCornersConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            size: 32,
        }
    }
}

/// Port of `ShellConfig::MprisConfig` (config_types.h:965-969).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct MprisConfig {
    pub blacklist: Vec<String>,
}

/// Port of `ShellConfig::ScreenshotConfig` (config_types.h:971-983).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ScreenshotConfig {
    pub save_to_file: bool,
    pub copy_to_clipboard: bool,
    pub freeze_screen: bool,
    pub confirm_region: bool,
    pub show_cursor: bool,
    pub pipe_to_command: bool,
    pub pipe_command: String,
    /// Empty = `~/Pictures`.
    pub directory: String,
    /// Empty = `screenshot_%Y%m%d_%H%M%S`.
    pub filename_pattern: String,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            save_to_file: true,
            copy_to_clipboard: true,
            freeze_screen: true,
            confirm_region: false,
            show_cursor: false,
            pipe_to_command: false,
            pipe_command: String::new(),
            directory: String::new(),
            filename_pattern: String::new(),
        }
    }
}

/// Port of `ShellConfig::PrivacyConfig` (config_types.h:985-991).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct PrivacyConfig {
    pub mic_filter_regex: String,
    pub cam_filter_regex: String,
    pub screen_filter_regex: String,
}

/// Port of `ShellConfig` (config_types.h:888-1048).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShellConfig {
    pub corner_radius_scale: f32,
    pub button_borders: bool,
    pub input_borders: bool,
    pub popup_borders: bool,
    pub popup_shadows: bool,
    pub card_borders: bool,
    pub font_family: String,
    /// Empty = auto-detect from `$LC_ALL`/`$LC_MESSAGES`/`$LANG`.
    pub lang: String,
    pub time_format: String,
    pub date_format: String,
    pub offline_mode: bool,
    /// Empty keeps per-source resolution (widget click bar, else first
    /// enabled bar).
    pub panel_anchor_bar: String,
    pub external_ip_enabled: bool,
    pub telemetry_enabled: bool,
    pub setup_wizard_enabled: bool,
    pub niri_overview_type_to_launch_enabled: bool,
    pub polkit_agent: bool,
    /// "default" | "random". C++ field name is `passwordMaskStyle`; TOML key
    /// is the shorter `password_style` (config_schema.cpp:1462), the one
    /// mismatch between a Rust field name and its mechanical
    /// camelCase-to-snake_case TOML key in this whole struct.
    #[serde(rename = "password_style")]
    pub password_mask_style: String,
    pub settings_show_advanced: bool,
    pub show_location: bool,
    pub app_icon_colorize: bool,
    #[serde(with = "optional_color_spec_serde")]
    pub app_icon_color: Option<ColorSpec>,
    pub launch_apps_as_systemd_services: bool,
    pub launch_apps_custom_command: String,
    pub clipboard_enabled: bool,
    pub clipboard_keep_from_closed_apps: bool,
    pub clipboard_history_max_entries: i32,
    pub clipboard_confirm_clear_history: bool,
    pub screen_time_enabled: bool,
    pub shared_gl_context: bool,
    pub disable_mipmaps: bool,
    /// "off" | "auto" | "ctrl_v" | "ctrl_shift_v" | "shift_insert".
    pub clipboard_auto_paste: String,
    pub clipboard_image_action_command: String,
    pub avatar_path: String,
    pub animation: AnimationConfig,
    pub shadow: ShadowConfig,
    pub panel: PanelConfig,
    pub launcher: LauncherConfig,
    pub screen_corners: ScreenCornersConfig,
    pub mpris: MprisConfig,
    pub screenshot: ScreenshotConfig,
    pub privacy: PrivacyConfig,
    pub session: ShellSessionConfig,
    pub greeter_sync: ShellGreeterSyncConfig,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            corner_radius_scale: 1.0,
            button_borders: true,
            input_borders: true,
            popup_borders: true,
            popup_shadows: true,
            card_borders: true,
            font_family: "sans-serif".to_string(),
            lang: String::new(),
            time_format: "{:%H:%M}".to_string(),
            date_format: "%A, %x".to_string(),
            offline_mode: false,
            panel_anchor_bar: String::new(),
            external_ip_enabled: false,
            telemetry_enabled: false,
            setup_wizard_enabled: true,
            niri_overview_type_to_launch_enabled: false,
            polkit_agent: false,
            password_mask_style: "default".to_string(),
            settings_show_advanced: true,
            show_location: true,
            app_icon_colorize: false,
            app_icon_color: None,
            launch_apps_as_systemd_services: false,
            launch_apps_custom_command: String::new(),
            clipboard_enabled: true,
            clipboard_keep_from_closed_apps: true,
            clipboard_history_max_entries: CLIPBOARD_HISTORY_DEFAULT_ENTRIES as i32,
            clipboard_confirm_clear_history: true,
            screen_time_enabled: false,
            shared_gl_context: true,
            disable_mipmaps: false,
            clipboard_auto_paste: "auto".to_string(),
            clipboard_image_action_command: String::new(),
            avatar_path: String::new(),
            animation: AnimationConfig::default(),
            shadow: ShadowConfig::default(),
            panel: PanelConfig::default(),
            launcher: LauncherConfig::default(),
            screen_corners: ScreenCornersConfig::default(),
            mpris: MprisConfig::default(),
            screenshot: ScreenshotConfig::default(),
            privacy: PrivacyConfig::default(),
            session: ShellSessionConfig::default(),
            greeter_sync: ShellGreeterSyncConfig::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use noctalia_core::color::ColorRole;

    fn example_toml() -> &'static str {
        include_str!("../../../../example.toml")
    }

    fn parse_shell() -> ShellConfig {
        let root: toml::Table = toml::from_str(example_toml()).expect("example.toml parses");
        let shell = root
            .get("shell")
            .and_then(toml::Value::as_table)
            .expect("[shell] table");
        shell
            .clone()
            .try_into()
            .expect("[shell] deserializes into ShellConfig")
    }

    #[test]
    fn shell_top_level_scalar_fields_deserialize_from_example_toml() {
        let shell = parse_shell();
        assert_eq!(shell.corner_radius_scale, 1.0);
        assert_eq!(shell.font_family, "sans-serif");
        assert_eq!(shell.time_format, "{:%H:%M}");
        assert_eq!(shell.date_format, "%A, %x");
        assert!(!shell.offline_mode);
        assert!(!shell.telemetry_enabled);
        assert!(!shell.niri_overview_type_to_launch_enabled);
        assert!(!shell.polkit_agent);
        assert_eq!(shell.password_mask_style, "default");
        assert!(shell.settings_show_advanced);
        assert!(shell.show_location);
        assert!(shell.clipboard_enabled);
        assert_eq!(shell.clipboard_history_max_entries, 100);
        assert!(shell.clipboard_keep_from_closed_apps);
        assert_eq!(shell.clipboard_auto_paste, "auto");
        assert_eq!(shell.clipboard_image_action_command, "");
        assert!(shell.shared_gl_context);
        // `lang`/`avatar_path`/`app_icon_color` are commented out in
        // example.toml — they keep their struct defaults.
        assert_eq!(shell.lang, "");
        assert_eq!(shell.avatar_path, "");
        assert_eq!(shell.app_icon_color, None);
    }

    #[test]
    fn shell_privacy_subtable_deserializes_from_example_toml() {
        let shell = parse_shell();
        assert_eq!(shell.privacy.mic_filter_regex, "");
        assert_eq!(shell.privacy.cam_filter_regex, "");
        assert_eq!(shell.privacy.screen_filter_regex, "");
    }

    #[test]
    fn shell_animation_subtable_deserializes_from_example_toml() {
        let shell = parse_shell();
        assert!(shell.animation.enabled);
        assert_eq!(shell.animation.speed, 1.0);
    }

    #[test]
    fn shell_shadow_subtable_deserializes_from_example_toml() {
        let shell = parse_shell();
        assert_eq!(shell.shadow.direction, "down");
        assert_eq!(shell.shadow.alpha, 0.55);
    }

    #[test]
    fn shell_panel_subtable_deserializes_from_example_toml() {
        let shell = parse_shell();
        assert_eq!(shell.panel.transparency_mode, "solid");
        assert!(shell.panel.borders);
        assert!(shell.panel.shadow);
        assert_eq!(shell.panel.launcher_placement, "floating");
        assert_eq!(shell.panel.clipboard_placement, "floating");
        assert_eq!(shell.panel.control_center_placement, "attached");
        assert_eq!(shell.panel.wallpaper_placement, "attached");
        assert_eq!(shell.panel.session_placement, "attached");
        assert_eq!(shell.panel.launcher_position, "center");
        assert_eq!(shell.panel.clipboard_position, "center");
        assert!(!shell.panel.open_near_click_control_center);
        assert!(!shell.panel.open_near_click_launcher);
    }

    #[test]
    fn shell_launcher_subtable_deserializes_from_example_toml() {
        let shell = parse_shell();
        assert!(shell.launcher.categories);
        assert!(shell.launcher.show_icons);
        assert!(!shell.launcher.compact);
        assert!(shell.launcher.sort_by_usage);
        assert!(shell.launcher.fetch_exchange_rates);
        assert_eq!(shell.launcher.provider_prefix, "/");
        assert_eq!(shell.launcher.auto_paste, "auto");
        // `providers`/`dmenu.entries` are `#[serde(skip)]` — the real
        // `[shell.launcher.providers.*]` map-key assembly is task 2.3's job
        // (see module doc comment); they keep their struct defaults even
        // though example.toml's `[shell.launcher.providers.*]` tables are
        // present and silently ignored by plain serde.
        assert!(shell.launcher.providers.is_empty());
        assert!(shell.launcher.dmenu.entries.is_empty());
    }

    #[test]
    fn shell_mpris_subtable_deserializes_from_example_toml() {
        let shell = parse_shell();
        assert_eq!(shell.mpris.blacklist, Vec::<String>::new());
    }

    #[test]
    fn shell_config_missing_subtables_use_cpp_defaults() {
        let shell: ShellConfig =
            toml::from_str("").expect("empty table deserializes via container default");
        assert_eq!(shell, ShellConfig::default());
        assert_eq!(shell.screen_corners.size, 32);
        assert_eq!(shell.session.grid_columns, 3);
        assert!(shell.session.show_shortcuts);
        assert_eq!(shell.greeter_sync.privilege_command, "");
        assert_eq!(shell.screenshot.directory, "");
        assert!(shell.screenshot.save_to_file);
    }

    #[test]
    fn shell_app_icon_color_round_trips_through_toml() {
        let shell: ShellConfig =
            toml::from_str("app_icon_colorize = true\napp_icon_color = \"on_surface\"\n")
                .expect("color field parses");
        assert!(shell.app_icon_colorize);
        assert_eq!(
            shell.app_icon_color.map(|s| s.role),
            Some(Some(ColorRole::OnSurface))
        );

        let serialized = toml::to_string(&shell).expect("serializes back");
        assert!(serialized.contains("app_icon_color = \"on_surface\""));
    }

    #[test]
    fn password_style_uses_the_short_toml_key_not_the_mechanical_field_name() {
        let shell: ShellConfig =
            toml::from_str("password_style = \"random\"\n").expect("short key parses");
        assert_eq!(shell.password_mask_style, "random");
    }

    #[test]
    fn shortcut_config_type_field_round_trips_through_the_type_keyword() {
        let sc: ShortcutConfig = toml::from_str("type = \"wifi\"\n").expect("parses");
        assert_eq!(sc.r#type, "wifi");
        let serialized = toml::to_string(&sc).expect("serializes");
        assert!(serialized.contains("type = \"wifi\""));
    }

    #[test]
    fn default_control_center_shortcuts_matches_cpp_list() {
        let shortcuts = default_control_center_shortcuts();
        let kinds: Vec<&str> = shortcuts.iter().map(|s| s.r#type.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "wifi",
                "bluetooth",
                "caffeine",
                "nightlight",
                "notification",
                "power_profile",
            ]
        );
    }

    #[test]
    fn default_session_panel_actions_matches_cpp_list() {
        let actions = default_session_panel_actions();
        assert_eq!(actions.len(), 5);

        assert_eq!(actions[0].action, "lock");
        assert_eq!(actions[0].variant, "default");
        assert_eq!(
            actions[0].shortcut,
            Some(KeyChord {
                sym: 0x0031,
                modifiers: 0
            })
        );

        assert_eq!(actions[1].action, "logout");
        assert_eq!(actions[1].shortcut.map(|c| c.sym), Some(0x0032));

        assert_eq!(actions[2].action, "lock_and_suspend");
        assert_eq!(actions[2].shortcut.map(|c| c.sym), Some(0x0033));

        assert_eq!(actions[3].action, "reboot");
        assert_eq!(actions[3].shortcut.map(|c| c.sym), Some(0x0034));

        assert_eq!(actions[4].action, "shutdown");
        assert_eq!(actions[4].variant, "destructive");
        assert_eq!(
            actions[4].shortcut,
            Some(KeyChord {
                sym: 0x0035,
                modifiers: 0
            })
        );

        // Every default action's `enabled` stays the struct default (true);
        // the C++ literal init never overrides it.
        assert!(actions.iter().all(|a| a.enabled));
    }

    #[test]
    fn session_panel_action_config_default_matches_cpp_in_class_initializers() {
        let action = SessionPanelActionConfig::default();
        assert_eq!(action.action, "");
        assert!(action.enabled);
        assert_eq!(action.command, None);
        assert_eq!(action.variant, "default");
        assert_eq!(action.shortcut, None);
        assert_eq!(action.countdown_seconds, 0.0);
    }

    #[test]
    fn shell_session_config_default_matches_cpp_in_class_initializers() {
        let session = ShellSessionConfig::default();
        assert!(session.actions.is_empty());
        assert!(!session.grid);
        assert_eq!(session.grid_columns, 3);
        assert!(session.show_shortcuts);
        assert_eq!(session.power, ShellSessionPowerConfig::default());
    }

    #[test]
    fn dmenu_entry_config_id_field_is_not_read_from_toml() {
        // `id` is `#[serde(skip)]` — even if a TOML table happened to have an
        // "id" key, it wouldn't populate this field (matches the C++'s own
        // "id comes from the enclosing table-map key" behavior).
        let entry: DmenuEntryConfig =
            toml::from_str("id = \"should-be-ignored\"\ncommand = \"ls\"\n").expect("parses");
        assert_eq!(entry.id, "");
        assert_eq!(entry.command, "ls");
    }

    #[test]
    fn launcher_provider_config_name_field_is_not_read_from_toml() {
        let provider: LauncherProviderConfig =
            toml::from_str("name = \"should-be-ignored\"\nprefix = \"calc\"\n").expect("parses");
        assert_eq!(provider.name, "");
        assert_eq!(provider.prefix, "calc");
    }
}
