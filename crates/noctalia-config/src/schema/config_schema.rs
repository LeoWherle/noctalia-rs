//! Per-section schema definitions and path resolution.
//! Port of `src/config/schema/config_schema.{h,cpp}`.

use std::collections::HashSet;
use std::sync::LazyLock;

use noctalia_core::color::{
    ColorRole, ColorSpec, color_spec_from_config_string, color_spec_from_role,
    color_spec_to_config_string,
};

use super::config_sections::find_section;
use super::diagnostics::{Diagnostics, join_path};
use super::engine::*;
use super::field::*;
use super::ranges::*;
use crate::types::accessibility::AccessibilityConfig;
use crate::types::audio::AudioConfig;
use crate::types::backdrop::BackdropConfig;
use crate::types::bar::*;
use crate::types::battery::*;
use crate::types::brightness::*;
use crate::types::calendar::*;
use crate::types::config::Config;
use crate::types::control_center::*;
use crate::types::desktop_widgets::*;
use crate::types::dock::*;
use crate::types::hooks::HooksConfig;
use crate::types::hotcorners::*;
use crate::types::idle::*;
use crate::types::keybinds::*;
use crate::types::location::LocationConfig;
use crate::types::lockscreen::LockscreenConfig;
use crate::types::nightlight::NightLightConfig;
use crate::types::notification::*;
use crate::types::osd::*;
use crate::types::plugins::*;
use crate::types::shell::*;
use crate::types::storage::*;
use crate::types::system::*;
use crate::types::theme::*;
use crate::types::wallpaper::*;
use crate::types::weather::WeatherConfig;

// ── Enum Option Constants ──────────────────────────────────────────────────

const DOCK_EDGE_OPTIONS: &[(&str, DockEdge)] = &[
    ("top", DockEdge::Top),
    ("bottom", DockEdge::Bottom),
    ("left", DockEdge::Left),
    ("right", DockEdge::Right),
];

const DOCK_LAUNCHER_POSITION_OPTIONS: &[(&str, DockLauncherPosition)] = &[
    ("none", DockLauncherPosition::None),
    ("start", DockLauncherPosition::Start),
    ("end", DockLauncherPosition::End),
];

const BRIGHTNESS_BACKEND_OPTIONS: &[(&str, BrightnessBackendPreference)] = &[
    ("auto", BrightnessBackendPreference::Auto),
    ("none", BrightnessBackendPreference::None),
    ("backlight", BrightnessBackendPreference::Backlight),
    ("ddcutil", BrightnessBackendPreference::Ddcutil),
];

const PLUGIN_SOURCE_KIND_OPTIONS: &[(&str, PluginSourceKind)] = &[
    ("git", PluginSourceKind::Git),
    ("path", PluginSourceKind::Path),
];

const CALENDAR_CREDENTIAL_SOURCE_OPTIONS: &[(&str, CalendarCredentialSource)] = &[
    ("secret_service", CalendarCredentialSource::SecretService),
    ("secret-service", CalendarCredentialSource::SecretService),
    ("file", CalendarCredentialSource::File),
];

const WALLPAPER_FILL_MODE_OPTIONS: &[(&str, WallpaperFillMode)] = &[
    ("center", WallpaperFillMode::Center),
    ("crop", WallpaperFillMode::Crop),
    ("fit", WallpaperFillMode::Fit),
    ("stretch", WallpaperFillMode::Stretch),
    ("repeat", WallpaperFillMode::Repeat),
    ("span", WallpaperFillMode::Span),
];

const WALLPAPER_ORDER_OPTIONS: &[(&str, WallpaperAutomationOrder)] = &[
    ("random", WallpaperAutomationOrder::Random),
    ("alphabetical", WallpaperAutomationOrder::Alphabetical),
];

const PALETTE_SOURCE_OPTIONS: &[(&str, PaletteSource)] = &[
    ("builtin", PaletteSource::Builtin),
    ("wallpaper", PaletteSource::Wallpaper),
    ("community", PaletteSource::Community),
    ("custom", PaletteSource::Custom),
];

const THEME_MODE_OPTIONS: &[(&str, ThemeMode)] =
    &[("dark", ThemeMode::Dark), ("light", ThemeMode::Light)];

const WALLPAPER_TRANSITION_OPTIONS: &[(&str, WallpaperTransition)] = &[
    ("fade", WallpaperTransition::Fade),
    ("wipe", WallpaperTransition::Wipe),
    ("disc", WallpaperTransition::Disc),
    ("stripes", WallpaperTransition::Stripes),
    ("zoom", WallpaperTransition::Zoom),
    ("honeycomb", WallpaperTransition::Honeycomb),
];

fn parse_wallpaper_transition(s: &str) -> Option<WallpaperTransition> {
    WALLPAPER_TRANSITION_OPTIONS
        .iter()
        .find(|(k, _)| *k == s.trim())
        .map(|(_, v)| *v)
}

fn wallpaper_transition_key(t: WallpaperTransition) -> &'static str {
    match t {
        WallpaperTransition::Fade => "fade",
        WallpaperTransition::Wipe => "wipe",
        WallpaperTransition::Disc => "disc",
        WallpaperTransition::Stripes => "stripes",
        WallpaperTransition::Zoom => "zoom",
        WallpaperTransition::Honeycomb => "honeycomb",
    }
}

// ── Path expansion helper ──────────────────────────────────────────────────

/// Expands `~` or `~/` using `$HOME`.
pub fn expand_user_path(path: &str) -> String {
    if path.is_empty() || !path.starts_with('~') {
        return path.to_string();
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        if path == "~" {
            return home;
        }
        if path.starts_with("~/") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

fn normalize_notification_match_token(token: &str) -> String {
    token.trim().to_lowercase()
}

fn normalize_filter_allowed_urgency_strings(urgencies: Vec<String>) -> Vec<String> {
    urgencies
        .into_iter()
        .map(|s| s.trim().to_lowercase())
        .collect()
}

fn normalize_notification_filter_names(filters: &mut [NotificationFilterConfig]) {
    for (i, filter) in filters.iter_mut().enumerate() {
        if filter.name.is_empty() {
            filter.name = format!("filter_{i}");
        }
    }
}

fn is_valid_plugin_id(id: &str) -> bool {
    !id.trim().is_empty()
}

// ── Helper field constructors ──────────────────────────────────────────────

pub fn path_string_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &str,
    set: fn(&mut S, String),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                let val = if v.is_empty() {
                    v.clone()
                } else {
                    expand_user_path(v)
                };
                set(out, val);
            }
        },
        move |tbl, s| {
            tbl.insert(key.to_string(), toml::Value::String(get(s).to_string()));
        },
    )
}

pub fn optional_path_string_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<String>,
    set: fn(&mut S, Option<String>),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                let val = if v.is_empty() {
                    v.clone()
                } else {
                    expand_user_path(v)
                };
                set(out, Some(val));
            }
        },
        move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::String(v.clone()));
            }
        },
    )
}

pub fn string_if_non_empty_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &str,
    set: fn(&mut S, String),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                set(out, v.clone());
            }
        },
        move |tbl, s| {
            let val = get(s);
            if !val.is_empty() {
                tbl.insert(key.to_string(), toml::Value::String(val.to_string()));
            }
        },
    )
}

pub fn color_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &ColorSpec,
    set: fn(&mut S, ColorSpec),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, parent, _diag| {
            if !tbl.contains_key(key) {
                return;
            }
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                set(
                    out,
                    color_spec_from_config_string(v, &join_path(parent, key))
                        .unwrap_or_else(|_| color_spec_from_role(ColorRole::Surface, 1.0)),
                );
            }
        },
        move |tbl, s| {
            tbl.insert(
                key.to_string(),
                toml::Value::String(color_spec_to_config_string(get(s))),
            );
        },
    )
}

pub fn color_spec_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<ColorSpec>,
    set: fn(&mut S, Option<ColorSpec>),
    always_emit: bool,
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, parent, _diag| {
            if !tbl.contains_key(key) {
                return;
            }
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                if v.trim().is_empty() {
                    set(out, None);
                } else {
                    set(
                        out,
                        color_spec_from_config_string(v, &join_path(parent, key)).ok(),
                    );
                }
            }
        },
        move |tbl, s| {
            if let Some(color) = get(s) {
                tbl.insert(
                    key.to_string(),
                    toml::Value::String(color_spec_to_config_string(color)),
                );
            } else if always_emit {
                tbl.insert(key.to_string(), toml::Value::String(String::new()));
            }
        },
    )
}

pub fn optional_color_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<ColorSpec>,
    set: fn(&mut S, Option<ColorSpec>),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, parent, _diag| {
            if !tbl.contains_key(key) {
                return;
            }
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                set(
                    out,
                    color_spec_from_config_string(v, &join_path(parent, key)).ok(),
                );
            }
        },
        move |tbl, s| {
            if let Some(color) = get(s) {
                tbl.insert(
                    key.to_string(),
                    toml::Value::String(color_spec_to_config_string(color)),
                );
            }
        },
    )
}

pub fn capsule_border_field<S: Send + Sync + 'static>(
    key: &'static str,
    get_color: fn(&S) -> &Option<ColorSpec>,
    get_specified: fn(&S) -> bool,
    set_color: fn(&mut S, Option<ColorSpec>),
    set_specified: fn(&mut S, bool),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, parent, _diag| {
            if !tbl.contains_key(key) {
                return;
            }
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                set_specified(out, true);
                if v.trim().is_empty() {
                    set_color(out, None);
                } else {
                    set_color(
                        out,
                        color_spec_from_config_string(v, &join_path(parent, key)).ok(),
                    );
                }
            }
        },
        move |tbl, s| {
            if get_specified(s) {
                let val = match get_color(s) {
                    Some(c) => color_spec_to_config_string(c),
                    None => String::new(),
                };
                tbl.insert(key.to_string(), toml::Value::String(val));
            }
        },
    )
}

pub fn optional_bool_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> Option<bool>,
    set: fn(&mut S, Option<bool>),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Boolean(v)) = tbl.get(key) {
                set(out, Some(*v));
            }
        },
        move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::Boolean(v));
            }
        },
    )
}

pub fn optional_string_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<String>,
    set: fn(&mut S, Option<String>),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                set(out, Some(v.clone()));
            }
        },
        move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::String(v.clone()));
            }
        },
    )
}

pub fn optional_trimmed_string_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<String>,
    set: fn(&mut S, Option<String>),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                let trimmed = v.trim().to_string();
                if !trimmed.is_empty() {
                    set(out, Some(trimmed));
                }
            }
        },
        move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::String(v.clone()));
            }
        },
    )
}

pub fn optional_string_vector_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<Vec<String>>,
    set: fn(&mut S, Option<Vec<String>>),
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Array(arr)) = tbl.get(key) {
                let values: Vec<String> = arr
                    .iter()
                    .filter_map(|item| {
                        if let toml::Value::String(s) = item {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                set(out, Some(values));
            }
        },
        move |tbl, s| {
            if let Some(values) = get(s) {
                let arr: Vec<toml::Value> = values
                    .iter()
                    .map(|v| toml::Value::String(v.clone()))
                    .collect();
                tbl.insert(key.to_string(), toml::Value::Array(arr));
            }
        },
    )
}

// ── Per-Section Schemas ────────────────────────────────────────────────────

/// Audio section schema.
pub fn audio_schema() -> &'static Schema<AudioConfig> {
    static S: LazyLock<Schema<AudioConfig>> = LazyLock::new(|| {
        vec![
            bool_field(
                "enable_overdrive",
                |s| s.enable_overdrive,
                |s, v| s.enable_overdrive = v,
            ),
            bool_field(
                "enable_sounds",
                |s| s.enable_sounds,
                |s, v| s.enable_sounds = v,
            ),
            f32_field(
                "sound_volume",
                |s| s.sound_volume,
                |s, v| s.sound_volume = v,
                Some(UNIT_RANGE),
            ),
            string_field(
                "volume_change_sound",
                |s| &s.volume_change_sound,
                |s, v| s.volume_change_sound = v,
            ),
            string_field(
                "notification_sound",
                |s| &s.notification_sound,
                |s, v| s.notification_sound = v,
            ),
        ]
    });
    &S
}

/// Weather section schema.
pub fn weather_schema() -> &'static Schema<WeatherConfig> {
    static S: LazyLock<Schema<WeatherConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            bool_field("effects", |s| s.effects, |s, v| s.effects = v),
            i32_field(
                "refresh_minutes",
                |s| s.refresh_minutes,
                |s, v| s.refresh_minutes = v,
                Some(REFRESH_MINUTES_RANGE),
            ),
            string_field("unit", |s| &s.unit, |s, v| s.unit = v),
        ]
    });
    &S
}

/// OSD kinds schema.
pub fn osd_kinds_schema() -> &'static Schema<OsdKindsConfig> {
    static S: LazyLock<Schema<OsdKindsConfig>> = LazyLock::new(|| {
        vec![
            bool_field("volume", |s| s.volume, |s, v| s.volume = v),
            bool_field(
                "volume_output",
                |s| s.volume_output,
                |s, v| s.volume_output = v,
            ),
            bool_field(
                "volume_input",
                |s| s.volume_input,
                |s, v| s.volume_input = v,
            ),
            bool_field("brightness", |s| s.brightness, |s, v| s.brightness = v),
            bool_field("wifi", |s| s.wifi, |s, v| s.wifi = v),
            bool_field("bluetooth", |s| s.bluetooth, |s, v| s.bluetooth = v),
            bool_field(
                "power_profile",
                |s| s.power_profile,
                |s, v| s.power_profile = v,
            ),
            bool_field("caffeine", |s| s.caffeine, |s, v| s.caffeine = v),
            bool_field("nightlight", |s| s.nightlight, |s, v| s.nightlight = v),
            bool_field("dnd", |s| s.dnd, |s, v| s.dnd = v),
            bool_field("lock_keys", |s| s.lock_keys, |s, v| s.lock_keys = v),
            bool_field(
                "keyboard_layout",
                |s| s.keyboard_layout,
                |s, v| s.keyboard_layout = v,
            ),
            bool_field("media", |s| s.media, |s, v| s.media = v),
            bool_field("privacy", |s| s.privacy, |s, v| s.privacy = v),
            bool_field(
                "keyboard_backlight",
                |s| s.keyboard_backlight,
                |s, v| s.keyboard_backlight = v,
            ),
        ]
    });
    &S
}

