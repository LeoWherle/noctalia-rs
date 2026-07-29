//! Task 2.1.6 — Idle configuration types and action resolution.
//! Port of `IdleBehaviorConfig`, `IdleConfig`, `IdleActionKind`,
//! `IdleActionRequest`, `ResolvedIdleBehavior`, `normalizeIdleBehaviorAction`,
//! `resolveIdleBehaviorActions`, and `defaultIdleBehaviors` from
//! `src/config/config_types.{h,cpp}` (structs: lines 252-325; bodies:
//! `config_types.cpp:119-146, 199-242`).

use serde::{Deserialize, Serialize};

/// Port of `IdleBehaviorConfig` (config_types.h:252-264).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdleBehaviorConfig {
    /// Table key in `[idle.behavior.<name>]`.
    #[serde(skip)]
    pub name: String,
    pub enabled: bool,
    pub timeout_seconds: f64,
    pub action: String,
    pub command: String,
    pub resume_command: String,
    pub lock_before_suspend: bool,
}

impl Default for IdleBehaviorConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            timeout_seconds: 0.0,
            action: String::new(),
            command: String::new(),
            resume_command: String::new(),
            lock_before_suspend: true,
        }
    }
}

/// Port of `IdleConfig` (config_types.h:284-292).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdleConfig {
    /// Populated from `[idle.behavior.<name>]` map during whole-config assembly in task 2.3.
    #[serde(skip)]
    pub behaviors: Vec<IdleBehaviorConfig>,
    pub pre_action_fade_seconds: f32,
}

impl Default for IdleConfig {
    fn default() -> Self {
        Self {
            behaviors: Vec::new(),
            pre_action_fade_seconds: 2.0,
        }
    }
}

/// Port of `IdleActionKind` (config_types.h:298-306).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleActionKind {
    #[default]
    None = 0,
    Command = 1,
    Lock = 2,
    ScreenOff = 3,
    ScreenOn = 4,
    Suspend = 5,
    LockAndSuspend = 6,
}

/// Port of `IdleActionRequest` (config_types.h:308-314).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdleActionRequest {
    pub kind: IdleActionKind,
    pub command: String,
    pub lock_before_suspend: bool,
}

impl Default for IdleActionRequest {
    fn default() -> Self {
        Self {
            kind: IdleActionKind::None,
            command: String::new(),
            lock_before_suspend: true,
        }
    }
}

/// Port of `ResolvedIdleBehavior` (config_types.h:316-322).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedIdleBehavior {
    pub idle_action: IdleActionRequest,
    pub resume_action: IdleActionRequest,
    pub resume_command: String,
}

/// Port of `idleAction` helper function.
#[inline]
pub fn idle_action(kind: IdleActionKind, lock_before_suspend: bool) -> IdleActionRequest {
    IdleActionRequest {
        kind,
        command: String::new(),
        lock_before_suspend,
    }
}

/// Port of `commandIdleAction` helper function.
#[inline]
pub fn command_idle_action(command: String) -> IdleActionRequest {
    IdleActionRequest {
        kind: IdleActionKind::Command,
        command,
        lock_before_suspend: true,
    }
}

/// Port of `normalizeIdleBehaviorAction` (config_types.cpp:199-203).
pub fn normalize_idle_behavior_action(behavior: &mut IdleBehaviorConfig) {
    if behavior.action == "suspend" && behavior.lock_before_suspend {
        behavior.action = "lock_and_suspend".to_string();
    }
}

/// Port of `resolveIdleBehaviorActions` (config_types.cpp:205-242).
pub fn resolve_idle_behavior_actions(behavior: &IdleBehaviorConfig) -> ResolvedIdleBehavior {
    let mut tmp = behavior.clone();
    normalize_idle_behavior_action(&mut tmp);
    let act = tmp.action.as_str();

    if act == "lock" {
        return ResolvedIdleBehavior {
            idle_action: idle_action(IdleActionKind::Lock, true),
            resume_action: IdleActionRequest::default(),
            resume_command: tmp.resume_command,
        };
    }
    if act == "screen_off" {
        return ResolvedIdleBehavior {
            idle_action: idle_action(IdleActionKind::ScreenOff, true),
            resume_action: idle_action(IdleActionKind::ScreenOn, true),
            resume_command: tmp.resume_command,
        };
    }
    if act == "suspend" {
        return ResolvedIdleBehavior {
            idle_action: idle_action(IdleActionKind::Suspend, true),
            resume_action: IdleActionRequest::default(),
            resume_command: tmp.resume_command,
        };
    }
    if act == "lock_and_suspend" {
        return ResolvedIdleBehavior {
            idle_action: idle_action(IdleActionKind::LockAndSuspend, true),
            resume_action: IdleActionRequest::default(),
            resume_command: tmp.resume_command,
        };
    }
    ResolvedIdleBehavior {
        idle_action: command_idle_action(behavior.command.clone()),
        resume_action: IdleActionRequest::default(),
        resume_command: behavior.resume_command.clone(),
    }
}

