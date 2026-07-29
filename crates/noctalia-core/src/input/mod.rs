//! Port of `src/core/input/*`'s data shapes needed ahead of task 10.2
//! (Wayland seat + input — `wl::seat`, xkbcommon-backed keyboard handling).
//! Task 2.1.3 (shell config) needs `KeyChord` as a plain data field
//! (`SessionPanelActionConfig::shortcut`), and task 2.1.6 (idle/keybinds/
//! hotcorners/accessibility) will need it too (`KeybindsConfig`) — pulled
//! forward here as a small POD, same "shared foundation both a config task
//! and a later phase need" reasoning as `core::color`'s `ColorSpec` (task
//! 2.1.1).
//!
//! `key_chord.h`'s real logic — `parseKeyChordSpec`/`keyChordToString`/
//! `keyChordDisplayLabel`/`keyChordMatches`/`isPrintableKey`/
//! `isPlainPrintableKey` — all depend on `xkbcommon` keysym lookups and stay
//! task 10.2's job; not ported here.

pub mod key_chord;

pub use key_chord::KeyChord;
