//! Task 2.1.4 — Lockscreen configuration type.
//! Port of `LockscreenConfig` and `isLockScreenEnabled` from
//! `src/config/config_types.h` (lines 492-507) and `config_schema.cpp:89-101`.

use serde::{Deserialize, Serialize};

/// Port of `LockscreenConfig` (config_types.h:492-503).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockscreenConfig {
    pub enabled: bool,
    pub fingerprint: bool,
    pub allow_empty_password: bool,
    pub blurred_desktop: bool,
    pub blur_intensity: f32,
    pub tint_intensity: f32,
    pub wallpaper: String,
    pub monitors: Vec<String>,
}

impl Default for LockscreenConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fingerprint: true,
            allow_empty_password: false,
            blurred_desktop: false,
            blur_intensity: 0.5,
            tint_intensity: 0.3,
            wallpaper: String::new(),
            monitors: Vec::new(),
        }
    }
}

/// Port of `isLockScreenEnabled` (config_types.h:505-507).
#[inline]
pub fn is_lock_screen_enabled(lockscreen: &LockscreenConfig) -> bool {
    lockscreen.enabled
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn lockscreen_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let lockscreen_tbl = root
            .get("lockscreen")
            .expect("[lockscreen] table present in example.toml");
        let config: LockscreenConfig = lockscreen_tbl
            .clone()
            .try_into()
            .expect("[lockscreen] parses into LockscreenConfig");

        assert!(config.enabled);
        assert!(!config.blurred_desktop);
        assert_eq!(config.blur_intensity, 0.5);
        assert_eq!(config.tint_intensity, 0.3);
    }

    #[test]
    fn lockscreen_config_defaults_match_cpp() {
        let config = LockscreenConfig::default();
        assert!(config.enabled);
        assert!(config.fingerprint);
        assert!(!config.allow_empty_password);
        assert!(!config.blurred_desktop);
        assert_eq!(config.blur_intensity, 0.5);
        assert_eq!(config.tint_intensity, 0.3);
        assert_eq!(config.wallpaper, "");
        assert!(config.monitors.is_empty());
        assert!(is_lock_screen_enabled(&config));
    }
}
