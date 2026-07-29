//! Config migrations and legacy config normalization.
//! Port of `src/config/config_migrations.{cpp,h}`.

use std::collections::BTreeSet;

use crate::schema::diagnostics::Diagnostics;

pub const K_CONFIG_VERSION_KEY: &str = "config_version";
pub const K_LEGACY_CONFIG_REMINDER_INTERVAL_SECONDS: i64 = 3 * 24 * 60 * 60;

const K_NEGATIVE_BAR_RADIUS_MIGRATION_VERSION: i32 = 1;
const K_CUSTOM_SCHEDULE_MIGRATION_VERSION: i32 = 2;
const K_WIDGET_ACTIONS_MIGRATION_VERSION: i32 = 3;
const K_WIDGET_GESTURE_SETTINGS_MIGRATION_VERSION: i32 = 4;
const K_REMAINING_WIDGET_GESTURES_MIGRATION_VERSION: i32 = 5;
const K_CUSTOM_BUTTON_COMMANDS_MIGRATION_VERSION: i32 = 6;
const K_DEAD_ZONE_ACTIONS_MIGRATION_VERSION: i32 = 7;
const K_LOCKSCREEN_LOGIN_BOX_DEPRECATED_SETTINGS_MIGRATION_VERSION: i32 = 8;

const K_MAX_BAR_RADIUS: i64 = 500;
const K_BAR_RADIUS_KEYS: [&str; 5] = [
    "radius",
    "radius_top_left",
    "radius_top_right",
    "radius_bottom_left",
    "radius_bottom_right",
];

pub struct ConfigMigration {
    pub to_version: i32,
    pub summary: &'static str,
    pub apply: fn(&mut toml::Table, &mut Diagnostics),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyConfigIssue {
    pub migration_version: i32,
    pub path: String,
    pub message: String,
}

fn migrate_negative_radii(table: &mut toml::Table) -> bool {
    let mut changed = false;
    for key in K_BAR_RADIUS_KEYS {
        let radius = table.get(key).and_then(|v| v.as_integer());
        let Some(r) = radius else {
            continue;
        };
        if r >= 0 {
            continue;
        }

        let magnitude = if r <= -K_MAX_BAR_RADIUS {
            K_MAX_BAR_RADIUS
        } else {
            -r
        };
        table.insert(key.to_string(), toml::Value::Integer(magnitude));
        changed = true;
    }

    if changed {
        table.insert(
            "concave_edge_corners".to_string(),
            toml::Value::Boolean(true),
        );
    }
    changed
}

fn migrate_negative_bar_radii<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str),
{
    let Some(bars) = root.get_mut("bar").and_then(|v| v.as_table_mut()) else {
        return;
    };

    let bar_names: Vec<String> = bars.keys().cloned().collect();
    for bar_name in bar_names {
        let Some(bar) = bars.get_mut(&bar_name).and_then(|v| v.as_table_mut()) else {
            continue;
        };

        let bar_path = format!("bar.{bar_name}");
        if migrate_negative_radii(bar) {
            on_changed(&bar_path);
        }

        let monitor_names: Vec<String> = bar
            .get("monitor")
            .and_then(|v| v.as_table())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();

        if let Some(monitors) = bar.get_mut("monitor").and_then(|v| v.as_table_mut()) {
            for monitor_name in monitor_names {
                if let Some(monitor) = monitors
                    .get_mut(&monitor_name)
                    .and_then(|v| v.as_table_mut())
                    && migrate_negative_radii(monitor)
                {
                    on_changed(&format!("{bar_path}.monitor.{monitor_name}"));
                }
            }
        }
    }
}

fn migrate_negative_bar_radii_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_negative_bar_radii(root, |path| {
        diag.warn(
            path,
            "migrated negative corner radii to concave_edge_corners",
        );
    });
}

fn migrate_custom_schedule<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str),
{
    let Some(location) = root.get_mut("location").and_then(|v| v.as_table_mut()) else {
        return;
    };

    if location.contains_key("custom_schedule") {
        return;
    }

    let sunset = location.get("sunset").and_then(|v| v.as_str());
    let sunrise = location.get("sunrise").and_then(|v| v.as_str());
    let (Some(sunset), Some(sunrise)) = (sunset, sunrise) else {
        return;
    };

    if sunset.is_empty() || sunrise.is_empty() {
        return;
    }

    let auto_locate = location
        .get("auto_locate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_address = !location
        .get("address")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty();
    let has_coordinates = location
        .get("latitude")
        .is_some_and(|v| v.is_integer() || v.is_float())
        && location
            .get("longitude")
            .is_some_and(|v| v.is_integer() || v.is_float());

    if auto_locate || has_address || has_coordinates {
        return;
    }

    location.insert("custom_schedule".to_string(), toml::Value::Boolean(true));
    on_changed("location");
}

fn migrate_custom_schedule_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_custom_schedule(root, |path| {
        diag.warn(
            path,
            "sunset/sunrise now require custom_schedule; enabled it to keep the schedule",
        );
    });
}

