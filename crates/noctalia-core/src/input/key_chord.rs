//! Port of `KeyChord`'s data shape only (`src/core/input/key_chord.h:8-13`).
//! See the parent module doc comment for what's deliberately not ported yet.

/// Port of `KeyChord` (key_chord.h:8-13): an XKB keysym plus a `KeyMod`
/// bitmask. `Default` matches the C++'s in-class member initializers
/// (`sym = 0`, `modifiers = 0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub sym: u32,
    pub modifiers: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_cpp_in_class_initializers() {
        assert_eq!(
            KeyChord::default(),
            KeyChord {
                sym: 0,
                modifiers: 0
            }
        );
    }
}
