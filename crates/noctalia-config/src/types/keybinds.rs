//! Task 2.1.6 — Keybinds configuration types & default keybind definitions.
//! Port of `KeybindAction`, `defaultKeybindSet`, and `KeybindsConfig` from
//! `src/config/config_types.{h,cpp}` (structs: `config_types.h:327-338, 1231-1242`;
//! bodies: `config_types.cpp:148-172`).

use noctalia_core::input::KeyChord;
use serde::{Deserialize, Serialize};

/// Port of `KeybindAction` (config_types.h:327-336).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeybindAction {
    #[default]
    Validate = 0,
    Cancel = 1,
    Left = 2,
    Right = 3,
    Up = 4,
    Down = 5,
    TabNext = 6,
    TabPrevious = 7,
}

// XKB keysym constants matching C++ <xkbcommon/xkbcommon-keysyms.h>
const XKB_KEY_RETURN: u32 = 0xff0d;
const XKB_KEY_KP_ENTER: u32 = 0xff8d;
const XKB_KEY_SPACE: u32 = 0x0020;
const XKB_KEY_ESCAPE: u32 = 0xff1b;
const XKB_KEY_LEFT: u32 = 0xff51;
const XKB_KEY_RIGHT: u32 = 0xff53;
const XKB_KEY_UP: u32 = 0xff52;
const XKB_KEY_DOWN: u32 = 0xff54;
const XKB_KEY_TAB: u32 = 0xff09;
const XKB_KEY_ISO_LEFT_TAB: u32 = 0xfe20;

// KeyMod::Shift bitmask constant from key_modifiers.h:6 (1U << 0)
const KEY_MOD_SHIFT: u32 = 1;

/// Port of `defaultKeybindSet` (config_types.cpp:148-172).
pub fn default_keybind_set(action: KeybindAction) -> Vec<KeyChord> {
    match action {
        KeybindAction::Validate => vec![
            KeyChord {
                sym: XKB_KEY_RETURN,
                modifiers: 0,
            },
            KeyChord {
                sym: XKB_KEY_KP_ENTER,
                modifiers: 0,
            },
            KeyChord {
                sym: XKB_KEY_SPACE,
                modifiers: 0,
            },
        ],
        KeybindAction::Cancel => vec![KeyChord {
            sym: XKB_KEY_ESCAPE,
            modifiers: 0,
        }],
        KeybindAction::Left => vec![KeyChord {
            sym: XKB_KEY_LEFT,
            modifiers: 0,
        }],
        KeybindAction::Right => vec![KeyChord {
            sym: XKB_KEY_RIGHT,
            modifiers: 0,
        }],
        KeybindAction::Up => vec![KeyChord {
            sym: XKB_KEY_UP,
            modifiers: 0,
        }],
        KeybindAction::Down => vec![KeyChord {
            sym: XKB_KEY_DOWN,
            modifiers: 0,
        }],
        KeybindAction::TabNext => vec![KeyChord {
            sym: XKB_KEY_TAB,
            modifiers: 0,
        }],
        KeybindAction::TabPrevious => vec![KeyChord {
            sym: XKB_KEY_ISO_LEFT_TAB,
            modifiers: KEY_MOD_SHIFT,
        }],
    }
}

/// Port of `KeybindsConfig` (config_types.h:1231-1242).
/// TOML string<->KeyChord parsing bridge is task 10.2's job (needs `xkbcommon` FFI);
/// fields are currently `#[serde(skip)]` populated via defaults or task 10.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindsConfig {
    #[serde(skip)]
    pub validate: Vec<KeyChord>,
    #[serde(skip)]
    pub cancel: Vec<KeyChord>,
    #[serde(skip)]
    pub left: Vec<KeyChord>,
    #[serde(skip)]
    pub right: Vec<KeyChord>,
    #[serde(skip)]
    pub up: Vec<KeyChord>,
    #[serde(skip)]
    pub down: Vec<KeyChord>,
    #[serde(skip, rename = "tab_next")]
    pub tab_next: Vec<KeyChord>,
    #[serde(skip, rename = "tab_previous")]
    pub tab_previous: Vec<KeyChord>,
}

impl Default for KeybindsConfig {
    fn default() -> Self {
        Self {
            validate: default_keybind_set(KeybindAction::Validate),
            cancel: default_keybind_set(KeybindAction::Cancel),
            left: default_keybind_set(KeybindAction::Left),
            right: default_keybind_set(KeybindAction::Right),
            up: default_keybind_set(KeybindAction::Up),
            down: default_keybind_set(KeybindAction::Down),
            tab_next: default_keybind_set(KeybindAction::TabNext),
            tab_previous: default_keybind_set(KeybindAction::TabPrevious),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const EXAMPLE_TOML: &str = include_str!("../../../../example.toml");

    #[test]
    fn keybinds_deserializes_from_example_toml() {
        let root: toml::Table = toml::from_str(EXAMPLE_TOML).expect("example.toml is valid TOML");
        let keybinds_tbl = root
            .get("keybinds")
            .expect("[keybinds] table present in example.toml");
        let config: KeybindsConfig = keybinds_tbl
            .clone()
            .try_into()
            .expect("[keybinds] parses into KeybindsConfig");

        assert_eq!(
            config.cancel,
            vec![KeyChord {
                sym: XKB_KEY_ESCAPE,
                modifiers: 0
            }]
        );
    }

    #[test]
    fn default_keybind_set_matches_cpp() {
        assert_eq!(default_keybind_set(KeybindAction::Cancel).len(), 1);
        assert_eq!(default_keybind_set(KeybindAction::Validate).len(), 3);
        assert_eq!(
            default_keybind_set(KeybindAction::TabPrevious),
            vec![KeyChord {
                sym: XKB_KEY_ISO_LEFT_TAB,
                modifiers: 1,
            }]
        );
    }
}