fn migrate_widget_actions<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str),
{
    let Some(shell) = root.get_mut("shell").and_then(|v| v.as_table_mut()) else {
        return;
    };

    if !shell.contains_key("middle_click_opens_widget_settings") {
        return;
    }

    let was_enabled = shell
        .get("middle_click_opens_widget_settings")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    shell.remove("middle_click_opens_widget_settings");
    on_changed("shell");

    if was_enabled {
        return;
    }

    if !root.contains_key("bar") {
        root.insert("bar".to_string(), toml::Value::Table(toml::Table::new()));
    }

    let Some(bars) = root.get_mut("bar").and_then(|v| v.as_table_mut()) else {
        return;
    };

    if bars.is_empty() {
        bars.insert(
            "default".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
    }

    let bar_names: Vec<String> = bars.keys().cloned().collect();
    for bar_name in bar_names {
        let Some(bar) = bars.get_mut(&bar_name).and_then(|v| v.as_table_mut()) else {
            continue;
        };

        if !bar.contains_key("actions") {
            bar.insert(
                "actions".to_string(),
                toml::Value::Table(toml::Table::new()),
            );
        }
        let Some(actions) = bar.get_mut("actions").and_then(|v| v.as_table_mut()) else {
            continue;
        };

        if actions.contains_key("middle") {
            continue;
        }
        actions.insert(
            "middle".to_string(),
            toml::Value::String("none".to_string()),
        );
        on_changed(&format!("bar.{bar_name}.actions"));
    }
}

fn bind_action(widget: &mut toml::Table, gesture: &str, action: &str) {
    if !widget.contains_key("actions") {
        widget.insert(
            "actions".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
    }
    if let Some(actions) = widget.get_mut("actions").and_then(|v| v.as_table_mut())
        && !actions.contains_key(gesture)
    {
        actions.insert(gesture.to_string(), toml::Value::String(action.to_string()));
    }
}

fn migrate_widget_gesture_settings<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str, &str),
{
    const K_SCROLL_TYPES: [&str; 3] = ["workspaces", "taskbar", "power_profile"];

    let Some(widgets) = root.get_mut("widget").and_then(|v| v.as_table_mut()) else {
        return;
    };

    let widget_names: Vec<String> = widgets.keys().cloned().collect();
    for widget_name in widget_names {
        let Some(widget) = widgets.get_mut(&widget_name).and_then(|v| v.as_table_mut()) else {
            continue;
        };

        let widget_type = widget
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(&widget_name)
            .to_string();
        let path = format!("widget.{widget_name}");

        if K_SCROLL_TYPES.contains(&widget_type.as_str()) && widget.contains_key("enable_scroll") {
            let was_enabled = widget
                .get("enable_scroll")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            widget.remove("enable_scroll");
            if !was_enabled {
                bind_action(widget, "scroll_up", "none");
                bind_action(widget, "scroll_down", "none");
            }
            on_changed(
                &path,
                "enable_scroll is now the scroll_up/scroll_down gesture bindings",
            );
        }

        if widget_type == "keyboard_layout" && widget.contains_key("cycle_command") {
            let command = widget
                .get("cycle_command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            widget.remove("cycle_command");
            if !command.is_empty() {
                bind_action(widget, "left", &format!("exec {command}"));
            }
            on_changed(&path, "cycle_command is now the left gesture binding");
        }
    }
}

fn migrate_remaining_widget_gestures<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str, &str),
{
    const K_DEFAULT_SCROLL_STEP: i64 = 5;

    let Some(widgets) = root.get_mut("widget").and_then(|v| v.as_table_mut()) else {
        return;
    };

    let widget_names: Vec<String> = widgets.keys().cloned().collect();
    for widget_name in widget_names {
        let Some(widget) = widgets.get_mut(&widget_name).and_then(|v| v.as_table_mut()) else {
            continue;
        };

        let widget_type = widget
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(&widget_name)
            .to_string();
        let path = format!("widget.{widget_name}");
        let scroll_type =
            widget_type == "media" || widget_type == "volume" || widget_type == "brightness";

        if scroll_type && widget.contains_key("enable_scroll") {
            let was_enabled = widget
                .get("enable_scroll")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            widget.remove("enable_scroll");
            if !was_enabled {
                bind_action(widget, "scroll_up", "none");
                bind_action(widget, "scroll_down", "none");
            }
            on_changed(
                &path,
                "enable_scroll is now the scroll_up/scroll_down gesture bindings",
            );
        }

        if (widget_type == "volume" || widget_type == "brightness")
            && widget.contains_key("scroll_step")
        {
            let step = widget
                .get("scroll_step")
                .and_then(|v| v.as_integer())
                .unwrap_or(K_DEFAULT_SCROLL_STEP);
            widget.remove("scroll_step");
            if step != K_DEFAULT_SCROLL_STEP {
                let suffix = format!(" {step}%");
                let mut up_verb = "brightness-up".to_string();
                let mut down_verb = "brightness-down".to_string();
                if widget_type == "volume" {
                    let microphone = widget
                        .get("device")
                        .and_then(|v| v.as_str())
                        .unwrap_or("output")
                        == "input";
                    up_verb = if microphone {
                        "mic-volume-up"
                    } else {
                        "volume-up"
                    }
                    .to_string();
                    down_verb = if microphone {
                        "mic-volume-down"
                    } else {
                        "volume-down"
                    }
                    .to_string();
                }
                bind_action(widget, "scroll_up", &format!("{up_verb}{suffix}"));
                bind_action(widget, "scroll_down", &format!("{down_verb}{suffix}"));
            }
            on_changed(
                &path,
                "scroll_step is now the step argument of the scroll gesture bindings",
            );
        }

        if widget_type == "screenshot" && widget.contains_key("primary_click") {
            let primary = widget
                .get("primary_click")
                .and_then(|v| v.as_str())
                .unwrap_or("region")
                .to_string();
            widget.remove("primary_click");
            if primary == "fullscreen" {
                bind_action(widget, "left", "screenshot-fullscreen");
            }
            on_changed(&path, "primary_click is now the left gesture binding");
        }
    }
}

fn migrate_custom_button_commands<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str, &str),
{
    const K_COMMAND_KEYS: [(&str, &str); 5] = [
        ("command", "left"),
        ("right_command", "right"),
        ("middle_command", "middle"),
        ("scroll_up_command", "scroll_up"),
        ("scroll_down_command", "scroll_down"),
    ];

    let Some(widgets) = root.get_mut("widget").and_then(|v| v.as_table_mut()) else {
        return;
    };

    let widget_names: Vec<String> = widgets.keys().cloned().collect();
    for widget_name in widget_names {
        let Some(widget) = widgets.get_mut(&widget_name).and_then(|v| v.as_table_mut()) else {
            continue;
        };
        let widget_type = widget
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(&widget_name);
        if widget_type != "custom_button" {
            continue;
        }

        let path = format!("widget.{widget_name}");
        if widget.contains_key("enable_scroll") {
            let was_enabled = widget
                .get("enable_scroll")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            widget.remove("enable_scroll");
            if !was_enabled {
                bind_action(widget, "scroll_up", "none");
                bind_action(widget, "scroll_down", "none");
            }
            on_changed(
                &path,
                "enable_scroll is now the scroll_up/scroll_down gesture bindings",
            );
        }

        for (key, gesture) in K_COMMAND_KEYS {
            if !widget.contains_key(key) {
                continue;
            }
            let command = widget
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            widget.remove(key);
            if !command.is_empty() {
                bind_action(widget, gesture, &format!("exec {command}"));
            }
            on_changed(
                &path,
                &format!("{key} is now the {gesture} gesture binding"),
            );
        }
    }
}