/// Port of `defaultIdleBehaviors` (config_types.cpp:119-146).
pub fn default_idle_behaviors() -> Vec<IdleBehaviorConfig> {
    vec![
        IdleBehaviorConfig {
            name: "lock".to_string(),
            enabled: false,
            timeout_seconds: 600.0,
            action: "lock".to_string(),
            command: String::new(),
            resume_command: String::new(),
            lock_before_suspend: true,
        },
        IdleBehaviorConfig {
            name: "screen-off".to_string(),
            enabled: false,
            timeout_seconds: 660.0,
            action: "screen_off".to_string(),
            command: String::new(),
            resume_command: String::new(),
            lock_before_suspend: true,
        },
        IdleBehaviorConfig {
            name: "lock-and-suspend".to_string(),
            enabled: false,
            timeout_seconds: 900.0,
            action: "lock_and_suspend".to_string(),
            command: String::new(),
            resume_command: String::new(),
            lock_before_suspend: true,
        },
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_idle_behaviors_matches_cpp() {
        let defaults = default_idle_behaviors();
        assert_eq!(defaults.len(), 3);
        assert_eq!(defaults[0].name, "lock");
        assert_eq!(defaults[0].timeout_seconds, 600.0);
        assert_eq!(defaults[0].action, "lock");

        assert_eq!(defaults[1].name, "screen-off");
        assert_eq!(defaults[1].timeout_seconds, 660.0);
        assert_eq!(defaults[1].action, "screen_off");

        assert_eq!(defaults[2].name, "lock-and-suspend");
        assert_eq!(defaults[2].timeout_seconds, 900.0);
        assert_eq!(defaults[2].action, "lock_and_suspend");
    }

    #[test]
    fn resolve_idle_behavior_actions_lock() {
        let behavior = IdleBehaviorConfig {
            action: "lock".to_string(),
            resume_command: "echo resumed".to_string(),
            ..IdleBehaviorConfig::default()
        };
        let res = resolve_idle_behavior_actions(&behavior);
        assert_eq!(res.idle_action.kind, IdleActionKind::Lock);
        assert_eq!(res.resume_action.kind, IdleActionKind::None);
        assert_eq!(res.resume_command, "echo resumed");
    }

    #[test]
    fn resolve_idle_behavior_actions_screen_off() {
        let behavior = IdleBehaviorConfig {
            action: "screen_off".to_string(),
            ..IdleBehaviorConfig::default()
        };
        let res = resolve_idle_behavior_actions(&behavior);
        assert_eq!(res.idle_action.kind, IdleActionKind::ScreenOff);
        assert_eq!(res.resume_action.kind, IdleActionKind::ScreenOn);
    }

    #[test]
    fn resolve_idle_behavior_actions_suspend_normalizes_to_lock_and_suspend() {
        let behavior = IdleBehaviorConfig {
            action: "suspend".to_string(),
            lock_before_suspend: true,
            ..IdleBehaviorConfig::default()
        };
        let res = resolve_idle_behavior_actions(&behavior);
        assert_eq!(res.idle_action.kind, IdleActionKind::LockAndSuspend);
    }

    #[test]
    fn resolve_idle_behavior_actions_custom_command() {
        let behavior = IdleBehaviorConfig {
            action: "custom".to_string(),
            command: "dim-screen.sh".to_string(),
            resume_command: "undim-screen.sh".to_string(),
            ..IdleBehaviorConfig::default()
        };
        let res = resolve_idle_behavior_actions(&behavior);
        assert_eq!(res.idle_action.kind, IdleActionKind::Command);
        assert_eq!(res.idle_action.command, "dim-screen.sh");
        assert_eq!(res.resume_command, "undim-screen.sh");
    }

    #[test]
    fn idle_deserializes_from_example_toml() {
        const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let idle_tbl = root
            .get("idle")
            .expect("[idle] table present in example.toml");
        let config: IdleConfig = idle_tbl
            .clone()
            .try_into()
            .expect("[idle] parses into IdleConfig");

        assert_eq!(config.pre_action_fade_seconds, 2.0);
    }
}