/// OSD section schema.
pub fn osd_schema() -> &'static Schema<OsdConfig> {
    static S: LazyLock<Schema<OsdConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            string_field("position", |s| &s.position, |s, v| s.position = v),
            string_field(
                "position_vertical",
                |s| &s.position_vertical,
                |s, v| s.position_vertical = v,
            ),
            string_field("orientation", |s| &s.orientation, |s, v| s.orientation = v),
            f32_field("scale", |s| s.scale, |s, v| s.scale = v, Some(SCALE_RANGE)),
            f32_field(
                "background_opacity",
                |s| s.background_opacity,
                |s, v| s.background_opacity = v,
                Some(UNIT_RANGE),
            ),
            bool_field("border", |s| s.border, |s, v| s.border = v),
            i32_field(
                "offset_x",
                |s| s.offset_x,
                |s, v| s.offset_x = v,
                Some(Range::new(Some(0), None, None)),
            ),
            i32_field(
                "offset_y",
                |s| s.offset_y,
                |s, v| s.offset_y = v,
                Some(Range::new(Some(0), None, None)),
            ),
            string_vec_field("monitors", |s| &s.monitors, |s, v| s.monitors = v),
            sub_table(
                "kinds",
                |s| &s.kinds,
                |s, v| s.kinds = v,
                osd_kinds_schema(),
            ),
        ]
    });
    &S
}

/// Backdrop section schema.
pub fn backdrop_schema() -> &'static Schema<BackdropConfig> {
    static S: LazyLock<Schema<BackdropConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            f32_field(
                "blur_intensity",
                |s| s.blur_intensity,
                |s, v| s.blur_intensity = v,
                Some(UNIT_RANGE),
            ),
            f32_field(
                "tint_intensity",
                |s| s.tint_intensity,
                |s, v| s.tint_intensity = v,
                Some(UNIT_RANGE),
            ),
        ]
    });
    &S
}

/// Lockscreen section schema.
pub fn lockscreen_schema() -> &'static Schema<LockscreenConfig> {
    static S: LazyLock<Schema<LockscreenConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            bool_field("fingerprint", |s| s.fingerprint, |s, v| s.fingerprint = v),
            bool_field(
                "allow_empty_password",
                |s| s.allow_empty_password,
                |s, v| s.allow_empty_password = v,
            ),
            bool_field(
                "blurred_desktop",
                |s| s.blurred_desktop,
                |s, v| s.blurred_desktop = v,
            ),
            f32_field(
                "blur_intensity",
                |s| s.blur_intensity,
                |s, v| s.blur_intensity = v,
                Some(UNIT_RANGE),
            ),
            f32_field(
                "tint_intensity",
                |s| s.tint_intensity,
                |s, v| s.tint_intensity = v,
                Some(UNIT_RANGE),
            ),
            path_string_field("wallpaper", |s| &s.wallpaper, |s, v| s.wallpaper = v),
            string_vec_field("monitors", |s| &s.monitors, |s, v| s.monitors = v),
        ]
    });
    &S
}

/// System monitor sub-schema.
fn system_monitor_schema() -> &'static Schema<SystemMonitorConfig> {
    static S: LazyLock<Schema<SystemMonitorConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            string_field(
                "cpu_temp_sensor_path",
                |s| &s.cpu_temp_sensor_path,
                |s, v| s.cpu_temp_sensor_path = v,
            ),
            f32_field(
                "cpu_poll_seconds",
                |s| s.cpu_poll_seconds,
                |s, v| s.cpu_poll_seconds = v,
                None,
            ),
            f32_field(
                "gpu_poll_seconds",
                |s| s.gpu_poll_seconds,
                |s, v| s.gpu_poll_seconds = v,
                None,
            ),
            f32_field(
                "memory_poll_seconds",
                |s| s.memory_poll_seconds,
                |s, v| s.memory_poll_seconds = v,
                None,
            ),
            f32_field(
                "network_poll_seconds",
                |s| s.network_poll_seconds,
                |s, v| s.network_poll_seconds = v,
                None,
            ),
            f32_field(
                "disk_poll_seconds",
                |s| s.disk_poll_seconds,
                |s, v| s.disk_poll_seconds = v,
                None,
            ),
            f64_field(
                "cpu_usage_activity_threshold",
                |s| s.cpu_usage_activity_threshold,
                |s, v| s.cpu_usage_activity_threshold = v,
                None,
            ),
            f64_field(
                "cpu_usage_critical_threshold",
                |s| s.cpu_usage_critical_threshold,
                |s, v| s.cpu_usage_critical_threshold = v,
                None,
            ),
            f64_field(
                "cpu_temp_activity_threshold",
                |s| s.cpu_temp_activity_threshold,
                |s, v| s.cpu_temp_activity_threshold = v,
                None,
            ),
            f64_field(
                "cpu_temp_critical_threshold",
                |s| s.cpu_temp_critical_threshold,
                |s, v| s.cpu_temp_critical_threshold = v,
                None,
            ),
            f64_field(
                "gpu_temp_activity_threshold",
                |s| s.gpu_temp_activity_threshold,
                |s, v| s.gpu_temp_activity_threshold = v,
                None,
            ),
            f64_field(
                "gpu_temp_critical_threshold",
                |s| s.gpu_temp_critical_threshold,
                |s, v| s.gpu_temp_critical_threshold = v,
                None,
            ),
            f64_field(
                "gpu_usage_activity_threshold",
                |s| s.gpu_usage_activity_threshold,
                |s, v| s.gpu_usage_activity_threshold = v,
                None,
            ),
            f64_field(
                "gpu_usage_critical_threshold",
                |s| s.gpu_usage_critical_threshold,
                |s, v| s.gpu_usage_critical_threshold = v,
                None,
            ),
            f64_field(
                "gpu_vram_activity_threshold",
                |s| s.gpu_vram_activity_threshold,
                |s, v| s.gpu_vram_activity_threshold = v,
                None,
            ),
            f64_field(
                "gpu_vram_critical_threshold",
                |s| s.gpu_vram_critical_threshold,
                |s, v| s.gpu_vram_critical_threshold = v,
                None,
            ),
            f64_field(
                "ram_pct_activity_threshold",
                |s| s.ram_pct_activity_threshold,
                |s, v| s.ram_pct_activity_threshold = v,
                None,
            ),
            f64_field(
                "ram_pct_critical_threshold",
                |s| s.ram_pct_critical_threshold,
                |s, v| s.ram_pct_critical_threshold = v,
                None,
            ),
            f64_field(
                "swap_pct_activity_threshold",
                |s| s.swap_pct_activity_threshold,
                |s, v| s.swap_pct_activity_threshold = v,
                None,
            ),
            f64_field(
                "swap_pct_critical_threshold",
                |s| s.swap_pct_critical_threshold,
                |s, v| s.swap_pct_critical_threshold = v,
                None,
            ),
            f64_field(
                "disk_used_pct_activity_threshold",
                |s| s.disk_used_pct_activity_threshold,
                |s, v| s.disk_used_pct_activity_threshold = v,
                None,
            ),
            f64_field(
                "disk_used_pct_critical_threshold",
                |s| s.disk_used_pct_critical_threshold,
                |s, v| s.disk_used_pct_critical_threshold = v,
                None,
            ),
            f64_field(
                "disk_used_activity_threshold",
                |s| s.disk_used_activity_threshold,
                |s, v| s.disk_used_activity_threshold = v,
                None,
            ),
            f64_field(
                "disk_used_critical_threshold",
                |s| s.disk_used_critical_threshold,
                |s, v| s.disk_used_critical_threshold = v,
                None,
            ),
            f64_field(
                "disk_free_pct_activity_threshold",
                |s| s.disk_free_pct_activity_threshold,
                |s, v| s.disk_free_pct_activity_threshold = v,
                None,
            ),
            f64_field(
                "disk_free_pct_critical_threshold",
                |s| s.disk_free_pct_critical_threshold,
                |s, v| s.disk_free_pct_critical_threshold = v,
                None,
            ),
            f64_field(
                "disk_free_activity_threshold",
                |s| s.disk_free_activity_threshold,
                |s, v| s.disk_free_activity_threshold = v,
                None,
            ),
            f64_field(
                "disk_free_critical_threshold",
                |s| s.disk_free_critical_threshold,
                |s, v| s.disk_free_critical_threshold = v,
                None,
            ),
            f64_field(
                "net_rx_activity_threshold",
                |s| s.net_rx_activity_threshold,
                |s, v| s.net_rx_activity_threshold = v,
                None,
            ),
            f64_field(
                "net_rx_critical_threshold",
                |s| s.net_rx_critical_threshold,
                |s, v| s.net_rx_critical_threshold = v,
                None,
            ),
            f64_field(
                "net_tx_activity_threshold",
                |s| s.net_tx_activity_threshold,
                |s, v| s.net_tx_activity_threshold = v,
                None,
            ),
            f64_field(
                "net_tx_critical_threshold",
                |s| s.net_tx_critical_threshold,
                |s, v| s.net_tx_critical_threshold = v,
                None,
            ),
        ]
    });
    &S
}

/// System section schema.
pub fn system_schema() -> &'static Schema<SystemConfig> {
    static S: LazyLock<Schema<SystemConfig>> = LazyLock::new(|| {
        vec![sub_table(
            "monitor",
            |s| &s.monitor,
            |s, v| s.monitor = v,
            system_monitor_schema(),
        )]
    });
    &S
}

/// NightLight section schema.
pub fn nightlight_schema() -> &'static Schema<NightLightConfig> {
    static S: LazyLock<Schema<NightLightConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            bool_field("force", |s| s.force, |s, v| s.force = v),
            i32_field(
                "temperature_day",
                |s| s.day_temperature,
                |s, v| s.day_temperature = v,
                Some(Range::new(Some(1000), Some(25000), None)),
            ),
            i32_field(
                "temperature_night",
                |s| s.night_temperature,
                |s, v| s.night_temperature = v,
                Some(Range::new(Some(1000), Some(25000), None)),
            ),
            finalize(
                |nl: &mut NightLightConfig, path: &str, diag: &mut Diagnostics| {
                    if nl.day_temperature - nl.night_temperature
                        >= NightLightConfig::TEMPERATURE_GAP
                    {
                        return;
                    }
                    let orig_day = nl.day_temperature;
                    let orig_night = nl.night_temperature;
                    nl.night_temperature = orig_day - NightLightConfig::TEMPERATURE_GAP;
                    if nl.night_temperature < NightLightConfig::TEMPERATURE_MIN {
                        nl.night_temperature = NightLightConfig::TEMPERATURE_MIN;
                        nl.day_temperature =
                            NightLightConfig::TEMPERATURE_MIN + NightLightConfig::TEMPERATURE_GAP;
                    }
                    diag.warn(
                    path,
                    format!(
                        "temperatures must satisfy day > night (day={}K night={}K); adjusted to day={}K night={}K",
                        orig_day, orig_night, nl.day_temperature, nl.night_temperature
                    ),
                );
                },
            ),
        ]
    });
    &S
}

/// Location section schema.
pub fn location_schema() -> &'static Schema<LocationConfig> {
    static S: LazyLock<Schema<LocationConfig>> = LazyLock::new(|| {
        vec![
            bool_field("auto_locate", |s| s.auto_locate, |s, v| s.auto_locate = v),
            string_field("address", |s| &s.address, |s, v| s.address = v),
            bool_field(
                "custom_schedule",
                |s| s.custom_schedule,
                |s, v| s.custom_schedule = v,
            ),
            string_field("sunset", |s| &s.sunset, |s, v| s.sunset = v),
            string_field("sunrise", |s| &s.sunrise, |s, v| s.sunrise = v),
            optional_f64_field("latitude", |s| s.latitude, |s, v| s.latitude = v),
            optional_f64_field("longitude", |s| s.longitude, |s, v| s.longitude = v),
        ]
    });
    &S
}

/// Notification filter sub-schema.
fn notification_filter_schema() -> &'static Schema<NotificationFilterConfig> {
    static S: LazyLock<Schema<NotificationFilterConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            custom_field(
                "matches",
                |tbl, out: &mut NotificationFilterConfig, _parent, _diag| {
                    if !out.match_.is_empty() {
                        return;
                    }
                    if let Some(toml::Value::Array(arr)) = tbl.get("matches") {
                        for node in arr {
                            if let Some(token) = node.as_str().filter(|s| !s.trim().is_empty()) {
                                out.match_ = normalize_notification_match_token(token);
                                return;
                            }
                        }
                    }
                },
                |_tbl, _in| {},
            ),
            string_field("match", |s| &s.match_, |s, v| s.match_ = v),
            string_field(
                "match_content",
                |s| &s.match_content,
                |s, v| s.match_content = v,
            ),
            bool_field("show_toast", |s| s.show_toast, |s, v| s.show_toast = v),
            bool_field(
                "save_history",
                |s| s.save_history,
                |s, v| s.save_history = v,
            ),
            bool_field("play_sound", |s| s.play_sound, |s, v| s.play_sound = v),
            bool_field(
                "allow_permanent",
                |s| s.allow_permanent,
                |s, v| s.allow_permanent = v,
            ),
            optional_i32_field(
                "override_duration",
                |s| s.override_duration,
                |s, v| s.override_duration = v,
                None,
            ),
            string_vec_field(
                "allowed_urgencies",
                |s| &s.allowed_urgencies,
                |s, v| s.allowed_urgencies = v,
            ),
            custom_field(
                "allow_critical",
                |_tbl, _out: &mut NotificationFilterConfig, _parent, _diag| {},
                |_tbl, _in| {},
            ),
            finalize(|filter: &mut NotificationFilterConfig, _path, _diag| {
                filter.match_ = normalize_notification_match_token(&filter.match_);
                filter.allowed_urgencies = normalize_filter_allowed_urgency_strings(
                    std::mem::take(&mut filter.allowed_urgencies),
                );
            }),
        ]
    });
    &S
}

