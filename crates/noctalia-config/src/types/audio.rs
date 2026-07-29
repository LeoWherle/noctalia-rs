//! Task 2.1.7 — Audio configuration types & volume utilities.
//! Port of `AudioConfig` and `maxAudioVolume` from `src/config/config_types.h`
//! (lines 1168-1181) and `config_schema.cpp:21-30`.

use serde::{Deserialize, Serialize};

/// Port of `AudioConfig` (config_types.h:1168-1176).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub enable_overdrive: bool,
    pub enable_sounds: bool,
    pub sound_volume: f32,
    pub volume_change_sound: String,
    pub notification_sound: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enable_overdrive: false,
            enable_sounds: false,
            sound_volume: 0.5,
            volume_change_sound: String::new(),
            notification_sound: String::new(),
        }
    }
}

/// Port of `maxAudioVolume` (config_types.h:1179-1181).
#[inline]
pub fn max_audio_volume(audio: &AudioConfig) -> f32 {
    if audio.enable_overdrive { 1.5 } else { 1.0 }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn audio_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let audio_tbl = root
            .get("audio")
            .expect("[audio] table present in example.toml");
        let config: AudioConfig = audio_tbl
            .clone()
            .try_into()
            .expect("[audio] parses into AudioConfig");

        assert!(!config.enable_overdrive);
        assert!(!config.enable_sounds);
        assert_eq!(config.sound_volume, 0.5);
    }

    #[test]
    fn max_audio_volume_reflects_overdrive() {
        let mut config = AudioConfig::default();
        assert_eq!(max_audio_volume(&config), 1.0);
        config.enable_overdrive = true;
        assert_eq!(max_audio_volume(&config), 1.5);
    }
}