fn migrate_dead_zone_actions<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str, &str),
{
    const K_COMMAND_KEYS: [(&str, &str); 5] = [
        ("command", "left"),
        ("right_command", "right"),
        ("middle_command", "middle"),
        ("scroll_up_command", "scroll_up"),
        ("scroll_down_command", "scroll_down"),
    ];

    let migrate_table = |owner: &mut toml::Table, path: &str, on_changed: &mut F| {
        let Some(dead_zone) = owner.get_mut("dead_zone").and_then(|v| v.as_table_mut()) else {
            return;
        };
        for (key, gesture) in K_COMMAND_KEYS {
            if !dead_zone.contains_key(key) {
                continue;
            }
            let command = dead_zone
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            dead_zone.remove(key);
            if !command.is_empty() {
                bind_action(dead_zone, gesture, &format!("exec {command}"));
            }
            on_changed(
                &format!("{path}.dead_zone"),
                &format!("{key} is now the {gesture} binding"),
            );
        }
    };

    let Some(bars) = root.get_mut("bar").and_then(|v| v.as_table_mut()) else {
        return;
    };

    let bar_names: Vec<String> = bars.keys().cloned().collect();
    for bar_name in bar_names {
        let Some(bar) = bars.get_mut(&bar_name).and_then(|v| v.as_table_mut()) else {
            continue;
        };
        let bar_path = format!("bar.{bar_name}");
        migrate_table(bar, &bar_path, &mut on_changed);

        let monitor_names: Vec<String> = bar
            .get("monitor")
            .and_then(|v| v.as_table())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();

        if let Some(monitors) = bar.get_mut("monitor").and_then(|v| v.as_table_mut()) {
            for monitor_name in monitor_names {
                if let Some(monitor) = monitors
                    .get_mut(&monitor_name)
                    .and_then(|v| v.as_table_mut())
                {
                    migrate_table(
                        monitor,
                        &format!("{bar_path}.monitor.{monitor_name}"),
                        &mut on_changed,
                    );
                }
            }
        }
    }
}

fn migrate_lockscreen_login_box_deprecated_settings<F>(root: &mut toml::Table, mut on_changed: F)
where
    F: FnMut(&str),
{
    let Some(section) = root
        .get_mut("lockscreen_widgets")
        .and_then(|v| v.as_table_mut())
    else {
        return;
    };
    let Some(widgets) = section.get_mut("widget").and_then(|v| v.as_table_mut()) else {
        return;
    };

    let widget_ids: Vec<String> = widgets.keys().cloned().collect();
    for widget_id in widget_ids {
        let Some(widget) = widgets.get_mut(&widget_id).and_then(|v| v.as_table_mut()) else {
            continue;
        };

        let widget_type = widget.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if widget_type != "login_box" && !widget_id.starts_with("lockscreen-login-box@") {
            continue;
        }

        let Some(settings) = widget.get_mut("settings").and_then(|v| v.as_table_mut()) else {
            continue;
        };

        if settings.remove("show_password_hint").is_some() {
            on_changed(&format!("lockscreen_widgets.widget.{widget_id}.settings"));
        }
    }
}

fn migrate_dead_zone_actions_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_dead_zone_actions(root, |path, message| {
        diag.warn(path, message);
    });
}