/// Notification section schema.
pub fn notification_schema() -> &'static Schema<NotificationConfig> {
    static S: LazyLock<Schema<NotificationConfig>> = LazyLock::new(|| {
        vec![
            bool_field(
                "enable_daemon",
                |s| s.enable_daemon,
                |s, v| s.enable_daemon = v,
            ),
            bool_field(
                "show_app_name",
                |s| s.show_app_name,
                |s, v| s.show_app_name = v,
            ),
            bool_field(
                "show_actions",
                |s| s.show_actions,
                |s, v| s.show_actions = v,
            ),
            string_field("position", |s| &s.position, |s, v| s.position = v),
            string_field("layer", |s| &s.layer, |s, v| s.layer = v),
            f32_field("scale", |s| s.scale, |s, v| s.scale = v, Some(SCALE_RANGE)),
            f32_field(
                "background_opacity",
                |s| s.background_opacity,
                |s, v| s.background_opacity = v,
                Some(UNIT_RANGE),
            ),
            bool_field("border", |s| s.border, |s, v| s.border = v),
            i32_field("offset_x", |s| s.offset_x, |s, v| s.offset_x = v, None),
            i32_field("offset_y", |s| s.offset_y, |s, v| s.offset_y = v, None),
            string_vec_field("monitors", |s| &s.monitors, |s, v| s.monitors = v),
            bool_field(
                "collapse_on_dismiss",
                |s| s.collapse_on_dismiss,
                |s, v| s.collapse_on_dismiss = v,
            ),
            i32_field(
                "history_retention_hours",
                |s| s.history_retention_hours,
                |s, v| s.history_retention_hours = v,
                Some(Range::new(Some(0), Some(8760), None)),
            ),
            custom_field(
                "blacklist",
                |tbl, out: &mut NotificationConfig, _parent, _diag| {
                    if !out.filters.is_empty() {
                        return;
                    }
                    if let Some(toml::Value::Array(arr)) = tbl.get("blacklist") {
                        for node in arr {
                            if let Some(token) = node.as_str().filter(|s| !s.trim().is_empty()) {
                                out.filters.push(NotificationFilterConfig {
                                    match_: normalize_notification_match_token(token),
                                    show_toast: false,
                                    save_history: false,
                                    play_sound: false,
                                    ..Default::default()
                                });
                            }
                        }
                        normalize_notification_filter_names(&mut out.filters);
                    }
                },
                |_tbl, _in| {},
            ),
            custom_field(
                "blacklist_allow_critical",
                |_tbl, _out: &mut NotificationConfig, _parent, _diag| {},
                |_tbl, _in| {},
            ),
            custom_field(
                "filter_order",
                |_tbl, _out: &mut NotificationConfig, _parent, _diag| {},
                |tbl, in_| {
                    let order: Vec<toml::Value> = in_
                        .filters
                        .iter()
                        .filter(|f| !f.name.is_empty())
                        .map(|f| toml::Value::String(f.name.clone()))
                        .collect();
                    if !order.is_empty() {
                        tbl.insert("filter_order".to_string(), toml::Value::Array(order));
                    }
                },
            ),
            named_map(
                "filter",
                |s| &s.filters,
                |s, v| s.filters = v,
                notification_filter_schema(),
                |filter, name| filter.name = name.to_string(),
                |filter| &filter.name,
                false,
            ),
            custom_field(
                "",
                |tbl, out: &mut NotificationConfig, _parent, _diag| {
                    if let Some(toml::Value::Array(arr)) = tbl.get("allowed_urgencies") {
                        let mut global = Vec::new();
                        for node in arr {
                            if let toml::Value::String(value) = node {
                                global.push(value.clone());
                            }
                        }
                        global = normalize_filter_allowed_urgency_strings(global);
                        if !global.is_empty() {
                            for filter in &mut out.filters {
                                if filter.allowed_urgencies.is_empty() {
                                    filter.allowed_urgencies = global.clone();
                                }
                            }
                        }
                    }

                    if let Some(toml::Value::Array(order_arr)) = tbl.get("filter_order") {
                        if !out.filters.is_empty() {
                            let mut by_name: std::collections::HashMap<
                                String,
                                NotificationFilterConfig,
                            > = std::collections::HashMap::new();
                            for filter in out.filters.drain(..) {
                                if !filter.name.is_empty() {
                                    by_name.insert(filter.name.clone(), filter);
                                }
                            }
                            let mut ordered = Vec::new();
                            let mut placed = HashSet::new();
                            for node in order_arr {
                                if let Some(filter) =
                                    node.as_str().and_then(|name| by_name.remove(name))
                                {
                                    placed.insert(filter.name.clone());
                                    ordered.push(filter);
                                }
                            }
                            for (name, filter) in by_name {
                                if !placed.contains(&name) {
                                    ordered.push(filter);
                                }
                            }
                            out.filters = ordered;
                            normalize_notification_filter_names(&mut out.filters);
                        }
                    } else {
                        normalize_notification_filter_names(&mut out.filters);
                    }
                },
                |_tbl, _in| {},
            ),
        ]
    });
    &S
}

/// Dock section schema.
pub fn dock_schema() -> &'static Schema<DockConfig> {
    static S: LazyLock<Schema<DockConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            enum_field(
                "position",
                |s| s.position,
                |s, v| s.position = v,
                DOCK_EDGE_OPTIONS,
            ),
            bool_field(
                "active_monitor_only",
                |s| s.active_monitor_only,
                |s, v| s.active_monitor_only = v,
            ),
            i32_field(
                "icon_size",
                |s| s.icon_size,
                |s, v| s.icon_size = v,
                Some(DOCK_ICON_SIZE_RANGE),
            ),
            i32_field(
                "main_axis_padding",
                |s| s.main_axis_padding,
                |s, v| s.main_axis_padding = v,
                Some(DOCK_PADDING_RANGE),
            ),
            i32_field(
                "cross_axis_padding",
                |s| s.cross_axis_padding,
                |s, v| s.cross_axis_padding = v,
                Some(DOCK_PADDING_RANGE),
            ),
            i32_field(
                "item_spacing",
                |s| s.item_spacing,
                |s, v| s.item_spacing = v,
                Some(DOCK_ITEM_SPACING_RANGE),
            ),
            f32_field(
                "background_opacity",
                |s| s.background_opacity,
                |s, v| s.background_opacity = v,
                Some(UNIT_RANGE),
            ),
            color_field("border", |s| &s.border, |s, v| s.border = v),
            f32_field(
                "border_width",
                |s| s.border_width,
                |s, v| s.border_width = v,
                Some(DOCK_BORDER_WIDTH_RANGE),
            ),
            custom_field(
                "radius",
                |tbl, d: &mut DockConfig, _parent, _diag| {
                    if let Some(toml::Value::Integer(v)) = tbl.get("radius") {
                        let r = apply_range(*v, &DOCK_RADIUS_RANGE) as i32;
                        d.radius = r;
                        d.radius_top_left = r;
                        d.radius_top_right = r;
                        d.radius_bottom_left = r;
                        d.radius_bottom_right = r;
                    }
                },
                |tbl, d| {
                    tbl.insert(
                        "radius".to_string(),
                        toml::Value::Integer(i64::from(d.radius)),
                    );
                },
            ),
            i32_field(
                "radius_top_left",
                |s| s.radius_top_left,
                |s, v| s.radius_top_left = v,
                Some(DOCK_RADIUS_RANGE),
            ),
            i32_field(
                "radius_top_right",
                |s| s.radius_top_right,
                |s, v| s.radius_top_right = v,
                Some(DOCK_RADIUS_RANGE),
            ),
            i32_field(
                "radius_bottom_left",
                |s| s.radius_bottom_left,
                |s, v| s.radius_bottom_left = v,
                Some(DOCK_RADIUS_RANGE),
            ),
            i32_field(
                "radius_bottom_right",
                |s| s.radius_bottom_right,
                |s, v| s.radius_bottom_right = v,
                Some(DOCK_RADIUS_RANGE),
            ),
            bool_field(
                "concave_edge_corners",
                |s| s.concave_edge_corners,
                |s, v| s.concave_edge_corners = v,
            ),
            i32_field(
                "margin_ends",
                |s| s.margin_ends,
                |s, v| s.margin_ends = v,
                Some(DOCK_MARGIN_ENDS_RANGE),
            ),
            i32_field(
                "margin_edge",
                |s| s.margin_edge,
                |s, v| s.margin_edge = v,
                Some(DOCK_MARGIN_EDGE_RANGE),
            ),
            bool_field("shadow", |s| s.shadow, |s, v| s.shadow = v),
            bool_field(
                "show_running",
                |s| s.show_running,
                |s, v| s.show_running = v,
            ),
            bool_field("auto_hide", |s| s.auto_hide, |s, v| s.auto_hide = v),
            bool_field(
                "smart_auto_hide",
                |s| s.smart_auto_hide,
                |s, v| s.smart_auto_hide = v,
            ),
            custom_field(
                "layer",
                |tbl, out: &mut DockConfig, parent, diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("layer") {
                        if v == "top" || v == "overlay" {
                            out.layer = v.clone();
                        } else {
                            diag.warn(
                                join_path(parent, "layer"),
                                format!("expected top or overlay, got \"{v}\""),
                            );
                        }
                    }
                },
                |tbl, in_| {
                    tbl.insert("layer".to_string(), toml::Value::String(in_.layer.clone()));
                },
            ),
            bool_field(
                "reserve_space",
                |s| s.reserve_space,
                |s, v| s.reserve_space = v,
            ),
            f32_field(
                "active_scale",
                |s| s.active_scale,
                |s, v| s.active_scale = v,
                Some(DOCK_ACTIVE_SCALE_RANGE),
            ),
            f32_field(
                "inactive_scale",
                |s| s.inactive_scale,
                |s, v| s.inactive_scale = v,
                Some(DOCK_INACTIVE_SCALE_RANGE),
            ),
            bool_field(
                "magnification",
                |s| s.magnification,
                |s, v| s.magnification = v,
            ),
            f32_field(
                "magnification_scale",
                |s| s.magnification_scale,
                |s, v| s.magnification_scale = v,
                Some(DOCK_MAGNIFICATION_SCALE_RANGE),
            ),
            f32_field(
                "active_opacity",
                |s| s.active_opacity,
                |s, v| s.active_opacity = v,
                Some(UNIT_RANGE),
            ),
            f32_field(
                "inactive_opacity",
                |s| s.inactive_opacity,
                |s, v| s.inactive_opacity = v,
                Some(UNIT_RANGE),
            ),
            bool_field("show_dots", |s| s.show_dots, |s, v| s.show_dots = v),
            bool_field(
                "show_instance_count",
                |s| s.show_instance_count,
                |s, v| s.show_instance_count = v,
            ),
            enum_field(
                "launcher_position",
                |s| s.launcher_position,
                |s, v| s.launcher_position = v,
                DOCK_LAUNCHER_POSITION_OPTIONS,
            ),
            string_field(
                "launcher_icon",
                |s| &s.launcher_icon,
                |s, v| s.launcher_icon = v,
            ),
            path_string_field(
                "launcher_custom_image",
                |s| &s.launcher_custom_image,
                |s, v| s.launcher_custom_image = v,
            ),
            bool_field(
                "launcher_custom_image_colorize",
                |s| s.launcher_custom_image_colorize,
                |s, v| s.launcher_custom_image_colorize = v,
            ),
            string_vec_field("pinned", |s| &s.pinned, |s, v| s.pinned = v),
            string_vec_field("monitors", |s| &s.monitors, |s, v| s.monitors = v),
        ]
    });
    &S
}

/// Desktop widgets grid state schema.
fn desktop_widgets_grid_schema() -> &'static Schema<DesktopWidgetsGridState> {
    static S: LazyLock<Schema<DesktopWidgetsGridState>> = LazyLock::new(|| {
        vec![
            bool_field("visible", |s| s.visible, |s, v| s.visible = v),
            i32_field(
                "cell_size",
                |s| s.cell_size,
                |s, v| s.cell_size = v,
                Some(Range::new(Some(8), Some(256), None)),
            ),
            i32_field(
                "major_interval",
                |s| s.major_interval,
                |s, v| s.major_interval = v,
                Some(Range::new(Some(1), Some(16), None)),
            ),
        ]
    });
    &S
}

/// Desktop widgets section schema.
pub fn desktop_widgets_schema() -> &'static Schema<DesktopWidgetsConfig> {
    static S: LazyLock<Schema<DesktopWidgetsConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            i32_field(
                "schema_version",
                |s| s.schema_version,
                |s, v| s.schema_version = v,
                None,
            ),
            sub_table(
                "grid",
                |s| &s.grid,
                |s, v| s.grid = v,
                desktop_widgets_grid_schema(),
            ),
        ]
    });
    &S
}

/// Lockscreen widgets section schema.
pub fn lockscreen_widgets_schema() -> &'static Schema<LockscreenWidgetsConfig> {
    static S: LazyLock<Schema<LockscreenWidgetsConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            i32_field(
                "schema_version",
                |s| s.schema_version,
                |s, v| s.schema_version = v,
                None,
            ),
            sub_table(
                "grid",
                |s| &s.grid,
                |s, v| s.grid = v,
                desktop_widgets_grid_schema(),
            ),
        ]
    });
    &S
}

/// Hot corners corner schema.
fn corner_schema() -> &'static Schema<HotCornerConfig> {
    static S: LazyLock<Schema<HotCornerConfig>> = LazyLock::new(|| {
        vec![
            string_field("action", |s| &s.action, |s, v| s.action = v),
            string_field("command", |s| &s.command, |s, v| s.command = v),
        ]
    });
    &S
}

/// Hot corners section schema.
pub fn hot_corners_schema() -> &'static Schema<HotCornersConfig> {
    static S: LazyLock<Schema<HotCornersConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            i32_field(
                "delay_ms",
                |s| s.delay_ms,
                |s, v| s.delay_ms = v,
                Some(HOT_CORNERS_DELAY_MS_RANGE),
            ),
            sub_table(
                "top_left",
                |s| &s.top_left,
                |s, v| s.top_left = v,
                corner_schema(),
            ),
            sub_table(
                "top_right",
                |s| &s.top_right,
                |s, v| s.top_right = v,
                corner_schema(),
            ),
            sub_table(
                "bottom_left",
                |s| &s.bottom_left,
                |s, v| s.bottom_left = v,
                corner_schema(),
            ),
            sub_table(
                "bottom_right",
                |s| &s.bottom_right,
                |s, v| s.bottom_right = v,
                corner_schema(),
            ),
        ]
    });
    &S
}

/// Brightness monitor override schema.
fn brightness_monitor_schema() -> &'static Schema<BrightnessMonitorOverride> {
    static S: LazyLock<Schema<BrightnessMonitorOverride>> = LazyLock::new(|| {
        vec![
            string_field("match", |s| &s.match_, |s, v| s.match_ = v),
            optional_enum_field(
                "backend",
                |s| s.backend,
                |s, v| s.backend = v,
                BRIGHTNESS_BACKEND_OPTIONS,
            ),
        ]
    });
    &S
}

/// Brightness section schema.
pub fn brightness_schema() -> &'static Schema<BrightnessConfig> {
    static S: LazyLock<Schema<BrightnessConfig>> = LazyLock::new(|| {
        vec![
            bool_field(
                "enable_ddcutil",
                |s| s.enable_ddcutil,
                |s, v| s.enable_ddcutil = v,
            ),
            string_vec_field(
                "ddcutil_ignore_mmids",
                |s| &s.ddcutil_ignore_mmids,
                |s, v| s.ddcutil_ignore_mmids = v,
            ),
            named_map(
                "monitor",
                |s| &s.monitor_overrides,
                |s, v| s.monitor_overrides = v,
                brightness_monitor_schema(),
                |o, name| o.match_ = name.to_string(),
                |o| &o.match_,
                false,
            ),
        ]
    });
    &S
}

/// Battery device threshold schema.
fn battery_device_schema() -> &'static Schema<BatteryDeviceWarningThreshold> {
    static S: LazyLock<Schema<BatteryDeviceWarningThreshold>> = LazyLock::new(|| {
        vec![
            string_field("name", |s| &s.selector, |s, v| s.selector = v),
            i32_field(
                "warning_threshold",
                |s| s.warning_threshold,
                |s, v| s.warning_threshold = v,
                Some(BATTERY_WARNING_THRESHOLD_RANGE),
            ),
        ]
    });
    &S
}

