//! Section registry for config sections.
//! Port of `src/config/schema/config_sections.{h,cpp}`.

use std::collections::HashSet;
use std::sync::LazyLock;

use super::config_schema::*;
use super::diagnostics::Diagnostics;
use super::engine::*;
use crate::types::config::Config;

pub type SectionReadFn = Box<dyn Fn(&toml::Table, &mut Config, &mut Diagnostics) + Send + Sync>;
pub type SectionWriteFn = Box<dyn Fn(&Config) -> toml::Table + Send + Sync>;
pub type SectionCollectUnknownFn = Box<dyn Fn(&toml::Table, &mut Vec<String>) + Send + Sync>;
pub type SectionCheckDefaultsFn = Box<dyn Fn(&toml::Table, &mut Diagnostics) + Send + Sync>;
pub type SectionEqualFn = Box<dyn Fn(&Config, &Config) -> bool + Send + Sync>;

/// One row per schema-backed section.
/// Port of `SectionSpec` (config_sections.h:23-43).
pub struct SectionSpec {
    pub name: &'static str,
    pub read: SectionReadFn,
    pub write: SectionWriteFn,
    pub collect_unknown: SectionCollectUnknownFn,
    pub check_against_defaults: SectionCheckDefaultsFn,
    pub section_equal: SectionEqualFn,
    pub allow_unknown_paths: HashSet<&'static str>,
}

fn make_section<T: Default + Clone + PartialEq + Send + Sync + 'static>(
    name: &'static str,
    get: fn(&Config) -> &T,
    set: fn(&mut Config, T),
    schema: &'static super::field::Schema<T>,
    allow_unknown_paths: HashSet<&'static str>,
) -> SectionSpec {
    SectionSpec {
        name,
        read: Box::new(move |tbl, out, diag| {
            let mut candidate = get(out).clone();
            read_into(tbl, &mut candidate, schema, name, diag);
            set(out, candidate);
        }),
        write: Box::new(move |in_| write_table(get(in_), schema)),
        collect_unknown: Box::new(move |tbl, unknown| {
            collect_unknown_keys(tbl, schema, name, unknown);
        }),
        check_against_defaults: Box::new(move |tbl, diag| {
            let mut defaults = T::default();
            read_into(tbl, &mut defaults, schema, name, diag);
        }),
        section_equal: Box::new(move |a, b| get(a) == get(b)),
        allow_unknown_paths,
    }
}

static SECTION_TABLE: LazyLock<Vec<SectionSpec>> = LazyLock::new(|| {
    let mut t = Vec::new();

    t.push(make_section(
        "storage",
        |c| &c.storage,
        |c, v| c.storage = v,
        storage_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "shell",
        |c| &c.shell,
        |c, v| c.shell = v,
        shell_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "accessibility",
        |c| &c.accessibility,
        |c, v| c.accessibility = v,
        accessibility_schema(),
        HashSet::new(),
    ));

    let wallpaper_unknown: HashSet<&'static str> = [
        "wallpaper.default",
        "wallpaper.last",
        "wallpaper.monitors",
        "wallpaper.favorite",
    ]
    .into_iter()
    .collect();
    t.push(make_section(
        "wallpaper",
        |c| &c.wallpaper,
        |c, v| c.wallpaper = v,
        wallpaper_schema(),
        wallpaper_unknown,
    ));

    t.push(make_section(
        "theme",
        |c| &c.theme,
        |c, v| c.theme = v,
        theme_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "backdrop",
        |c| &c.backdrop,
        |c, v| c.backdrop = v,
        backdrop_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "lockscreen",
        |c| &c.lockscreen,
        |c, v| c.lockscreen = v,
        lockscreen_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "notification",
        |c| &c.notification,
        |c, v| c.notification = v,
        notification_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "osd",
        |c| &c.osd,
        |c, v| c.osd = v,
        osd_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "system",
        |c| &c.system,
        |c, v| c.system = v,
        system_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "weather",
        |c| &c.weather,
        |c, v| c.weather = v,
        weather_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "calendar",
        |c| &c.calendar,
        |c, v| c.calendar = v,
        calendar_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "audio",
        |c| &c.audio,
        |c, v| c.audio = v,
        audio_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "brightness",
        |c| &c.brightness,
        |c, v| c.brightness = v,
        brightness_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "battery",
        |c| &c.battery,
        |c, v| c.battery = v,
        battery_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "nightlight",
        |c| &c.nightlight,
        |c, v| c.nightlight = v,
        nightlight_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "location",
        |c| &c.location,
        |c, v| c.location = v,
        location_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "idle",
        |c| &c.idle,
        |c, v| c.idle = v,
        idle_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "keybinds",
        |c| &c.keybinds,
        |c, v| c.keybinds = v,
        keybinds_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "dock",
        |c| &c.dock,
        |c, v| c.dock = v,
        dock_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "hot_corners",
        |c| &c.hot_corners,
        |c, v| c.hot_corners = v,
        hot_corners_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "control_center",
        |c| &c.control_center,
        |c, v| c.control_center = v,
        control_center_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "plugins",
        |c| &c.plugins,
        |c, v| c.plugins = v,
        plugins_schema(),
        HashSet::new(),
    ));
    t.push(make_section(
        "hooks",
        |c| &c.hooks,
        |c, v| c.hooks = v,
        hooks_schema(),
        HashSet::new(),
    ));

    t
});

/// Port of `sections()` (config_sections.h:45).
pub fn sections() -> &'static [SectionSpec] {
    &SECTION_TABLE
}

/// Port of `findSection` (config_sections.h:47).
pub fn find_section(name: &str) -> Option<&'static SectionSpec> {
    SECTION_TABLE.iter().find(|s| s.name == name)
}

/// Port of `customRootKeys()` (config_sections.h:54).
pub const CUSTOM_ROOT_KEYS: &[&str] = &[
    "bar",
    "widget",
    "desktop_widgets",
    "lockscreen_widgets",
    "plugin_settings",
    "include",
    "config_version",
    "config",
];

pub fn custom_root_keys() -> &'static [&'static str] {
    CUSTOM_ROOT_KEYS
}

/// Port of `isKnownRootKey` (config_sections.h:56).
pub fn is_known_root_key(name: &str) -> bool {
    find_section(name).is_some() || CUSTOM_ROOT_KEYS.contains(&name)
}
