//! Task 2.1.5 — Shadow direction & offset utilities.
//! Port of `ShadowDirection`, `ShadowDirectionOffset`, and `shadowDirectionOffset`
//! from `src/config/config_types.h` (lines 745-796).

use serde::{Deserialize, Serialize};

/// Port of `ShadowDirection` (config_types.h:745-755).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowDirection {
    Center = 0,
    #[default]
    Down = 1,
    Up = 2,
    Left = 3,
    Right = 4,
    DownLeft = 5,
    DownRight = 6,
    UpLeft = 7,
    UpRight = 8,
}

/// Port of `ShadowDirectionOffset` (config_types.h:769-772).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShadowDirectionOffset {
    pub x: i32,
    pub y: i32,
}

/// Port of `shadowDirectionOffset` (config_types.h:774-796).
#[inline]
pub fn shadow_direction_offset(dir: ShadowDirection) -> ShadowDirectionOffset {
    match dir {
        ShadowDirection::Center => ShadowDirectionOffset { x: 0, y: 0 },
        ShadowDirection::Down => ShadowDirectionOffset { x: 0, y: 2 },
        ShadowDirection::Up => ShadowDirectionOffset { x: 0, y: -2 },
        ShadowDirection::Left => ShadowDirectionOffset { x: -2, y: 0 },
        ShadowDirection::Right => ShadowDirectionOffset { x: 2, y: 0 },
        ShadowDirection::DownLeft => ShadowDirectionOffset { x: -2, y: 2 },
        ShadowDirection::DownRight => ShadowDirectionOffset { x: 2, y: 2 },
        ShadowDirection::UpLeft => ShadowDirectionOffset { x: -2, y: -2 },
        ShadowDirection::UpRight => ShadowDirectionOffset { x: 2, y: -2 },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn shadow_direction_offsets_match_cpp() {
        assert_eq!(
            shadow_direction_offset(ShadowDirection::Center),
            ShadowDirectionOffset { x: 0, y: 0 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::Down),
            ShadowDirectionOffset { x: 0, y: 2 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::Up),
            ShadowDirectionOffset { x: 0, y: -2 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::Left),
            ShadowDirectionOffset { x: -2, y: 0 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::Right),
            ShadowDirectionOffset { x: 2, y: 0 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::DownLeft),
            ShadowDirectionOffset { x: -2, y: 2 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::DownRight),
            ShadowDirectionOffset { x: 2, y: 2 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::UpLeft),
            ShadowDirectionOffset { x: -2, y: -2 }
        );
        assert_eq!(
            shadow_direction_offset(ShadowDirection::UpRight),
            ShadowDirectionOffset { x: 2, y: -2 }
        );
    }

    #[test]
    fn shadow_direction_serde_roundtrip() {
        assert_eq!(
            serde_json::from_str::<ShadowDirection>("\"down\"").unwrap(),
            ShadowDirection::Down
        );
        assert_eq!(
            serde_json::from_str::<ShadowDirection>("\"down_left\"").unwrap(),
            ShadowDirection::DownLeft
        );
    }
}