/// Battery section schema.
pub fn battery_schema() -> &'static Schema<BatteryConfig> {
    static S: LazyLock<Schema<BatteryConfig>> = LazyLock::new(|| {
        vec![
            i32_field(
                "warning_threshold",
                |s| s.warning_threshold,
                |s, v| s.warning_threshold = v,
                Some(BATTERY_WARNING_THRESHOLD_RANGE),
            ),
            named_map(
                "device",
                |s| &s.device_thresholds,
                |s, v| s.device_thresholds = v,
                battery_device_schema(),
                |d, name| d.selector = name.to_string(),
                |d| &d.selector,
                true,
            ),
        ]
    });
    &S
}

/// Shortcut sub-schema.
fn shortcut_schema() -> &'static Schema<ShortcutConfig> {
    static S: LazyLock<Schema<ShortcutConfig>> =
        LazyLock::new(|| vec![string_field("type", |s| &s.r#type, |s, v| s.r#type = v)]);
    &S
}

/// Calendar tab sub-schema.
fn calendar_tab_schema() -> &'static Schema<CalendarTabConfig> {
    static S: LazyLock<Schema<CalendarTabConfig>> = LazyLock::new(|| {
        vec![
            bool_field(
                "show_events_card",
                |s| s.show_events_card,
                |s, v| s.show_events_card = v,
            ),
            bool_field(
                "show_week_numbers",
                |s| s.show_week_numbers,
                |s, v| s.show_week_numbers = v,
            ),
            string_field(
                "event_date_format",
                |s| &s.event_date_format,
                |s, v| s.event_date_format = v,
            ),
            string_field(
                "event_time_format",
                |s| &s.event_time_format,
                |s, v| s.event_time_format = v,
            ),
        ]
    });
    &S
}

/// Control center section schema.
pub fn control_center_schema() -> &'static Schema<ControlCenterConfig> {
    static S: LazyLock<Schema<ControlCenterConfig>> = LazyLock::new(|| {
        vec![
            string_field("sidebar", |s| &s.sidebar_mode, |s, v| s.sidebar_mode = v),
            string_field(
                "sidebar_section",
                |s| &s.sidebar_section_mode,
                |s, v| s.sidebar_section_mode = v,
            ),
            i32_field(
                "width",
                |s| s.width,
                |s, v| s.width = v,
                Some(CONTROL_CENTER_WIDTH_RANGE),
            ),
            bool_field(
                "show_shortcut_labels",
                |s| s.show_shortcut_labels,
                |s, v| s.show_shortcut_labels = v,
            ),
            string_vec_field("hidden_tabs", |s| &s.hidden_tabs, |s, v| s.hidden_tabs = v),
            sub_table(
                "calendar",
                |s| &s.calendar_tab,
                |s, v| s.calendar_tab = v,
                calendar_tab_schema(),
            ),
            array_of(
                "shortcuts",
                |s| &s.shortcuts,
                |s, v| s.shortcuts = v,
                shortcut_schema(),
                |elem| !elem.r#type.is_empty(),
            ),
        ]
    });
    &S
}

/// Plugin source sub-schema.
fn plugin_source_schema() -> &'static Schema<PluginSourceConfig> {
    static S: LazyLock<Schema<PluginSourceConfig>> = LazyLock::new(|| {
        vec![
            custom_field(
                "name",
                |tbl, out: &mut PluginSourceConfig, parent, diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("name") {
                        if is_valid_plugin_source_name(v) {
                            out.name = v.clone();
                        } else {
                            diag.warn(
                                join_path(parent, "name"),
                                format!("invalid plugin source name \"{v}\""),
                            );
                        }
                    }
                },
                |tbl, in_| {
                    tbl.insert("name".to_string(), toml::Value::String(in_.name.clone()));
                },
            ),
            enum_field(
                "kind",
                |s| s.kind,
                |s, v| s.kind = v,
                PLUGIN_SOURCE_KIND_OPTIONS,
            ),
            path_string_field("location", |s| &s.location, |s, v| s.location = v),
        ]
    });
    &S
}

/// Plugins section schema.
pub fn plugins_schema() -> &'static Schema<PluginsConfig> {
    static S: LazyLock<Schema<PluginsConfig>> = LazyLock::new(|| {
        vec![
            array_of(
                "source",
                |s| &s.sources,
                |s, v| s.sources = v,
                plugin_source_schema(),
                |s| is_valid_plugin_source_name(&s.name),
            ),
            custom_field(
                "enabled",
                |tbl, out: &mut PluginsConfig, parent, diag| {
                    if let Some(toml::Value::Array(arr)) = tbl.get("enabled") {
                        let mut valid = Vec::new();
                        for item in arr {
                            if let toml::Value::String(s) = item {
                                if is_valid_plugin_id(s) {
                                    valid.push(s.clone());
                                } else {
                                    diag.warn(
                                        join_path(parent, "enabled"),
                                        format!("invalid plugin id \"{s}\""),
                                    );
                                }
                            }
                        }
                        out.enabled = valid;
                    }
                },
                |tbl, in_| {
                    let arr: Vec<toml::Value> = in_
                        .enabled
                        .iter()
                        .map(|s| toml::Value::String(s.clone()))
                        .collect();
                    tbl.insert("enabled".to_string(), toml::Value::Array(arr));
                },
            ),
            bool_field("auto_update", |s| s.auto_update, |s, v| s.auto_update = v),
        ]
    });
    &S
}

/// Calendar account sub-schema.
fn calendar_account_schema() -> &'static Schema<CalendarAccountConfig> {
    static S: LazyLock<Schema<CalendarAccountConfig>> = LazyLock::new(|| {
        vec![
            string_field("type", |s| &s.type_, |s, v| s.type_ = v),
            string_field("name", |s| &s.display_name, |s, v| s.display_name = v),
            string_field("color", |s| &s.color, |s, v| s.color = v),
            string_field("provider", |s| &s.provider, |s, v| s.provider = v),
            string_field("server_url", |s| &s.server_url, |s, v| s.server_url = v),
            string_field("username", |s| &s.username, |s, v| s.username = v),
            string_vec_field(
                "enabled_calendars",
                |s| &s.calendars,
                |s, v| s.calendars = v,
            ),
            enum_field(
                "credential_source",
                |s| s.credential_source,
                |s, v| s.credential_source = v,
                CALENDAR_CREDENTIAL_SOURCE_OPTIONS,
            ),
            path_string_field(
                "password_file",
                |s| &s.password_file,
                |s, v| s.password_file = v,
            ),
            finalize(
                |out: &mut CalendarAccountConfig, parent_path: &str, diag: &mut Diagnostics| {
                    if out.credential_source == CalendarCredentialSource::File {
                        if out.password_file.is_empty() {
                            diag.error(join_path(parent_path, "password_file"), r#"calendar account with credential_source = "file" requires password_file"#);
                        } else if !std::path::Path::new(&out.password_file).is_absolute() {
                            diag.error(
                                join_path(parent_path, "password_file"),
                                "password_file must resolve to an absolute path",
                            );
                        }
                    } else if !out.password_file.is_empty() {
                        diag.error(
                            join_path(parent_path, "password_file"),
                            r#"password_file requires credential_source = "file""#,
                        );
                    }
                },
            ),
        ]
    });
    &S
}

/// Calendar section schema.
pub fn calendar_schema() -> &'static Schema<CalendarConfig> {
    static S: LazyLock<Schema<CalendarConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            i32_field(
                "refresh_minutes",
                |s| s.refresh_minutes,
                |s, v| s.refresh_minutes = v,
                Some(REFRESH_MINUTES_RANGE),
            ),
            named_map(
                "account",
                |s| &s.accounts,
                |s, v| s.accounts = v,
                calendar_account_schema(),
                |a, id| a.id = id.to_string(),
                |a| &a.id,
                true,
            ),
        ]
    });
    &S
}

/// Keybinds section schema.
/// `keybindActionField` (`config_schema.cpp:763-819`) parses each key's
/// string/array-of-strings value via `parseKeyChordSpec`, which needs `xkbcommon`
/// FFI — task 10.2's job (see `KeybindsConfig`'s own doc comment in
/// `types/keybinds.rs`). Until then these 8 fields only mark the real TOML keys
/// as *known* (so `checkSection`'s unknown-key scan doesn't flag every
/// `[keybinds]` entry as unrecognized) — they don't parse or write anything. Task
/// 10.2 replaces these no-op read/write closures with the real
/// `parseKeyChordSpec` bridge.
pub fn keybinds_schema() -> &'static Schema<KeybindsConfig> {
    static S: LazyLock<Schema<KeybindsConfig>> = LazyLock::new(|| {
        vec![
            custom_field("validate", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field("cancel", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field("left", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field("right", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field("up", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field("down", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field("tab_next", |_, _: &mut KeybindsConfig, _, _| {}, |_, _| {}),
            custom_field(
                "tab_previous",
                |_, _: &mut KeybindsConfig, _, _| {},
                |_, _| {},
            ),
        ]
    });
    &S
}

/// Hooks section schema.
pub fn hooks_schema() -> &'static Schema<HooksConfig> {
    static S: LazyLock<Schema<HooksConfig>> = LazyLock::new(|| {
        vec![
            string_vec_field("started", |s| &s.started, |s, v| s.started = v),
            string_vec_field(
                "wallpaper_changed",
                |s| &s.wallpaper_changed,
                |s, v| s.wallpaper_changed = v,
            ),
            string_vec_field(
                "colors_changed",
                |s| &s.colors_changed,
                |s, v| s.colors_changed = v,
            ),
            string_vec_field(
                "theme_mode_changed",
                |s| &s.theme_mode_changed,
                |s, v| s.theme_mode_changed = v,
            ),
            string_vec_field(
                "session_locked",
                |s| &s.session_locked,
                |s, v| s.session_locked = v,
            ),
            string_vec_field(
                "session_unlocked",
                |s| &s.session_unlocked,
                |s, v| s.session_unlocked = v,
            ),
            string_vec_field("logging_out", |s| &s.logging_out, |s, v| s.logging_out = v),
            string_vec_field("rebooting", |s| &s.rebooting, |s, v| s.rebooting = v),
            string_vec_field(
                "shutting_down",
                |s| &s.shutting_down,
                |s, v| s.shutting_down = v,
            ),
            string_vec_field(
                "wifi_enabled",
                |s| &s.wifi_enabled,
                |s, v| s.wifi_enabled = v,
            ),
            string_vec_field(
                "wifi_disabled",
                |s| &s.wifi_disabled,
                |s, v| s.wifi_disabled = v,
            ),
            string_vec_field(
                "bluetooth_enabled",
                |s| &s.bluetooth_enabled,
                |s, v| s.bluetooth_enabled = v,
            ),
            string_vec_field(
                "bluetooth_disabled",
                |s| &s.bluetooth_disabled,
                |s, v| s.bluetooth_disabled = v,
            ),
            string_vec_field(
                "battery_charging",
                |s| &s.battery_charging,
                |s, v| s.battery_charging = v,
            ),
            string_vec_field(
                "battery_discharging",
                |s| &s.battery_discharging,
                |s, v| s.battery_discharging = v,
            ),
            string_vec_field(
                "battery_plugged",
                |s| &s.battery_plugged,
                |s, v| s.battery_plugged = v,
            ),
            string_vec_field(
                "battery_percentage_changed",
                |s| &s.battery_percentage_changed,
                |s, v| s.battery_percentage_changed = v,
            ),
            string_vec_field(
                "power_profile_changed",
                |s| &s.power_profile_changed,
                |s, v| s.power_profile_changed = v,
            ),
        ]
    });
    &S
}

/// Idle behavior sub-schema.
fn idle_behavior_schema() -> &'static Schema<IdleBehaviorConfig> {
    static S: LazyLock<Schema<IdleBehaviorConfig>> = LazyLock::new(|| {
        vec![
            string_field("name", |s| &s.name, |s, v| s.name = v),
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            f64_field(
                "timeout",
                |s| s.timeout_seconds,
                |s, v| s.timeout_seconds = v,
                None,
            ),
            string_field("action", |s| &s.action, |s, v| s.action = v),
            string_field("command", |s| &s.command, |s, v| s.command = v),
            string_field(
                "resume_command",
                |s| &s.resume_command,
                |s, v| s.resume_command = v,
            ),
            bool_field(
                "lock_before_suspend",
                |s| s.lock_before_suspend,
                |s, v| s.lock_before_suspend = v,
            ),
        ]
    });
    &S
}

/// Idle section schema.
pub fn idle_schema() -> &'static Schema<IdleConfig> {
    static S: LazyLock<Schema<IdleConfig>> = LazyLock::new(|| {
        vec![
            f32_field(
                "pre_action_fade_seconds",
                |s| s.pre_action_fade_seconds,
                |s, v| s.pre_action_fade_seconds = v,
                None,
            ),
            array_of(
                "behavior",
                |s| &s.behaviors,
                |s, v| s.behaviors = v,
                idle_behavior_schema(),
                |b| !b.name.is_empty(),
            ),
        ]
    });
    &S
}

/// Wallpaper automation sub-schema.
fn wallpaper_automation_schema() -> &'static Schema<WallpaperAutomationConfig> {
    static S: LazyLock<Schema<WallpaperAutomationConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            i32_field(
                "interval_seconds",
                |s| s.interval_seconds,
                |s, v| s.interval_seconds = v,
                Some(WALLPAPER_AUTOMATION_INTERVAL_RANGE),
            ),
            enum_field(
                "order",
                |s| s.order,
                |s, v| s.order = v,
                WALLPAPER_ORDER_OPTIONS,
            ),
            bool_field("recursive", |s| s.recursive, |s, v| s.recursive = v),
        ]
    });
    &S
}

/// Wallpaper monitor override sub-schema.
fn wallpaper_monitor_schema() -> &'static Schema<WallpaperMonitorOverride> {
    static S: LazyLock<Schema<WallpaperMonitorOverride>> = LazyLock::new(|| {
        vec![
            string_field("match", |s| &s.match_, |s, v| s.match_ = v),
            optional_bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            color_spec_field(
                "fill_color",
                |s| &s.fill_color,
                |s, v| s.fill_color = v,
                false,
            ),
            optional_path_string_field("directory", |s| &s.directory, |s, v| s.directory = v),
            optional_path_string_field(
                "directory_light",
                |s| &s.directory_light,
                |s, v| s.directory_light = v,
            ),
            optional_path_string_field(
                "directory_dark",
                |s| &s.directory_dark,
                |s, v| s.directory_dark = v,
            ),
        ]
    });
    &S
}