fn migrate_lockscreen_login_box_deprecated_settings_sidecar(
    root: &mut toml::Table,
    diag: &mut Diagnostics,
) {
    migrate_lockscreen_login_box_deprecated_settings(root, |path| {
        diag.warn(path, "removed deprecated show_password_hint");
    });
}

fn migrate_custom_button_commands_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_custom_button_commands(root, |path, message| {
        diag.warn(path, message);
    });
}

fn migrate_remaining_widget_gestures_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_remaining_widget_gestures(root, |path, message| {
        diag.warn(path, message);
    });
}

fn migrate_widget_gesture_settings_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_widget_gesture_settings(root, |path, message| {
        diag.warn(path, message);
    });
}

fn migrate_widget_actions_sidecar(root: &mut toml::Table, diag: &mut Diagnostics) {
    migrate_widget_actions(root, |path| {
        diag.warn(
            path,
            "middle_click_opens_widget_settings is now the `middle` widget gesture binding",
        );
    });
}

fn stable_issue_hash(migration_version: i32, path: &str) -> u64 {
    const K_OFFSET: u64 = 14695981039346656037;
    const K_PRIME: u64 = 1099511628211;
    let mut hash = K_OFFSET;
    let mut append = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(K_PRIME);
        }
    };
    append(migration_version.to_string().as_bytes());
    append(b":");
    append(path.as_bytes());
    hash
}

fn split_fingerprint(fingerprint: &str) -> BTreeSet<&str> {
    fingerprint.split(',').filter(|s| !s.is_empty()).collect()
}

pub fn config_migrations() -> &'static [ConfigMigration] {
    static MIGRATIONS: [ConfigMigration; 8] = [
        ConfigMigration {
            to_version: K_NEGATIVE_BAR_RADIUS_MIGRATION_VERSION,
            summary: "bar: migrate negative corner radii",
            apply: migrate_negative_bar_radii_sidecar,
        },
        ConfigMigration {
            to_version: K_CUSTOM_SCHEDULE_MIGRATION_VERSION,
            summary: "location: opt legacy sunset/sunrise schedules into custom_schedule",
            apply: migrate_custom_schedule_sidecar,
        },
        ConfigMigration {
            to_version: K_WIDGET_ACTIONS_MIGRATION_VERSION,
            summary: "bar: move middle_click_opens_widget_settings to widget gesture actions",
            apply: migrate_widget_actions_sidecar,
        },
        ConfigMigration {
            to_version: K_WIDGET_GESTURE_SETTINGS_MIGRATION_VERSION,
            summary: "widget: move enable_scroll and cycle_command to gesture actions",
            apply: migrate_widget_gesture_settings_sidecar,
        },
        ConfigMigration {
            to_version: K_REMAINING_WIDGET_GESTURES_MIGRATION_VERSION,
            summary: "widget: move scroll_step and primary_click to gesture actions",
            apply: migrate_remaining_widget_gestures_sidecar,
        },
        ConfigMigration {
            to_version: K_CUSTOM_BUTTON_COMMANDS_MIGRATION_VERSION,
            summary: "widget: move custom_button commands to gesture actions",
            apply: migrate_custom_button_commands_sidecar,
        },
        ConfigMigration {
            to_version: K_DEAD_ZONE_ACTIONS_MIGRATION_VERSION,
            summary: "bar: move dead zone commands to gesture actions",
            apply: migrate_dead_zone_actions_sidecar,
        },
        ConfigMigration {
            to_version: K_LOCKSCREEN_LOGIN_BOX_DEPRECATED_SETTINGS_MIGRATION_VERSION,
            summary: "lockscreen: drop removed login box show_password_hint setting",
            apply: migrate_lockscreen_login_box_deprecated_settings_sidecar,
        },
    ];
    &MIGRATIONS
}

pub fn current_config_version() -> i32 {
    let migrations = config_migrations();
    migrations.last().map_or(0, |m| m.to_version)
}

pub fn stored_config_version(root: &toml::Table, diag: &mut Diagnostics) -> Option<i32> {
    let node = root.get(K_CONFIG_VERSION_KEY);
    let Some(node) = node else {
        return Some(0);
    };

    let value = node.as_integer();
    let Some(value) = value else {
        diag.fatal_with_code(
            K_CONFIG_VERSION_KEY,
            "expected a non-negative integer",
            "config.version.invalid",
        );
        return None;
    };

    if value < 0 || value > i64::from(i32::MAX) {
        diag.fatal_with_code(
            K_CONFIG_VERSION_KEY,
            "expected a non-negative integer",
            "config.version.invalid",
        );
        return None;
    }

    let version = value as i32;
    if version > current_config_version() {
        diag.fatal_with_code(
            K_CONFIG_VERSION_KEY,
            format!(
                "version {} is newer than supported version {}",
                version,
                current_config_version()
            ),
            "config.version.unsupported",
        );
        return None;
    }
    Some(version)
}

pub fn apply_pending_config_migrations(
    root: &mut toml::Table,
    stored_version: i32,
    diag: &mut Diagnostics,
    migrations: &[ConfigMigration],
) -> i32 {
    let mut applied_version = stored_version;
    for migration in migrations {
        if migration.to_version <= stored_version {
            continue;
        }
        (migration.apply)(root, diag);
        applied_version = migration.to_version;
    }
    applied_version
}