/// Wallpaper section schema.
pub fn wallpaper_schema() -> &'static Schema<WallpaperConfig> {
    static S: LazyLock<Schema<WallpaperConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            enum_field(
                "fill_mode",
                |s| s.fill_mode,
                |s, v| s.fill_mode = v,
                WALLPAPER_FILL_MODE_OPTIONS,
            ),
            color_spec_field(
                "fill_color",
                |s| &s.fill_color,
                |s, v| s.fill_color = v,
                true,
            ),
            custom_field(
                "transition",
                |tbl, out: &mut WallpaperConfig, _parent, _diag| {
                    if let Some(toml::Value::Array(arr)) = tbl.get("transition") {
                        let mut vec = Vec::new();
                        for item in arr {
                            if let Some(t) = item.as_str().and_then(parse_wallpaper_transition) {
                                vec.push(t);
                            }
                        }
                        out.transitions = vec;
                    }
                },
                |tbl, in_| {
                    let arr: Vec<toml::Value> = in_
                        .transitions
                        .iter()
                        .map(|t| toml::Value::String(wallpaper_transition_key(*t).to_string()))
                        .collect();
                    tbl.insert("transition".to_string(), toml::Value::Array(arr));
                },
            ),
            f32_field(
                "transition_duration",
                |s| s.transition_duration_ms,
                |s, v| s.transition_duration_ms = v,
                Some(WALLPAPER_TRANSITION_DURATION_RANGE),
            ),
            f32_field(
                "edge_smoothness",
                |s| s.edge_smoothness,
                |s, v| s.edge_smoothness = v,
                Some(UNIT_RANGE),
            ),
            bool_field(
                "transition_on_startup",
                |s| s.transition_on_startup,
                |s, v| s.transition_on_startup = v,
            ),
            path_string_field("directory", |s| &s.directory, |s, v| s.directory = v),
            path_string_field(
                "directory_light",
                |s| &s.directory_light,
                |s, v| s.directory_light = v,
            ),
            path_string_field(
                "directory_dark",
                |s| &s.directory_dark,
                |s, v| s.directory_dark = v,
            ),
            bool_field(
                "per_monitor_directories",
                |s| s.per_monitor_directories,
                |s, v| s.per_monitor_directories = v,
            ),
            sub_table(
                "automation",
                |s| &s.automation,
                |s, v| s.automation = v,
                wallpaper_automation_schema(),
            ),
            named_map(
                "monitor",
                |s| &s.monitor_overrides,
                |s, v| s.monitor_overrides = v,
                wallpaper_monitor_schema(),
                |o, name| o.match_ = name.to_string(),
                |o| &o.match_,
                false,
            ),
        ]
    });
    &S
}

// ── Custom colors map parsing ───────────────────────────────────────────────

pub fn parse_custom_colors_map(map: &toml::Table, out: &mut Vec<TemplateColorConfig>) {
    for (name, value) in map {
        let mut color = TemplateColorConfig {
            name: name.clone(),
            color: String::new(),
            color_dark: String::new(),
            color_light: String::new(),
            blend: true,
        };
        match value {
            toml::Value::String(s) => {
                color.color = s.clone();
            }
            toml::Value::Table(t) => {
                if let Some(toml::Value::String(c)) = t.get("color_dark") {
                    color.color_dark = c.clone();
                }
                if let Some(toml::Value::String(c)) = t.get("color_light") {
                    color.color_light = c.clone();
                }
                if let Some(toml::Value::String(c)) = t.get("color") {
                    color.color = c.clone();
                } else if !color.color_dark.is_empty() {
                    color.color = color.color_dark.clone();
                } else {
                    color.color = color.color_light.clone();
                }
                if let Some(toml::Value::Boolean(b)) = t.get("blend") {
                    color.blend = *b;
                }
            }
            _ => {}
        }
        if !color.name.trim().is_empty() && !color.color.trim().is_empty() {
            out.push(color);
        }
    }
}

pub fn append_unique_custom_colors(
    into: &mut Vec<TemplateColorConfig>,
    additional: Vec<TemplateColorConfig>,
) {
    for color in additional {
        if !into.iter().any(|c| c.name == color.name) {
            into.push(color);
        }
    }
}

pub fn lift_template_config_custom_colors(root: &toml::Table, config: &mut Config) {
    if let Some(custom_colors) = root
        .get("config")
        .and_then(toml::Value::as_table)
        .and_then(|c| c.get("custom_colors"))
        .and_then(toml::Value::as_table)
    {
        let mut lifted = Vec::new();
        parse_custom_colors_map(custom_colors, &mut lifted);
        append_unique_custom_colors(&mut config.theme.templates.custom_colors, lifted);
    }
}

fn user_template_schema() -> &'static Schema<UserTemplateConfig> {
    static S: LazyLock<Schema<UserTemplateConfig>> = LazyLock::new(|| {
        vec![
            custom_field(
                "name",
                |tbl, out: &mut UserTemplateConfig, _parent, _diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("name") {
                        out.id = v.trim().to_string();
                    }
                },
                |tbl, in_| {
                    tbl.insert("name".to_string(), toml::Value::String(in_.id.clone()));
                },
            ),
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            path_string_field("input_path", |s| &s.input_path, |s, v| s.input_path = v),
            custom_field(
                "input_path_modes",
                |tbl, out: &mut UserTemplateConfig, _parent, _diag| {
                    if let Some(toml::Value::Table(sub)) = tbl.get("input_path_modes") {
                        let mut modes = TemplateInputPathModesConfig::default();
                        if let Some(toml::Value::String(d)) = sub.get("dark") {
                            modes.dark = expand_user_path(d);
                        }
                        if let Some(toml::Value::String(l)) = sub.get("light") {
                            modes.light = expand_user_path(l);
                        }
                        out.input_path_modes = Some(modes);
                    }
                },
                |tbl, in_| {
                    if let Some(modes) = in_
                        .input_path_modes
                        .as_ref()
                        .filter(|m| !m.dark.is_empty() || !m.light.is_empty())
                    {
                        let mut sub = toml::Table::new();
                        sub.insert("dark".to_string(), toml::Value::String(modes.dark.clone()));
                        sub.insert(
                            "light".to_string(),
                            toml::Value::String(modes.light.clone()),
                        );
                        tbl.insert("input_path_modes".to_string(), toml::Value::Table(sub));
                    }
                },
            ),
            custom_field(
                "output_path",
                |tbl, out: &mut UserTemplateConfig, _parent, _diag| match tbl.get("output_path") {
                    Some(toml::Value::String(s)) => {
                        if !s.is_empty() {
                            out.output_paths = vec![expand_user_path(s)];
                        }
                    }
                    Some(toml::Value::Array(arr)) => {
                        let mut paths = Vec::new();
                        for item in arr {
                            if let Some(s) = item.as_str().filter(|s| !s.is_empty()) {
                                paths.push(expand_user_path(s));
                            }
                        }
                        out.output_paths = paths;
                    }
                    _ => {}
                },
                |tbl, in_| {
                    if in_.output_paths.len() == 1 {
                        tbl.insert(
                            "output_path".to_string(),
                            toml::Value::String(in_.output_paths[0].clone()),
                        );
                    } else if !in_.output_paths.is_empty() {
                        let arr: Vec<toml::Value> = in_
                            .output_paths
                            .iter()
                            .map(|p| toml::Value::String(p.clone()))
                            .collect();
                        tbl.insert("output_path".to_string(), toml::Value::Array(arr));
                    }
                },
            ),
            path_string_field(
                "dynamic_template",
                |s| &s.output_path_dynamic,
                |s, v| s.output_path_dynamic = v,
            ),
            string_field("compare_mode", |s| &s.compare_to, |s, v| s.compare_to = v),
            custom_field(
                "compare_colors",
                |tbl, out: &mut UserTemplateConfig, _parent, _diag| {
                    if let Some(toml::Value::Table(sub)) = tbl.get("compare_colors") {
                        let mut vec = Vec::new();
                        for (k, v) in sub {
                            if let toml::Value::String(c) = v {
                                vec.push(TemplateCompareColorConfig {
                                    name: k.clone(),
                                    color: c.clone(),
                                });
                            }
                        }
                        out.colors_to_compare = vec;
                    }
                },
                |tbl, in_| {
                    if !in_.colors_to_compare.is_empty() {
                        let mut sub = toml::Table::new();
                        for item in &in_.colors_to_compare {
                            sub.insert(item.name.clone(), toml::Value::String(item.color.clone()));
                        }
                        tbl.insert("compare_colors".to_string(), toml::Value::Table(sub));
                    }
                },
            ),
            string_field("pre_command", |s| &s.pre_hook, |s, v| s.pre_hook = v),
            string_field("post_command", |s| &s.post_hook, |s, v| s.post_hook = v),
            string_field(
                "palette_export_kind",
                |s| &s.post_action,
                |s, v| s.post_action = v,
            ),
            i32_field(
                "palette_export_variant_index",
                |s| s.index,
                |s, v| s.index = v,
                None,
            ),
        ]
    });
    &S
}

fn templates_schema() -> &'static Schema<TemplatesConfig> {
    static S: LazyLock<Schema<TemplatesConfig>> = LazyLock::new(|| {
        vec![
            bool_field(
                "enable_builtin_templates",
                |s| s.enable_builtin_templates,
                |s, v| s.enable_builtin_templates = v,
            ),
            string_vec_field("builtin_ids", |s| &s.builtin_ids, |s, v| s.builtin_ids = v),
            bool_field(
                "enable_community_templates",
                |s| s.enable_community_templates,
                |s, v| s.enable_community_templates = v,
            ),
            string_vec_field(
                "community_ids",
                |s| &s.community_ids,
                |s, v| s.community_ids = v,
            ),
            custom_field(
                "custom_colors",
                |tbl, out: &mut TemplatesConfig, _parent, _diag| {
                    if let Some(toml::Value::Table(map)) = tbl.get("custom_colors") {
                        out.custom_colors.clear();
                        parse_custom_colors_map(map, &mut out.custom_colors);
                    }
                },
                |tbl, in_| {
                    if in_.custom_colors.is_empty() {
                        return;
                    }
                    let mut map = toml::Table::new();
                    for color in &in_.custom_colors {
                        let mut color_table = toml::Table::new();
                        color_table.insert(
                            "color".to_string(),
                            toml::Value::String(color.color.clone()),
                        );
                        color_table.insert("blend".to_string(), toml::Value::Boolean(color.blend));
                        if !color.color_dark.is_empty() {
                            color_table.insert(
                                "color_dark".to_string(),
                                toml::Value::String(color.color_dark.clone()),
                            );
                        }
                        if !color.color_light.is_empty() {
                            color_table.insert(
                                "color_light".to_string(),
                                toml::Value::String(color.color_light.clone()),
                            );
                        }
                        map.insert(color.name.clone(), toml::Value::Table(color_table));
                    }
                    tbl.insert("custom_colors".to_string(), toml::Value::Table(map));
                },
            ),
            named_map(
                "user",
                |s| &s.user_templates,
                |s, v| s.user_templates = v,
                user_template_schema(),
                |t, name| t.id = name.to_string(),
                |t| &t.id,
                true,
            ),
        ]
    });
    &S
}

/// Theme section schema.
pub fn theme_schema() -> &'static Schema<ThemeConfig> {
    static S: LazyLock<Schema<ThemeConfig>> = LazyLock::new(|| {
        vec![
            enum_field(
                "source",
                |s| s.source,
                |s, v| s.source = v,
                PALETTE_SOURCE_OPTIONS,
            ),
            string_field(
                "builtin",
                |s| &s.builtin_palette,
                |s, v| s.builtin_palette = v,
            ),
            string_field(
                "community_palette",
                |s| &s.community_palette,
                |s, v| s.community_palette = v,
            ),
            string_field(
                "custom_palette",
                |s| &s.custom_palette,
                |s, v| s.custom_palette = v,
            ),
            string_field(
                "wallpaper_scheme",
                |s| &s.wallpaper_scheme,
                |s, v| s.wallpaper_scheme = v,
            ),
            enum_field("mode", |s| s.mode, |s, v| s.mode = v, THEME_MODE_OPTIONS),
            bool_field(
                "pure_black_dark",
                |s| s.pure_black_dark,
                |s, v| s.pure_black_dark = v,
            ),
            sub_table(
                "templates",
                |s| &s.templates,
                |s, v| s.templates = v,
                templates_schema(),
            ),
        ]
    });
    &S
}

// ── Shell Sub-Schemas ──────────────────────────────────────────────────────

fn shell_animation_schema() -> &'static Schema<AnimationConfig> {
    static S: LazyLock<Schema<AnimationConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            f32_field(
                "speed",
                |s| s.speed,
                |s, v| s.speed = v,
                Some(ANIMATION_SPEED_RANGE),
            ),
        ]
    });
    &S
}

fn shell_shadow_schema() -> &'static Schema<ShadowConfig> {
    static S: LazyLock<Schema<ShadowConfig>> = LazyLock::new(|| {
        vec![
            string_field("direction", |s| &s.direction, |s, v| s.direction = v),
            f32_field("alpha", |s| s.alpha, |s, v| s.alpha = v, Some(UNIT_RANGE)),
        ]
    });
    &S
}

fn dmenu_entry_schema() -> &'static Schema<DmenuEntryConfig> {
    static S: LazyLock<Schema<DmenuEntryConfig>> = LazyLock::new(|| {
        vec![
            string_field("id", |s| &s.id, |s, v| s.id = v),
            string_field("exec", |s| &s.command, |s, v| s.command = v),
            optional_string_field("prefix", |s| &s.exec, |s, v| s.exec = v),
            optional_string_field("label", |s| &s.label, |s, v| s.label = v),
            optional_string_field("glyph", |s| &s.glyph, |s, v| s.glyph = v),
            bool_field("freeform", |s| s.freeform, |s, v| s.freeform = v),
        ]
    });
    &S
}

fn shell_launcher_dmenu_schema() -> &'static Schema<DmenuConfig> {
    static S: LazyLock<Schema<DmenuConfig>> = LazyLock::new(|| {
        vec![array_of(
            "entries",
            |s| &s.entries,
            |s, v| s.entries = v,
            dmenu_entry_schema(),
            |e| !e.id.is_empty(),
        )]
    });
    &S
}

fn shell_panel_schema() -> &'static Schema<PanelConfig> {
    static S: LazyLock<Schema<PanelConfig>> = LazyLock::new(|| {
        vec![
            string_field(
                "transparency_mode",
                |s| &s.transparency_mode,
                |s, v| s.transparency_mode = v,
            ),
            bool_field("borders", |s| s.borders, |s, v| s.borders = v),
            bool_field("shadow", |s| s.shadow, |s, v| s.shadow = v),
            bool_field(
                "list_item_background",
                |s| s.list_item_background,
                |s, v| s.list_item_background = v,
            ),
            string_field(
                "launcher_placement",
                |s| &s.launcher_placement,
                |s, v| s.launcher_placement = v,
            ),
            string_field(
                "clipboard_placement",
                |s| &s.clipboard_placement,
                |s, v| s.clipboard_placement = v,
            ),
            string_field(
                "control_center_placement",
                |s| &s.control_center_placement,
                |s, v| s.control_center_placement = v,
            ),
            string_field(
                "wallpaper_placement",
                |s| &s.wallpaper_placement,
                |s, v| s.wallpaper_placement = v,
            ),
            string_field(
                "session_placement",
                |s| &s.session_placement,
                |s, v| s.session_placement = v,
            ),
            string_field(
                "polkit_placement",
                |s| &s.polkit_placement,
                |s, v| s.polkit_placement = v,
            ),
            string_field(
                "launcher_position",
                |s| &s.launcher_position,
                |s, v| s.launcher_position = v,
            ),
            string_field(
                "clipboard_position",
                |s| &s.clipboard_position,
                |s, v| s.clipboard_position = v,
            ),
            string_field(
                "control_center_position",
                |s| &s.control_center_position,
                |s, v| s.control_center_position = v,
            ),
            string_field(
                "wallpaper_position",
                |s| &s.wallpaper_position,
                |s, v| s.wallpaper_position = v,
            ),
            string_field(
                "session_position",
                |s| &s.session_position,
                |s, v| s.session_position = v,
            ),
            string_field(
                "polkit_position",
                |s| &s.polkit_position,
                |s, v| s.polkit_position = v,
            ),
            i32_field(
                "floating_offset",
                |s| s.floating_offset,
                |s, v| s.floating_offset = v,
                Some(Range::new(Some(0), Some(100), None)),
            ),
            bool_field(
                "open_near_click_control_center",
                |s| s.open_near_click_control_center,
                |s, v| s.open_near_click_control_center = v,
            ),
            bool_field(
                "open_near_click_launcher",
                |s| s.open_near_click_launcher,
                |s, v| s.open_near_click_launcher = v,
            ),
            bool_field(
                "open_near_click_clipboard",
                |s| s.open_near_click_clipboard,
                |s, v| s.open_near_click_clipboard = v,
            ),
            bool_field(
                "open_near_click_wallpaper",
                |s| s.open_near_click_wallpaper,
                |s, v| s.open_near_click_wallpaper = v,
            ),
            bool_field(
                "open_near_click_session",
                |s| s.open_near_click_session,
                |s, v| s.open_near_click_session = v,
            ),
        ]
    });
    &S
}

fn launcher_provider_schema() -> &'static Schema<LauncherProviderConfig> {
    static S: LazyLock<Schema<LauncherProviderConfig>> = LazyLock::new(|| {
        vec![
            string_field("prefix", |s| &s.prefix, |s, v| s.prefix = v),
            optional_bool_field("global", |s| s.global, |s, v| s.global = v),
        ]
    });
    &S
}

fn shell_launcher_schema() -> &'static Schema<LauncherConfig> {
    static S: LazyLock<Schema<LauncherConfig>> = LazyLock::new(|| {
        vec![
            bool_field("categories", |s| s.categories, |s, v| s.categories = v),
            bool_field("show_icons", |s| s.show_icons, |s, v| s.show_icons = v),
            bool_field("compact", |s| s.compact, |s, v| s.compact = v),
            bool_field("app_grid", |s| s.app_grid, |s, v| s.app_grid = v),
            bool_field(
                "sort_by_usage",
                |s| s.sort_by_usage,
                |s, v| s.sort_by_usage = v,
            ),
            bool_field(
                "fetch_exchange_rates",
                |s| s.fetch_exchange_rates,
                |s, v| s.fetch_exchange_rates = v,
            ),
            string_field(
                "provider_prefix",
                |s| &s.provider_prefix,
                |s, v| s.provider_prefix = v,
            ),
            string_field("auto_paste", |s| &s.auto_paste, |s, v| s.auto_paste = v),
            sub_table(
                "dmenu",
                |s| &s.dmenu,
                |s, v| s.dmenu = v,
                shell_launcher_dmenu_schema(),
            ),
            named_map(
                "providers",
                |s| &s.providers,
                |s, v| s.providers = v,
                launcher_provider_schema(),
                |p, name| p.name = name.to_lowercase(),
                |p| &p.name,
                false,
            ),
        ]
    });
    &S
}

fn shell_screen_corners_schema() -> &'static Schema<ScreenCornersConfig> {
    static S: LazyLock<Schema<ScreenCornersConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            i32_field(
                "size",
                |s| s.size,
                |s, v| s.size = v,
                Some(SCREEN_CORNERS_SIZE_RANGE),
            ),
        ]
    });
    &S
}

fn shell_mpris_schema() -> &'static Schema<MprisConfig> {
    static S: LazyLock<Schema<MprisConfig>> = LazyLock::new(|| {
        vec![string_vec_field(
            "blacklist",
            |s| &s.blacklist,
            |s, v| s.blacklist = v,
        )]
    });
    &S
}

fn shell_screenshot_schema() -> &'static Schema<ScreenshotConfig> {
    static S: LazyLock<Schema<ScreenshotConfig>> = LazyLock::new(|| {
        vec![
            path_string_field("directory", |s| &s.directory, |s, v| s.directory = v),
            bool_field(
                "pipe_to_command",
                |s| s.pipe_to_command,
                |s, v| s.pipe_to_command = v,
            ),
        ]
    });
    &S
}

fn shell_privacy_schema() -> &'static Schema<PrivacyConfig> {
    static S: LazyLock<Schema<PrivacyConfig>> = LazyLock::new(|| {
        vec![
            string_field(
                "mic_filter_regex",
                |s| &s.mic_filter_regex,
                |s, v| s.mic_filter_regex = v,
            ),
            string_field(
                "cam_filter_regex",
                |s| &s.cam_filter_regex,
                |s, v| s.cam_filter_regex = v,
            ),
            string_field(
                "screen_filter_regex",
                |s| &s.screen_filter_regex,
                |s, v| s.screen_filter_regex = v,
            ),
        ]
    });
    &S
}

fn session_action_schema() -> &'static Schema<SessionPanelActionConfig> {
    static S: LazyLock<Schema<SessionPanelActionConfig>> = LazyLock::new(|| {
        vec![
            custom_field(
                "action",
                |tbl, out: &mut SessionPanelActionConfig, _parent, _diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("action") {
                        out.action = v.trim().to_lowercase();
                    }
                },
                |tbl, in_| {
                    tbl.insert(
                        "action".to_string(),
                        toml::Value::String(in_.action.clone()),
                    );
                },
            ),
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            optional_string_field("command", |s| &s.command, |s, v| s.command = v),
            optional_string_field("label", |s| &s.label, |s, v| s.label = v),
            optional_string_field("glyph", |s| &s.glyph, |s, v| s.glyph = v),
            string_field("variant", |s| &s.variant, |s, v| s.variant = v),
            f64_field(
                "countdown_seconds",
                |s| s.countdown_seconds,
                |s, v| s.countdown_seconds = v,
                None,
            ),
        ]
    });
    &S
}

fn shell_session_power_schema() -> &'static Schema<ShellSessionPowerConfig> {
    static S: LazyLock<Schema<ShellSessionPowerConfig>> = LazyLock::new(|| {
        vec![
            optional_trimmed_string_field("suspend", |s| &s.suspend, |s, v| s.suspend = v),
            optional_trimmed_string_field("reboot", |s| &s.reboot, |s, v| s.reboot = v),
            optional_trimmed_string_field("shutdown", |s| &s.shutdown, |s, v| s.shutdown = v),
        ]
    });
    &S
}

fn shell_greeter_sync_schema() -> &'static Schema<ShellGreeterSyncConfig> {
    static S: LazyLock<Schema<ShellGreeterSyncConfig>> = LazyLock::new(|| {
        vec![
            bool_field("auto_sync", |s| s.auto_sync, |s, v| s.auto_sync = v),
            custom_field(
                "privilege_command",
                |tbl, out: &mut ShellGreeterSyncConfig, _parent, _diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("privilege_command") {
                        out.privilege_command = v.trim().to_string();
                    }
                },
                |tbl, in_| {
                    if !in_.privilege_command.is_empty() {
                        tbl.insert(
                            "privilege_command".to_string(),
                            toml::Value::String(in_.privilege_command.clone()),
                        );
                    }
                },
            ),
        ]
    });
    &S
}

fn shell_session_schema() -> &'static Schema<ShellSessionConfig> {
    static S: LazyLock<Schema<ShellSessionConfig>> = LazyLock::new(|| {
        vec![
            array_of(
                "actions",
                |s| &s.actions,
                |s, v| s.actions = v,
                session_action_schema(),
                |a| !a.action.is_empty(),
            ),
            sub_table(
                "power",
                |s| &s.power,
                |s, v| s.power = v,
                shell_session_power_schema(),
            ),
        ]
    });
    &S
}

/// Shell section schema.
pub fn shell_schema() -> &'static Schema<ShellConfig> {
    static S: LazyLock<Schema<ShellConfig>> = LazyLock::new(|| {
        vec![
            f32_field(
                "corner_radius_scale",
                |s| s.corner_radius_scale,
                |s, v| s.corner_radius_scale = v,
                Some(CORNER_RADIUS_SCALE_RANGE),
            ),
            bool_field(
                "button_borders",
                |s| s.button_borders,
                |s, v| s.button_borders = v,
            ),
            bool_field(
                "input_borders",
                |s| s.input_borders,
                |s, v| s.input_borders = v,
            ),
            bool_field(
                "popup_borders",
                |s| s.popup_borders,
                |s, v| s.popup_borders = v,
            ),
            bool_field(
                "popup_shadows",
                |s| s.popup_shadows,
                |s, v| s.popup_shadows = v,
            ),
            bool_field(
                "card_borders",
                |s| s.card_borders,
                |s, v| s.card_borders = v,
            ),
            custom_field(
                "font_family",
                |tbl, out: &mut ShellConfig, _parent, _diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("font_family") {
                        let trimmed = v.trim().to_string();
                        out.font_family = if trimmed.is_empty() {
                            "sans-serif".to_string()
                        } else {
                            trimmed
                        };
                    }
                },
                |tbl, in_| {
                    tbl.insert(
                        "font_family".to_string(),
                        toml::Value::String(in_.font_family.clone()),
                    );
                },
            ),
            string_if_non_empty_field("lang", |s| &s.lang, |s, v| s.lang = v),
            string_field("time_format", |s| &s.time_format, |s, v| s.time_format = v),
            string_field("date_format", |s| &s.date_format, |s, v| s.date_format = v),
            bool_field(
                "offline_mode",
                |s| s.offline_mode,
                |s, v| s.offline_mode = v,
            ),
            string_if_non_empty_field(
                "panel_anchor_bar",
                |s| &s.panel_anchor_bar,
                |s, v| s.panel_anchor_bar = v,
            ),
            bool_field(
                "external_ip_enabled",
                |s| s.external_ip_enabled,
                |s, v| s.external_ip_enabled = v,
            ),
            bool_field(
                "telemetry_enabled",
                |s| s.telemetry_enabled,
                |s, v| s.telemetry_enabled = v,
            ),
            bool_field(
                "setup_wizard_enabled",
                |s| s.setup_wizard_enabled,
                |s, v| s.setup_wizard_enabled = v,
            ),
            bool_field(
                "niri_overview_type_to_launch_enabled",
                |s| s.niri_overview_type_to_launch_enabled,
                |s, v| s.niri_overview_type_to_launch_enabled = v,
            ),
            bool_field(
                "polkit_agent",
                |s| s.polkit_agent,
                |s, v| s.polkit_agent = v,
            ),
            string_field(
                "password_style",
                |s| &s.password_mask_style,
                |s, v| s.password_mask_style = v,
            ),
            bool_field(
                "settings_show_advanced",
                |s| s.settings_show_advanced,
                |s, v| s.settings_show_advanced = v,
            ),
            bool_field(
                "show_location",
                |s| s.show_location,
                |s, v| s.show_location = v,
            ),
            bool_field(
                "app_icon_colorize",
                |s| s.app_icon_colorize,
                |s, v| s.app_icon_colorize = v,
            ),
            color_spec_field(
                "app_icon_color",
                |s| &s.app_icon_color,
                |s, v| s.app_icon_color = v,
                false,
            ),
            bool_field(
                "launch_apps_as_systemd_services",
                |s| s.launch_apps_as_systemd_services,
                |s, v| s.launch_apps_as_systemd_services = v,
            ),
            string_field(
                "launch_apps_custom_command",
                |s| &s.launch_apps_custom_command,
                |s, v| s.launch_apps_custom_command = v,
            ),
            bool_field(
                "clipboard_enabled",
                |s| s.clipboard_enabled,
                |s, v| s.clipboard_enabled = v,
            ),
            bool_field(
                "clipboard_keep_from_closed_apps",
                |s| s.clipboard_keep_from_closed_apps,
                |s, v| s.clipboard_keep_from_closed_apps = v,
            ),
            i32_field(
                "clipboard_history_max_entries",
                |s| s.clipboard_history_max_entries,
                |s, v| s.clipboard_history_max_entries = v,
                Some(CLIPBOARD_HISTORY_MAX_ENTRIES_RANGE),
            ),
            bool_field(
                "clipboard_confirm_clear_history",
                |s| s.clipboard_confirm_clear_history,
                |s, v| s.clipboard_confirm_clear_history = v,
            ),
            bool_field(
                "screen_time_enabled",
                |s| s.screen_time_enabled,
                |s, v| s.screen_time_enabled = v,
            ),
            bool_field(
                "shared_gl_context",
                |s| s.shared_gl_context,
                |s, v| s.shared_gl_context = v,
            ),
            bool_field(
                "disable_mipmaps",
                |s| s.disable_mipmaps,
                |s, v| s.disable_mipmaps = v,
            ),
            string_field(
                "clipboard_auto_paste",
                |s| &s.clipboard_auto_paste,
                |s, v| s.clipboard_auto_paste = v,
            ),
            string_field(
                "clipboard_image_action_command",
                |s| &s.clipboard_image_action_command,
                |s, v| s.clipboard_image_action_command = v,
            ),
            path_string_field("avatar_path", |s| &s.avatar_path, |s, v| s.avatar_path = v),
            sub_table(
                "animation",
                |s| &s.animation,
                |s, v| s.animation = v,
                shell_animation_schema(),
            ),
            sub_table(
                "shadow",
                |s| &s.shadow,
                |s, v| s.shadow = v,
                shell_shadow_schema(),
            ),
            sub_table(
                "panel",
                |s| &s.panel,
                |s, v| s.panel = v,
                shell_panel_schema(),
            ),
            sub_table(
                "launcher",
                |s| &s.launcher,
                |s, v| s.launcher = v,
                shell_launcher_schema(),
            ),
            sub_table(
                "screen_corners",
                |s| &s.screen_corners,
                |s, v| s.screen_corners = v,
                shell_screen_corners_schema(),
            ),
            sub_table(
                "mpris",
                |s| &s.mpris,
                |s, v| s.mpris = v,
                shell_mpris_schema(),
            ),
            sub_table(
                "screenshot",
                |s| &s.screenshot,
                |s, v| s.screenshot = v,
                shell_screenshot_schema(),
            ),
            sub_table(
                "privacy",
                |s| &s.privacy,
                |s, v| s.privacy = v,
                shell_privacy_schema(),
            ),
            sub_table(
                "session",
                |s| &s.session,
                |s, v| s.session = v,
                shell_session_schema(),
            ),
            sub_table(
                "greeter_sync",
                |s| &s.greeter_sync,
                |s, v| s.greeter_sync = v,
                shell_greeter_sync_schema(),
            ),
        ]
    });
    &S
}