pub fn normalize_legacy_config(root: &mut toml::Table, issues: &mut Vec<LegacyConfigIssue>) {
    migrate_negative_bar_radii(root, |path| {
        issues.push(LegacyConfigIssue {
            migration_version: K_NEGATIVE_BAR_RADIUS_MIGRATION_VERSION,
            path: path.to_string(),
            message: "negative corner radii are deprecated; use positive radii and concave_edge_corners = true".to_string(),
        });
    });

    migrate_custom_schedule(root, |path| {
        issues.push(LegacyConfigIssue {
            migration_version: K_CUSTOM_SCHEDULE_MIGRATION_VERSION,
            path: path.to_string(),
            message: "sunset/sunrise no longer schedule on their own; set custom_schedule = true"
                .to_string(),
        });
    });

    migrate_widget_actions(root, |path| {
        issues.push(LegacyConfigIssue {
            migration_version: K_WIDGET_ACTIONS_MIGRATION_VERSION,
            path: path.to_string(),
            message:
                "middle_click_opens_widget_settings is now the `middle` bar widget gesture binding"
                    .to_string(),
        });
    });

    migrate_widget_gesture_settings(root, |path, message| {
        issues.push(LegacyConfigIssue {
            migration_version: K_WIDGET_GESTURE_SETTINGS_MIGRATION_VERSION,
            path: path.to_string(),
            message: message.to_string(),
        });
    });

    migrate_remaining_widget_gestures(root, |path, message| {
        issues.push(LegacyConfigIssue {
            migration_version: K_REMAINING_WIDGET_GESTURES_MIGRATION_VERSION,
            path: path.to_string(),
            message: message.to_string(),
        });
    });

    migrate_custom_button_commands(root, |path, message| {
        issues.push(LegacyConfigIssue {
            migration_version: K_CUSTOM_BUTTON_COMMANDS_MIGRATION_VERSION,
            path: path.to_string(),
            message: message.to_string(),
        });
    });

    migrate_dead_zone_actions(root, |path, message| {
        issues.push(LegacyConfigIssue {
            migration_version: K_DEAD_ZONE_ACTIONS_MIGRATION_VERSION,
            path: path.to_string(),
            message: message.to_string(),
        });
    });

    migrate_lockscreen_login_box_deprecated_settings(root, |path| {
        issues.push(LegacyConfigIssue {
            migration_version: K_LOCKSCREEN_LOGIN_BOX_DEPRECATED_SETTINGS_MIGRATION_VERSION,
            path: path.to_string(),
            message: "removed deprecated show_password_hint".to_string(),
        });
    });
}

pub fn legacy_config_issue_fingerprint(issues: &[LegacyConfigIssue]) -> String {
    let mut entries = BTreeSet::new();
    for issue in issues {
        entries.insert(format!(
            "{:016x}",
            stable_issue_hash(issue.migration_version, &issue.path)
        ));
    }

    let mut fingerprint = String::new();
    for entry in entries {
        if !fingerprint.is_empty() {
            fingerprint.push(',');
        }
        fingerprint.push_str(&entry);
    }
    fingerprint
}

pub fn legacy_config_fingerprint_has_new_issues(
    current_fingerprint: &str,
    previous_fingerprint: &str,
) -> bool {
    let current = split_fingerprint(current_fingerprint);
    let previous = split_fingerprint(previous_fingerprint);
    current.iter().any(|entry| !previous.contains(entry))
}