/// Accessibility section schema.
pub fn accessibility_schema() -> &'static Schema<AccessibilityConfig> {
    static S: LazyLock<Schema<AccessibilityConfig>> = LazyLock::new(|| {
        vec![
            f32_field(
                "ui_scale",
                |s| s.ui_scale,
                |s, v| s.ui_scale = v,
                Some(SCALE_RANGE),
            ),
            bool_field(
                "high_contrast",
                |s| s.high_contrast,
                |s, v| s.high_contrast = v,
            ),
        ]
    });
    &S
}

/// Storage section schema.
pub fn storage_schema() -> &'static Schema<StorageConfig> {
    static S: LazyLock<Schema<StorageConfig>> = LazyLock::new(|| {
        vec![
            custom_field(
                "key_source",
                |tbl, out: &mut StorageConfig, parent, diag| {
                    if let Some(toml::Value::String(value)) = tbl.get("key_source") {
                        let source = value.trim();
                        if source == "secret-service" {
                            out.key_source = StorageKeySource::SecretService;
                        } else if source == "file" {
                            out.key_source = StorageKeySource::File;
                        } else {
                            diag.error(
                                join_path(parent, "key_source"),
                                r#"key_source must be "secret-service" or "file""#,
                            );
                        }
                    }
                },
                |tbl, in_| {
                    tbl.insert(
                        "key_source".to_string(),
                        toml::Value::String(if in_.key_source == StorageKeySource::File {
                            "file".to_string()
                        } else {
                            "secret-service".to_string()
                        }),
                    );
                },
            ),
            path_string_field("key_file", |s| &s.key_file, |s, v| s.key_file = v),
            finalize(
                |out: &mut StorageConfig, parent_path: &str, diag: &mut Diagnostics| {
                    if out.key_source == StorageKeySource::File {
                        if out.key_file.is_empty() {
                            diag.error(
                                join_path(parent_path, "key_file"),
                                r#"storage with key_source = "file" requires key_file"#,
                            );
                        } else if !std::path::Path::new(&out.key_file).is_absolute() {
                            diag.error(
                                join_path(parent_path, "key_file"),
                                "key_file must resolve to an absolute path",
                            );
                        }
                    } else if !out.key_file.is_empty() {
                        diag.error(
                            join_path(parent_path, "key_file"),
                            r#"key_file requires key_source = "file""#,
                        );
                    }
                },
            ),
        ]
    });
    &S
}

// ── Bar Schemas ────────────────────────────────────────────────────────────

const BAR_THICKNESS_RANGE: Range<i64> = Range::new(Some(10), Some(300), None);
const BAR_RADIUS_RANGE: Range<i64> = Range::new(Some(0), Some(500), None);
const BAR_PANEL_OVERLAP_RANGE: Range<i64> = Range::new(Some(-2), Some(3), None);
const BAR_CAPSULE_THICKNESS_RANGE: Range<f64> = Range::new(Some(0.1), Some(1.0), None);
const BAR_OPACITY_RANGE: Range<f64> = Range::new(Some(0.0), Some(1.0), None);
const BAR_BORDER_WIDTH_RANGE: Range<f64> = Range::new(Some(0.0), Some(20.0), None);
const BAR_SCALE_RANGE: Range<f64> = Range::new(Some(0.5), Some(4.0), None);
const BAR_CAPSULE_PADDING_RANGE: Range<f64> = Range::new(Some(0.0), Some(48.0), None);
const BAR_CAPSULE_RADIUS_RANGE: Range<f64> = Range::new(Some(0.0), Some(80.0), None);

fn bar_capsule_group_schema() -> &'static Schema<BarCapsuleGroupStyle> {
    static S: LazyLock<Schema<BarCapsuleGroupStyle>> = LazyLock::new(|| {
        vec![
            custom_field(
                "id",
                |tbl, out: &mut BarCapsuleGroupStyle, _parent, _diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("id") {
                        out.id = v.trim().to_string();
                    }
                },
                |tbl, in_| {
                    tbl.insert("id".to_string(), toml::Value::String(in_.id.clone()));
                },
            ),
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            string_vec_field("members", |s| &s.members, |s, v| s.members = v),
            color_field("fill", |s| &s.fill, |s, v| s.fill = v),
            capsule_border_field(
                "border",
                |s| &s.border,
                |s| s.border_specified,
                |s, v| s.border = v,
                |s, v| s.border_specified = v,
            ),
            optional_color_field("foreground", |s| &s.foreground, |s, v| s.foreground = v),
            f32_field(
                "padding",
                |s| s.padding,
                |s, v| s.padding = v,
                Some(BAR_CAPSULE_PADDING_RANGE),
            ),
            optional_f32_field(
                "radius",
                |s| s.radius,
                |s, v| s.radius = v,
                Some(BAR_CAPSULE_RADIUS_RANGE),
            ),
            f32_field(
                "opacity",
                |s| s.opacity,
                |s, v| s.opacity = v,
                Some(BAR_OPACITY_RANGE),
            ),
        ]
    });
    &S
}

fn bar_layer_field<S: Send + Sync + 'static>(
    get: fn(&S) -> &str,
    set: fn(&mut S, String),
) -> Field<S> {
    custom_field(
        "layer",
        move |tbl, out, parent, diag| {
            if let Some(toml::Value::String(v)) = tbl.get("layer") {
                if v == "top" || v == "overlay" {
                    set(out, v.clone());
                } else {
                    diag.warn(
                        join_path(parent, "layer"),
                        format!("expected top or overlay, got \"{v}\""),
                    );
                }
            }
        },
        move |tbl, in_| {
            tbl.insert(
                "layer".to_string(),
                toml::Value::String(get(in_).to_string()),
            );
        },
    )
}

fn bar_radius_field() -> Field<BarConfig> {
    custom_field(
        "radius",
        |tbl, out: &mut BarConfig, _parent, _diag| {
            if let Some(toml::Value::Integer(v)) = tbl.get("radius") {
                let r = apply_range(*v, &BAR_RADIUS_RANGE) as i32;
                out.radius = r;
                out.radius_top_left = r;
                out.radius_top_right = r;
                out.radius_bottom_left = r;
                out.radius_bottom_right = r;
            }
        },
        |tbl, in_| {
            tbl.insert(
                "radius".to_string(),
                toml::Value::Integer(i64::from(in_.radius)),
            );
        },
    )
}

fn bar_dead_zone_schema() -> &'static Schema<BarDeadZoneConfig> {
    static S: LazyLock<Schema<BarDeadZoneConfig>> = LazyLock::new(|| {
        vec![string_map_field(
            "actions",
            |s| &s.actions,
            |s, v| s.actions = v,
        )]
    });
    &S
}

fn bar_dead_zone_override_schema() -> &'static Schema<BarDeadZoneOverride> {
    static S: LazyLock<Schema<BarDeadZoneOverride>> = LazyLock::new(|| {
        vec![optional_string_map_field(
            "actions",
            |s| &s.actions,
            |s, v| s.actions = v,
        )]
    });
    &S
}

pub fn bar_fields_schema() -> &'static Schema<BarConfig> {
    static S: LazyLock<Schema<BarConfig>> = LazyLock::new(|| {
        vec![
            bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            bool_field("auto_hide", |s| s.auto_hide, |s, v| s.auto_hide = v),
            bool_field(
                "smart_auto_hide",
                |s| s.smart_auto_hide,
                |s, v| s.smart_auto_hide = v,
            ),
            bool_field(
                "show_on_workspace_switch",
                |s| s.show_on_workspace_switch,
                |s, v| s.show_on_workspace_switch = v,
            ),
            bool_field(
                "reserve_space",
                |s| s.reserve_space,
                |s, v| s.reserve_space = v,
            ),
            bar_layer_field(|s| &s.layer, |s, v| s.layer = v),
            i32_field(
                "thickness",
                |s| s.thickness,
                |s, v| s.thickness = v,
                Some(BAR_THICKNESS_RANGE),
            ),
            f32_field(
                "background_opacity",
                |s| s.background_opacity,
                |s, v| s.background_opacity = v,
                Some(BAR_OPACITY_RANGE),
            ),
            color_field("border", |s| &s.border, |s, v| s.border = v),
            f32_field(
                "border_width",
                |s| s.border_width,
                |s, v| s.border_width = v,
                Some(BAR_BORDER_WIDTH_RANGE),
            ),
            bar_radius_field(),
            i32_field(
                "radius_top_left",
                |s| s.radius_top_left,
                |s, v| s.radius_top_left = v,
                Some(BAR_RADIUS_RANGE),
            ),
            i32_field(
                "radius_top_right",
                |s| s.radius_top_right,
                |s, v| s.radius_top_right = v,
                Some(BAR_RADIUS_RANGE),
            ),
            i32_field(
                "radius_bottom_left",
                |s| s.radius_bottom_left,
                |s, v| s.radius_bottom_left = v,
                Some(BAR_RADIUS_RANGE),
            ),
            i32_field(
                "radius_bottom_right",
                |s| s.radius_bottom_right,
                |s, v| s.radius_bottom_right = v,
                Some(BAR_RADIUS_RANGE),
            ),
            bool_field(
                "concave_edge_corners",
                |s| s.concave_edge_corners,
                |s, v| s.concave_edge_corners = v,
            ),
            i32_field(
                "margin_ends",
                |s| s.margin_ends,
                |s, v| s.margin_ends = v,
                None,
            ),
            i32_field(
                "margin_edge",
                |s| s.margin_edge,
                |s, v| s.margin_edge = v,
                None,
            ),
            i32_field(
                "margin_opposite_edge",
                |s| s.margin_opposite_edge,
                |s, v| s.margin_opposite_edge = v,
                None,
            ),
            i32_field("padding", |s| s.padding, |s, v| s.padding = v, None),
            i32_field(
                "widget_spacing",
                |s| s.widget_spacing,
                |s, v| s.widget_spacing = v,
                None,
            ),
            bool_field("shadow", |s| s.shadow, |s, v| s.shadow = v),
            bool_field(
                "contact_shadow",
                |s| s.contact_shadow,
                |s, v| s.contact_shadow = v,
            ),
            i32_field(
                "panel_overlap",
                |s| s.panel_overlap,
                |s, v| s.panel_overlap = v,
                Some(BAR_PANEL_OVERLAP_RANGE),
            ),
            f32_field(
                "capsule_thickness",
                |s| s.capsule_thickness,
                |s, v| s.capsule_thickness = v,
                Some(BAR_CAPSULE_THICKNESS_RANGE),
            ),
            f32_field(
                "scale",
                |s| s.scale,
                |s, v| s.scale = v,
                Some(BAR_SCALE_RANGE),
            ),
            i32_field(
                "font_weight",
                |s| s.font_weight,
                |s, v| s.font_weight = v,
                None,
            ),
            optional_trimmed_string_field(
                "font_family",
                |s| &s.font_family,
                |s, v| s.font_family = v,
            ),
            string_vec_field("start", |s| &s.start_widgets, |s, v| s.start_widgets = v),
            string_vec_field("center", |s| &s.center_widgets, |s, v| s.center_widgets = v),
            string_vec_field("end", |s| &s.end_widgets, |s, v| s.end_widgets = v),
            bool_field(
                "capsule",
                |s| s.widget_capsule_default,
                |s, v| s.widget_capsule_default = v,
            ),
            color_field(
                "capsule_fill",
                |s| &s.widget_capsule_fill,
                |s, v| s.widget_capsule_fill = v,
            ),
            optional_color_field(
                "capsule_foreground",
                |s| &s.widget_capsule_foreground,
                |s, v| s.widget_capsule_foreground = v,
            ),
            optional_color_field("color", |s| &s.widget_color, |s, v| s.widget_color = v),
            optional_color_field(
                "icon_color",
                |s| &s.widget_icon_color,
                |s, v| s.widget_icon_color = v,
            ),
            array_of(
                "capsule_group",
                |s| &s.widget_capsule_groups,
                |s, v| s.widget_capsule_groups = v,
                bar_capsule_group_schema(),
                |g| !g.id.is_empty(),
            ),
            f32_field(
                "capsule_padding",
                |s| s.widget_capsule_padding,
                |s, v| s.widget_capsule_padding = v,
                Some(BAR_CAPSULE_PADDING_RANGE),
            ),
            optional_f64_field(
                "capsule_radius",
                |s| s.widget_capsule_radius,
                |s, v| s.widget_capsule_radius = v,
            ),
            f32_field(
                "capsule_opacity",
                |s| s.widget_capsule_opacity,
                |s, v| s.widget_capsule_opacity = v,
                Some(BAR_OPACITY_RANGE),
            ),
            capsule_border_field(
                "capsule_border",
                |s| &s.widget_capsule_border,
                |s| s.widget_capsule_border_specified,
                |s, v| s.widget_capsule_border = v,
                |s, v| s.widget_capsule_border_specified = v,
            ),
            bool_field(
                "hover_highlight",
                |s| s.hover_highlight,
                |s, v| s.hover_highlight = v,
            ),
            sub_table(
                "dead_zone",
                |s| &s.dead_zone,
                |s, v| s.dead_zone = v,
                bar_dead_zone_schema(),
            ),
            string_map_field("actions", |s| &s.actions, |s, v| s.actions = v),
        ]
    });
    &S
}

pub fn bar_monitor_override_schema() -> &'static Schema<BarMonitorOverride> {
    static S: LazyLock<Schema<BarMonitorOverride>> = LazyLock::new(|| {
        vec![
            string_field("match", |s| &s.match_, |s, v| s.match_ = v),
            optional_string_field("position", |s| &s.position, |s, v| s.position = v),
            optional_bool_field("enabled", |s| s.enabled, |s, v| s.enabled = v),
            optional_bool_field("auto_hide", |s| s.auto_hide, |s, v| s.auto_hide = v),
            optional_bool_field(
                "smart_auto_hide",
                |s| s.smart_auto_hide,
                |s, v| s.smart_auto_hide = v,
            ),
            optional_bool_field(
                "show_on_workspace_switch",
                |s| s.show_on_workspace_switch,
                |s, v| s.show_on_workspace_switch = v,
            ),
            optional_bool_field(
                "reserve_space",
                |s| s.reserve_space,
                |s, v| s.reserve_space = v,
            ),
            custom_field(
                "layer",
                |tbl, out: &mut BarMonitorOverride, parent, diag| {
                    if let Some(toml::Value::String(v)) = tbl.get("layer") {
                        if v == "top" || v == "overlay" {
                            out.layer = Some(v.clone());
                        } else {
                            diag.warn(
                                join_path(parent, "layer"),
                                format!("expected top or overlay, got \"{v}\""),
                            );
                        }
                    }
                },
                |tbl, in_| {
                    if let Some(ref l) = in_.layer {
                        tbl.insert("layer".to_string(), toml::Value::String(l.clone()));
                    }
                },
            ),
            optional_i32_field(
                "thickness",
                |s| s.thickness,
                |s, v| s.thickness = v,
                Some(BAR_THICKNESS_RANGE),
            ),
            optional_f32_field(
                "background_opacity",
                |s| s.background_opacity,
                |s, v| s.background_opacity = v,
                Some(BAR_OPACITY_RANGE),
            ),
            optional_color_field("border", |s| &s.border, |s, v| s.border = v),
            optional_f32_field(
                "border_width",
                |s| s.border_width,
                |s, v| s.border_width = v,
                Some(BAR_BORDER_WIDTH_RANGE),
            ),
            optional_i32_field(
                "radius",
                |s| s.radius,
                |s, v| s.radius = v,
                Some(BAR_RADIUS_RANGE),
            ),
            optional_i32_field(
                "radius_top_left",
                |s| s.radius_top_left,
                |s, v| s.radius_top_left = v,
                Some(BAR_RADIUS_RANGE),
            ),
            optional_i32_field(
                "radius_top_right",
                |s| s.radius_top_right,
                |s, v| s.radius_top_right = v,
                Some(BAR_RADIUS_RANGE),
            ),
            optional_i32_field(
                "radius_bottom_left",
                |s| s.radius_bottom_left,
                |s, v| s.radius_bottom_left = v,
                Some(BAR_RADIUS_RANGE),
            ),
            optional_i32_field(
                "radius_bottom_right",
                |s| s.radius_bottom_right,
                |s, v| s.radius_bottom_right = v,
                Some(BAR_RADIUS_RANGE),
            ),
            optional_bool_field(
                "concave_edge_corners",
                |s| s.concave_edge_corners,
                |s, v| s.concave_edge_corners = v,
            ),
            optional_i32_field(
                "margin_ends",
                |s| s.margin_ends,
                |s, v| s.margin_ends = v,
                None,
            ),
            optional_i32_field(
                "margin_edge",
                |s| s.margin_edge,
                |s, v| s.margin_edge = v,
                None,
            ),
            optional_i32_field(
                "margin_opposite_edge",
                |s| s.margin_opposite_edge,
                |s, v| s.margin_opposite_edge = v,
                None,
            ),
            optional_i32_field("padding", |s| s.padding, |s, v| s.padding = v, None),
            optional_i32_field(
                "widget_spacing",
                |s| s.widget_spacing,
                |s, v| s.widget_spacing = v,
                None,
            ),
            optional_f32_field(
                "scale",
                |s| s.scale,
                |s, v| s.scale = v,
                Some(BAR_SCALE_RANGE),
            ),
            optional_bool_field("shadow", |s| s.shadow, |s, v| s.shadow = v),
            optional_bool_field(
                "contact_shadow",
                |s| s.contact_shadow,
                |s, v| s.contact_shadow = v,
            ),
            optional_i32_field(
                "panel_overlap",
                |s| s.panel_overlap,
                |s, v| s.panel_overlap = v,
                Some(BAR_PANEL_OVERLAP_RANGE),
            ),
            optional_f32_field(
                "capsule_thickness",
                |s| s.capsule_thickness,
                |s, v| s.capsule_thickness = v,
                Some(BAR_CAPSULE_THICKNESS_RANGE),
            ),
            optional_trimmed_string_field(
                "font_family",
                |s| &s.font_family,
                |s, v| s.font_family = v,
            ),
            optional_string_vector_field("start", |s| &s.start_widgets, |s, v| s.start_widgets = v),
            optional_string_vector_field(
                "center",
                |s| &s.center_widgets,
                |s, v| s.center_widgets = v,
            ),
            optional_string_vector_field("end", |s| &s.end_widgets, |s, v| s.end_widgets = v),
            optional_bool_field(
                "capsule",
                |s| s.widget_capsule_default,
                |s, v| s.widget_capsule_default = v,
            ),
            optional_color_field(
                "capsule_fill",
                |s| &s.widget_capsule_fill,
                |s, v| s.widget_capsule_fill = v,
            ),
            optional_color_field(
                "capsule_foreground",
                |s| &s.widget_capsule_foreground,
                |s, v| s.widget_capsule_foreground = v,
            ),
            optional_color_field("color", |s| &s.widget_color, |s, v| s.widget_color = v),
            optional_color_field(
                "icon_color",
                |s| &s.widget_icon_color,
                |s, v| s.widget_icon_color = v,
            ),
            optional_f64_field(
                "capsule_padding",
                |s| s.widget_capsule_padding,
                |s, v| s.widget_capsule_padding = v,
            ),
            optional_f64_field(
                "capsule_radius",
                |s| s.widget_capsule_radius,
                |s, v| s.widget_capsule_radius = v,
            ),
            optional_f64_field(
                "capsule_opacity",
                |s| s.widget_capsule_opacity,
                |s, v| s.widget_capsule_opacity = v,
            ),
            capsule_border_field(
                "capsule_border",
                |s| &s.widget_capsule_border,
                |s| s.widget_capsule_border_specified,
                |s, v| s.widget_capsule_border = v,
                |s, v| s.widget_capsule_border_specified = v,
            ),
            optional_bool_field(
                "hover_highlight",
                |s| s.hover_highlight,
                |s, v| s.hover_highlight = v,
            ),
            custom_field(
                "capsule_group",
                |tbl, out: &mut BarMonitorOverride, parent_path, diag| {
                    if let Some(toml::Value::Array(arr)) = tbl.get("capsule_group") {
                        let mut groups = Vec::new();
                        for node in arr {
                            if let toml::Value::Table(sub) = node {
                                let mut g = BarCapsuleGroupStyle::default();
                                read_into(
                                    sub,
                                    &mut g,
                                    bar_capsule_group_schema(),
                                    &join_path(parent_path, "capsule_group"),
                                    diag,
                                );
                                if !g.id.is_empty() {
                                    groups.push(g);
                                }
                            }
                        }
                        out.widget_capsule_groups = Some(groups);
                    }
                },
                |_tbl, _in_| {},
            ),
            sub_table(
                "dead_zone",
                |s| &s.dead_zone,
                |s, v| s.dead_zone = v,
                bar_dead_zone_override_schema(),
            ),
        ]
    });
    &S
}

pub fn optional_f32_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> Option<f32>,
    set: fn(&mut S, Option<f32>),
    range: Option<Range<f64>>,
) -> Field<S> {
    custom_field(
        key,
        move |tbl, out, _parent, _diag| {
            if let Some(val) = tbl.get(key).and_then(|v| match v {
                toml::Value::Float(f) if f.is_finite() => Some(*f),
                toml::Value::Integer(i) => Some(*i as f64),
                _ => None,
            }) {
                let v = if let Some(ref r) = range {
                    apply_range(val, r) as f32
                } else {
                    val as f32
                };
                set(out, Some(v));
            }
        },
        move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::Float(f64::from(v)));
            }
        },
    )
}

// ── Path resolution ────────────────────────────────────────────────────────

fn nested_from_path(path: &[String], from: usize) -> toml::Table {
    let mut root = toml::Table::new();
    if from >= path.len() {
        return root;
    }
    if from == path.len() - 1 {
        root.insert(path[from].clone(), toml::Value::Integer(0));
    } else {
        let child = nested_from_path(path, from + 1);
        root.insert(path[from].clone(), toml::Value::Table(child));
    }
    root
}

fn collect_unknown_in_section(section: &str, tbl: &toml::Table, unknown: &mut Vec<String>) -> bool {
    if let Some(spec) = find_section(section) {
        (spec.collect_unknown)(tbl, unknown);
        return true;
    }
    if section == "desktop_widgets" {
        collect_unknown_keys(tbl, desktop_widgets_schema(), section, unknown);
        return true;
    }
    if section == "lockscreen_widgets" {
        collect_unknown_keys(tbl, lockscreen_widgets_schema(), section, unknown);
        return true;
    }
    false
}

fn is_known_desktop_widget_path(path: &[String]) -> bool {
    if path.len() == 2 && path[1] == "widget_order" {
        return true;
    }
    if path.len() < 2 || path[1] != "widget" {
        return false;
    }
    if path.len() <= 3 {
        return true;
    }
    let widget_keys: HashSet<&str> = [
        "id",
        "type",
        "output",
        "cx",
        "cy",
        "box_width",
        "box_height",
        "rotation",
        "flip_x",
        "flip_y",
        "enabled",
        "settings",
    ]
    .into_iter()
    .collect();

    if !widget_keys.contains(path[3].as_str()) {
        return false;
    }
    path.len() == 4 || path[3] == "settings"
}

/// Port of `isKnownConfigPath` (config_schema.h:66 / config_schema.cpp:1673-1720).
pub fn is_known_config_path(path: &[String]) -> bool {
    if path.is_empty() {
        return false;
    }
    let section = &path[0];

    if section == "bar" {
        if path.len() <= 2 {
            return true;
        }
        if path[2] == "monitor" {
            if path.len() <= 4 {
                return true;
            }
            let mut unknown = Vec::new();
            collect_unknown_keys(
                &nested_from_path(path, 4),
                bar_monitor_override_schema(),
                "bar",
                &mut unknown,
            );
            return unknown.is_empty();
        }
        if path.len() == 3 && (path[2] == "position" || path[2] == "name") {
            return true;
        }
        let mut unknown = Vec::new();
        collect_unknown_keys(
            &nested_from_path(path, 2),
            bar_fields_schema(),
            "bar",
            &mut unknown,
        );
        return unknown.is_empty();
    }

    if section == "desktop_widgets" && is_known_desktop_widget_path(path) {
        return true;
    }

    if section == "plugin_settings" {
        return path.len() <= 3;
    }

    if path.len() < 2 {
        return false;
    }
    let mut unknown = Vec::new();
    if !collect_unknown_in_section(section, &nested_from_path(path, 1), &mut unknown) {
        return false;
    }
    unknown.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(segs: &[&str]) -> Vec<String> {
        segs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn config_path_resolution() {
        // Known override paths
        assert!(is_known_config_path(&path(&["accessibility", "ui_scale"])));
        assert!(is_known_config_path(&path(&[
            "shell",
            "animation",
            "speed"
        ])));
        assert!(is_known_config_path(&path(&["shell", "panel_anchor_bar"])));
        assert!(is_known_config_path(&path(&[
            "shell",
            "panel",
            "control_center_placement"
        ])));
        assert!(is_known_config_path(&path(&["shell", "shadow", "alpha"])));
        assert!(is_known_config_path(&path(&[
            "shell",
            "screen_corners",
            "size"
        ])));
        assert!(is_known_config_path(&path(&[
            "shell",
            "screenshot",
            "directory"
        ])));
        assert!(is_known_config_path(&path(&[
            "system",
            "monitor",
            "cpu_poll_seconds"
        ])));
        assert!(is_known_config_path(&path(&["theme", "mode"])));
        assert!(is_known_config_path(&path(&[
            "theme",
            "templates",
            "enable_builtin_templates"
        ])));
        assert!(is_known_config_path(&path(&[
            "wallpaper",
            "automation",
            "interval_seconds"
        ])));
        assert!(is_known_config_path(&path(&["wallpaper", "fill_color"])));
        assert!(is_known_config_path(&path(&["dock", "icon_size"])));
        assert!(is_known_config_path(&path(&["dock", "radius_top_left"])));
        assert!(is_known_config_path(&path(&["desktop_widgets", "enabled"])));
        assert!(is_known_config_path(&path(&[
            "desktop_widgets",
            "grid",
            "cell_size"
        ])));
        assert!(is_known_config_path(&path(&[
            "desktop_widgets",
            "widget_order"
        ])));
        assert!(is_known_config_path(&path(&[
            "desktop_widgets",
            "widget",
            "clock1",
            "type"
        ])));
        assert!(is_known_config_path(&path(&[
            "desktop_widgets",
            "widget",
            "clock1",
            "settings",
            "format"
        ])));
        assert!(is_known_config_path(&path(&["osd", "scale"])));
        assert!(is_known_config_path(&path(&[
            "notification",
            "background_opacity"
        ])));
        assert!(is_known_config_path(&path(&[
            "battery",
            "warning_threshold"
        ])));
        assert!(is_known_config_path(&path(&[
            "calendar",
            "refresh_minutes"
        ])));
        assert!(is_known_config_path(&path(&[
            "calendar", "account", "icloud", "provider"
        ])));
        assert!(is_known_config_path(&path(&[
            "control_center",
            "calendar",
            "show_events_card"
        ])));
        assert!(is_known_config_path(&path(&[
            "nightlight",
            "temperature_day"
        ])));
        assert!(is_known_config_path(&path(&["location", "auto_locate"])));
        assert!(is_known_config_path(&path(&["keybinds", "validate"])));
        assert!(is_known_config_path(&path(&["control_center", "sidebar"])));
        assert!(is_known_config_path(&path(&["hooks", "wallpaper_changed"])));
        assert!(is_known_config_path(&path(&[
            "bar",
            "default",
            "thickness"
        ])));
        assert!(is_known_config_path(&path(&[
            "bar",
            "default",
            "concave_edge_corners"
        ])));
        assert!(is_known_config_path(&path(&["bar", "default", "position"])));
        assert!(is_known_config_path(&path(&["bar", "default"])));
        assert!(is_known_config_path(&path(&[
            "bar",
            "default",
            "monitor",
            "DP-1",
            "thickness"
        ])));
        assert!(is_known_config_path(&path(&[
            "bar",
            "default",
            "monitor",
            "DP-1",
            "concave_edge_corners"
        ])));

        // Unknown / typos
        assert!(!is_known_config_path(&path(&["shell", "ui_scl"])));
        assert!(!is_known_config_path(&path(&[
            "shell",
            "panel",
            "control_center_palcement"
        ])));
        assert!(!is_known_config_path(&path(&["accessibilit", "ui_scale"])));
        assert!(!is_known_config_path(&path(&["shell"])));
        assert!(!is_known_config_path(&path(&["dock", "radius_top_typo"])));
        assert!(!is_known_config_path(&path(&[
            "desktop_widgets",
            "enabeld"
        ])));
        assert!(!is_known_config_path(&path(&[
            "desktop_widgets",
            "grid",
            "cell_szie"
        ])));
        assert!(!is_known_config_path(&path(&[
            "desktop_widgets",
            "widget",
            "clock1",
            "bogus"
        ])));
        assert!(!is_known_config_path(&path(&[
            "bar",
            "default",
            "thicknesss"
        ])));
        assert!(!is_known_config_path(&path(&[
            "bar", "default", "monitor", "DP-1", "bogus"
        ])));
        assert!(!is_known_config_path(&[]));
    }
}