pub fn legacy_config_reminder_interval_elapsed(
    now_epoch_seconds: i64,
    previous_epoch_seconds: i64,
) -> bool {
    previous_epoch_seconds > now_epoch_seconds
        || now_epoch_seconds - previous_epoch_seconds >= K_LEGACY_CONFIG_REMINDER_INTERVAL_SECONDS
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    fn parse(s: &str) -> toml::Table {
        s.parse::<toml::Table>()
            .expect("test fixture must be valid TOML")
    }

    #[test]
    fn negative_radius_migration() {
        let mut root = parse(
            r#"
[bar.main]
radius = -12
radius_top_left = -20
radius_top_right = 8

[bar.main.monitor.dp1]
match = "DP-1"
radius = -16

[dock]
radius = -7
"#,
        );

        let mut issues = Vec::new();
        normalize_legacy_config(&mut root, &mut issues);

        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("radius"))
                .and_then(|v| v.as_integer()),
            Some(12)
        );
        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("radius_top_left"))
                .and_then(|v| v.as_integer()),
            Some(20)
        );
        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("radius_top_right"))
                .and_then(|v| v.as_integer()),
            Some(8)
        );
        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("concave_edge_corners"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("monitor"))
                .and_then(|v| v.get("dp1"))
                .and_then(|v| v.get("radius"))
                .and_then(|v| v.as_integer()),
            Some(16)
        );
        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("monitor"))
                .and_then(|v| v.get("dp1"))
                .and_then(|v| v.get("concave_edge_corners"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            root.get("dock")
                .and_then(|v| v.get("radius"))
                .and_then(|v| v.as_integer()),
            Some(-7)
        );
        assert_eq!(issues.len(), 2);

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut root, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());
    }

    #[test]
    fn extreme_negative_radius() {
        let mut root = toml::Table::new();
        let mut bar = toml::Table::new();
        bar.insert("radius".to_string(), toml::Value::Integer(i64::MIN));
        let mut bars = toml::Table::new();
        bars.insert("main".to_string(), toml::Value::Table(bar));
        root.insert("bar".to_string(), toml::Value::Table(bars));

        let mut issues = Vec::new();
        normalize_legacy_config(&mut root, &mut issues);
        assert_eq!(
            root.get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("radius"))
                .and_then(|v| v.as_integer()),
            Some(500)
        );
    }

    #[test]
    fn custom_schedule_migration() {
        let mut legacy = parse(
            r#"
[location]
sunset = "20:30"
sunrise = "07:30"
"#,
        );
        let mut issues = Vec::new();
        normalize_legacy_config(&mut legacy, &mut issues);
        assert_eq!(
            legacy
                .get("location")
                .and_then(|v| v.get("custom_schedule"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(issues.len(), 1);

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut legacy, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());

        for source in [
            "auto_locate = true",
            "address = \"Toronto, ON\"",
            "latitude = 52.52\nlongitude = 13.405",
        ] {
            let mut coords = parse(&format!(
                "[location]\nsunset = \"20:30\"\nsunrise = \"07:30\"\n{source}\n"
            ));
            let mut coord_issues = Vec::new();
            normalize_legacy_config(&mut coords, &mut coord_issues);
            assert!(
                coords
                    .get("location")
                    .and_then(|v| v.get("custom_schedule"))
                    .is_none()
            );
            assert!(coord_issues.is_empty());
        }

        let mut explicit_off = parse(
            r#"
[location]
custom_schedule = false
sunset = "20:30"
sunrise = "07:30"
"#,
        );
        let mut off_issues = Vec::new();
        normalize_legacy_config(&mut explicit_off, &mut off_issues);
        assert_eq!(
            explicit_off
                .get("location")
                .and_then(|v| v.get("custom_schedule"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn widget_actions_migration() {
        let mut enabled = parse(
            r#"
[shell]
middle_click_opens_widget_settings = true
"#,
        );
        let mut enabled_issues = Vec::new();
        normalize_legacy_config(&mut enabled, &mut enabled_issues);
        assert!(
            enabled
                .get("shell")
                .and_then(|v| v.get("middle_click_opens_widget_settings"))
                .is_none()
        );
        assert!(enabled.get("bar").is_none());

        let mut disabled = parse(
            r#"
[shell]
middle_click_opens_widget_settings = false

[bar.default]
position = "top"

[bar.secondary]
position = "bottom"
"#,
        );
        let mut disabled_issues = Vec::new();
        normalize_legacy_config(&mut disabled, &mut disabled_issues);
        assert!(
            disabled
                .get("shell")
                .and_then(|v| v.get("middle_click_opens_widget_settings"))
                .is_none()
        );
        for bar_name in ["default", "secondary"] {
            assert_eq!(
                disabled
                    .get("bar")
                    .and_then(|v| v.get(bar_name))
                    .and_then(|v| v.get("actions"))
                    .and_then(|v| v.get("middle"))
                    .and_then(|v| v.as_str()),
                Some("none")
            );
        }

        let mut no_bars = parse(
            r#"
[shell]
middle_click_opens_widget_settings = false
"#,
        );
        let mut no_bar_issues = Vec::new();
        normalize_legacy_config(&mut no_bars, &mut no_bar_issues);
        assert_eq!(
            no_bars
                .get("bar")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("middle"))
                .and_then(|v| v.as_str()),
            Some("none")
        );

        let mut explicit_binding = parse(
            r#"
[shell]
middle_click_opens_widget_settings = false

[bar.default.actions]
middle = "media toggle"
"#,
        );
        let mut explicit_issues = Vec::new();
        normalize_legacy_config(&mut explicit_binding, &mut explicit_issues);
        assert_eq!(
            explicit_binding
                .get("bar")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("middle"))
                .and_then(|v| v.as_str()),
            Some("media toggle")
        );

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut disabled, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());
    }

    #[test]
    fn widget_gesture_settings_migration() {
        let mut config = parse(
            r#"
[widget.workspaces]
enable_scroll = false

[widget.taskbar]
enable_scroll = true

[widget.my_profile]
type = "power_profile"
enable_scroll = false

[widget.keyboard_layout]
cycle_command = "hyprctl switchxkblayout all next"

[widget.plugin_thing]
type = "someone/plugin:entry"
enable_scroll = false
"#,
        );
        let mut issues = Vec::new();
        normalize_legacy_config(&mut config, &mut issues);

        for gesture in ["scroll_up", "scroll_down"] {
            assert_eq!(
                config
                    .get("widget")
                    .and_then(|v| v.get("workspaces"))
                    .and_then(|v| v.get("actions"))
                    .and_then(|v| v.get(gesture))
                    .and_then(|v| v.as_str()),
                Some("none")
            );
        }
        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("workspaces"))
                .and_then(|v| v.get("enable_scroll"))
                .is_none()
        );

        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("taskbar"))
                .and_then(|v| v.get("enable_scroll"))
                .is_none()
        );
        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("taskbar"))
                .and_then(|v| v.get("actions"))
                .is_none()
        );

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("my_profile"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("scroll_up"))
                .and_then(|v| v.as_str()),
            Some("none")
        );

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("keyboard_layout"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("left"))
                .and_then(|v| v.as_str()),
            Some("exec hyprctl switchxkblayout all next")
        );
        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("keyboard_layout"))
                .and_then(|v| v.get("cycle_command"))
                .is_none()
        );

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("plugin_thing"))
                .and_then(|v| v.get("enable_scroll"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut config, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());
    }

    #[test]
    fn remaining_widget_gestures_migration() {
        let mut config = parse(
            r#"
[widget.media]
enable_scroll = false

[widget.volume]
scroll_step = 10

[widget.mic]
type = "volume"
device = "input"
scroll_step = 2

[widget.brightness]
enable_scroll = true
scroll_step = 5

[widget.screenshot]
primary_click = "fullscreen"

[widget.shot2]
type = "screenshot"
primary_click = "region"
"#,
        );
        let mut issues = Vec::new();
        normalize_legacy_config(&mut config, &mut issues);

        for gesture in ["scroll_up", "scroll_down"] {
            assert_eq!(
                config
                    .get("widget")
                    .and_then(|v| v.get("media"))
                    .and_then(|v| v.get("actions"))
                    .and_then(|v| v.get(gesture))
                    .and_then(|v| v.as_str()),
                Some("none")
            );
        }

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("volume"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("scroll_up"))
                .and_then(|v| v.as_str()),
            Some("volume-up 10%")
        );
        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("mic"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("scroll_down"))
                .and_then(|v| v.as_str()),
            Some("mic-volume-down 2%")
        );

        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("brightness"))
                .and_then(|v| v.get("scroll_step"))
                .is_none()
        );
        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("brightness"))
                .and_then(|v| v.get("actions"))
                .is_none()
        );

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("screenshot"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("left"))
                .and_then(|v| v.as_str()),
            Some("screenshot-fullscreen")
        );
        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("shot2"))
                .and_then(|v| v.get("actions"))
                .is_none()
        );
        assert!(
            config
                .get("widget")
                .and_then(|v| v.get("screenshot"))
                .and_then(|v| v.get("primary_click"))
                .is_none()
        );

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut config, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());
    }

    #[test]
    fn custom_button_commands_migration() {
        let mut config = parse(
            r#"
[widget.custom_button]
command = "notify-send 'hello world'"
right_command = "playerctl next"
scroll_up_command = "brightnessctl set +5%"

[widget.dead_scroll]
type = "custom_button"
enable_scroll = false
scroll_up_command = "echo up"

[widget.explicit]
type = "custom_button"
command = "echo old"

[widget.explicit.actions]
left = "media toggle"
"#,
        );
        let mut issues = Vec::new();
        normalize_legacy_config(&mut config, &mut issues);

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("custom_button"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("left"))
                .and_then(|v| v.as_str()),
            Some("exec notify-send 'hello world'")
        );
        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("custom_button"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("right"))
                .and_then(|v| v.as_str()),
            Some("exec playerctl next")
        );
        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("custom_button"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("scroll_up"))
                .and_then(|v| v.as_str()),
            Some("exec brightnessctl set +5%")
        );

        for key in ["command", "right_command", "scroll_up_command"] {
            assert!(
                config
                    .get("widget")
                    .and_then(|v| v.get("custom_button"))
                    .and_then(|v| v.get(key))
                    .is_none()
            );
        }

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("dead_scroll"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("scroll_up"))
                .and_then(|v| v.as_str()),
            Some("none")
        );

        assert_eq!(
            config
                .get("widget")
                .and_then(|v| v.get("explicit"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("left"))
                .and_then(|v| v.as_str()),
            Some("media toggle")
        );

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut config, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());
    }

    #[test]
    fn dead_zone_actions_migration() {
        let mut config = parse(
            r#"
[bar.default.dead_zone]
command = "notify-send left"
right_command = "notify-send right"

[bar.default.monitor.DP-1.dead_zone]
scroll_up_command = "notify-send up"

[bar.other.dead_zone]
middle_command = ""
"#,
        );
        let mut issues = Vec::new();
        normalize_legacy_config(&mut config, &mut issues);

        assert_eq!(
            config
                .get("bar")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.get("dead_zone"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("left"))
                .and_then(|v| v.as_str()),
            Some("exec notify-send left")
        );
        assert_eq!(
            config
                .get("bar")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.get("dead_zone"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("right"))
                .and_then(|v| v.as_str()),
            Some("exec notify-send right")
        );
        assert!(
            config
                .get("bar")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.get("dead_zone"))
                .and_then(|v| v.get("command"))
                .is_none()
        );

        assert_eq!(
            config
                .get("bar")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.get("monitor"))
                .and_then(|v| v.get("DP-1"))
                .and_then(|v| v.get("dead_zone"))
                .and_then(|v| v.get("actions"))
                .and_then(|v| v.get("scroll_up"))
                .and_then(|v| v.as_str()),
            Some("exec notify-send up")
        );

        assert!(
            config
                .get("bar")
                .and_then(|v| v.get("other"))
                .and_then(|v| v.get("dead_zone"))
                .and_then(|v| v.get("actions"))
                .is_none()
        );

        let mut second_pass_issues = Vec::new();
        normalize_legacy_config(&mut config, &mut second_pass_issues);
        assert!(second_pass_issues.is_empty());
    }

    #[test]
    fn version_gating() {
        let mut legacy = parse(
            r#"
[bar.main]
radius = -10
"#,
        );
        let mut diag = Diagnostics::default();
        let stored = stored_config_version(&legacy, &mut diag);
        assert_eq!(stored, Some(0));
        let applied = apply_pending_config_migrations(
            &mut legacy,
            stored.unwrap_or(0),
            &mut diag,
            config_migrations(),
        );
        assert_eq!(applied, current_config_version());
        assert_eq!(
            legacy
                .get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("radius"))
                .and_then(|v| v.as_integer()),
            Some(10)
        );

        let mut current = parse(
            r#"
config_version = 1
[bar.main]
radius = -10
"#,
        );
        let mut current_diag = Diagnostics::default();
        let current_stored = stored_config_version(&current, &mut current_diag);
        assert_eq!(current_stored, Some(1));
        apply_pending_config_migrations(
            &mut current,
            current_stored.unwrap_or(0),
            &mut current_diag,
            config_migrations(),
        );
        assert_eq!(
            current
                .get("bar")
                .and_then(|v| v.get("main"))
                .and_then(|v| v.get("radius"))
                .and_then(|v| v.as_integer()),
            Some(-10)
        );

        let invalid = parse("config_version = \"one\"");
        let mut invalid_diag = Diagnostics::default();
        assert!(stored_config_version(&invalid, &mut invalid_diag).is_none());
        assert!(invalid_diag.has_errors());
        assert!(invalid_diag.has_fatal_errors());

        let future = parse("config_version = 999");
        let mut future_diag = Diagnostics::default();
        assert!(stored_config_version(&future, &mut future_diag).is_none());
        assert!(future_diag.has_errors());
        assert!(future_diag.has_fatal_errors());

        let mut baseline = Diagnostics::default();
        baseline.component_error("widget.clock.timezone", "widget.clock", "unknown timezone");
        let mut candidate = baseline.clone();
        candidate.error("accessibility.ui_scale", "expected a number");
        let introduced = candidate.introduced_errors_compared_to(&baseline);
        assert_eq!(introduced.entries.len(), 1);
        assert_eq!(introduced.entries[0].path, "accessibility.ui_scale");
    }

    #[test]
    fn reminder_fingerprint() {
        let first = vec![LegacyConfigIssue {
            migration_version: 1,
            path: "bar.main".to_string(),
            message: "message".to_string(),
        }];
        let reordered = vec![
            LegacyConfigIssue {
                migration_version: 1,
                path: "bar.second".to_string(),
                message: "message".to_string(),
            },
            LegacyConfigIssue {
                migration_version: 1,
                path: "bar.main".to_string(),
                message: "different display message".to_string(),
            },
        ];
        let same_reordered = vec![
            LegacyConfigIssue {
                migration_version: 1,
                path: "bar.main".to_string(),
                message: "message".to_string(),
            },
            LegacyConfigIssue {
                migration_version: 1,
                path: "bar.second".to_string(),
                message: "message".to_string(),
            },
        ];

        let first_fingerprint = legacy_config_issue_fingerprint(&first);
        let expanded_fingerprint = legacy_config_issue_fingerprint(&reordered);
        assert_eq!(
            expanded_fingerprint,
            legacy_config_issue_fingerprint(&same_reordered)
        );
        assert!(legacy_config_fingerprint_has_new_issues(
            &expanded_fingerprint,
            &first_fingerprint
        ));
        assert!(!legacy_config_fingerprint_has_new_issues(
            &first_fingerprint,
            &expanded_fingerprint
        ));

        const K_START: i64 = 1_000_000;
        assert!(!legacy_config_reminder_interval_elapsed(
            K_START + K_LEGACY_CONFIG_REMINDER_INTERVAL_SECONDS - 1,
            K_START
        ));
        assert!(legacy_config_reminder_interval_elapsed(
            K_START + K_LEGACY_CONFIG_REMINDER_INTERVAL_SECONDS,
            K_START
        ));
        assert!(legacy_config_reminder_interval_elapsed(
            K_START - 1,
            K_START
        ));
    }

    #[test]
    fn registry_ordering() {
        let mut expected_version = 1;
        for migration in config_migrations() {
            assert_eq!(migration.to_version, expected_version);
            assert!(!migration.summary.is_empty());
            expected_version += 1;
        }
        assert_eq!(expected_version - 1, current_config_version());
    }

    static SYNTHETIC_MIGRATION_APPLICATIONS: AtomicI32 = AtomicI32::new(0);

    fn count_synthetic_migration(_: &mut toml::Table, _: &mut Diagnostics) {
        SYNTHETIC_MIGRATION_APPLICATIONS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn large_current_registry_skips_bodies() {
        let mut migrations = Vec::new();
        for version in 1..=100 {
            migrations.push(ConfigMigration {
                to_version: version,
                summary: "synthetic migration",
                apply: count_synthetic_migration,
            });
        }

        let mut root = toml::Table::new();
        let mut diag = Diagnostics::default();
        SYNTHETIC_MIGRATION_APPLICATIONS.store(0, Ordering::SeqCst);
        let current = apply_pending_config_migrations(&mut root, 100, &mut diag, &migrations);
        assert_eq!(current, 100);
        assert_eq!(SYNTHETIC_MIGRATION_APPLICATIONS.load(Ordering::SeqCst), 0);

        let upgraded = apply_pending_config_migrations(&mut root, 99, &mut diag, &migrations);
        assert_eq!(upgraded, 100);
        assert_eq!(SYNTHETIC_MIGRATION_APPLICATIONS.load(Ordering::SeqCst), 1);
    }
}
