//! Port of the non-`Palette` half of `ui/palette.h`/`palette.cpp` (task
//! 2.1.1): `ColorRole`, `ColorRoleToken`/`kColorRoleTokens`, `ColorSpec`, and
//! the token<->role/spec-construction helpers that don't touch the live
//! `palette` global. `colorForRole`/`resolveColorSpec`/`isLightPalette` and
//! the `Palette` struct itself stay out of scope here — they need the
//! process-wide palette singleton `setPalette()` writes, which belongs to
//! task 3.1's `noctalia-theme`, not this crate.

use crate::color::rgb::{Color, clear_color, trim_c_whitespace};

/// Port of `ColorRole` (palette.h:12-29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRole {
    Primary,
    OnPrimary,
    Secondary,
    OnSecondary,
    Tertiary,
    OnTertiary,
    Error,
    OnError,
    Surface,
    OnSurface,
    SurfaceVariant,
    OnSurfaceVariant,
    Outline,
    Shadow,
    Hover,
    OnHover,
}

/// Port of `ColorRoleToken` (palette.h:31-34).
#[derive(Debug, Clone, Copy)]
pub struct ColorRoleToken {
    pub role: ColorRole,
    pub token: &'static str,
}

/// Port of `kColorRoleTokens` (palette.h:36-53).
pub const COLOR_ROLE_TOKENS: [ColorRoleToken; 16] = [
    ColorRoleToken {
        role: ColorRole::Primary,
        token: "primary",
    },
    ColorRoleToken {
        role: ColorRole::OnPrimary,
        token: "on_primary",
    },
    ColorRoleToken {
        role: ColorRole::Secondary,
        token: "secondary",
    },
    ColorRoleToken {
        role: ColorRole::OnSecondary,
        token: "on_secondary",
    },
    ColorRoleToken {
        role: ColorRole::Tertiary,
        token: "tertiary",
    },
    ColorRoleToken {
        role: ColorRole::OnTertiary,
        token: "on_tertiary",
    },
    ColorRoleToken {
        role: ColorRole::Error,
        token: "error",
    },
    ColorRoleToken {
        role: ColorRole::OnError,
        token: "on_error",
    },
    ColorRoleToken {
        role: ColorRole::Surface,
        token: "surface",
    },
    ColorRoleToken {
        role: ColorRole::OnSurface,
        token: "on_surface",
    },
    ColorRoleToken {
        role: ColorRole::SurfaceVariant,
        token: "surface_variant",
    },
    ColorRoleToken {
        role: ColorRole::OnSurfaceVariant,
        token: "on_surface_variant",
    },
    ColorRoleToken {
        role: ColorRole::Outline,
        token: "outline",
    },
    ColorRoleToken {
        role: ColorRole::Shadow,
        token: "shadow",
    },
    ColorRoleToken {
        role: ColorRole::Hover,
        token: "hover",
    },
    ColorRoleToken {
        role: ColorRole::OnHover,
        token: "on_hover",
    },
];

/// Either a palette role (resolved later against the live palette) or a fixed
/// color, plus an alpha multiplier applied on resolution. Port of
/// `ColorSpec` (palette.h:57-61).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorSpec {
    pub role: Option<ColorRole>,
    pub fixed: Color,
    pub alpha: f32,
}

/// Port of `clearColorSpec()` (palette.h:67-69).
pub fn clear_color_spec() -> ColorSpec {
    ColorSpec {
        role: None,
        fixed: clear_color(),
        alpha: 1.0,
    }
}

/// Trims and lowercases (ASCII-only, matching `StringUtils::toLowerInPlace`'s
/// byte-wise `tolower`) before comparing against `COLOR_ROLE_TOKENS`. Port of
/// `normalizedRoleToken()` (palette.cpp:14-18, anonymous namespace).
fn normalized_role_token(token: &str) -> String {
    trim_c_whitespace(token).to_ascii_lowercase()
}

/// Port of `colorRoleFromToken()` (palette.cpp:67-75).
pub fn color_role_from_token(token: &str) -> Option<ColorRole> {
    let normalized = normalized_role_token(token);
    COLOR_ROLE_TOKENS
        .iter()
        .find(|entry| entry.token == normalized)
        .map(|entry| entry.role)
}

/// Port of `colorRoleToken()` (palette.cpp:77-84). The C++ falls back to
/// `"on_surface"` if `role` somehow isn't in the table (structurally
/// unreachable given `ColorRole`'s exhaustive `match` below, but the
/// fallback is preserved as dead-but-documented parity rather than an
/// `unreachable!()`, since the whole point of this table-driven design is
/// that adding a `ColorRole` variant without a token is a silent C++ bug,
/// not a Rust compile error).
pub fn color_role_token(role: ColorRole) -> &'static str {
    COLOR_ROLE_TOKENS
        .iter()
        .find(|entry| entry.role == role)
        .map_or("on_surface", |entry| entry.token)
}

/// Port of `colorSpecFromRole()` (palette.cpp:86-88).
pub fn color_spec_from_role(role: ColorRole, alpha: f32) -> ColorSpec {
    ColorSpec {
        role: Some(role),
        fixed: clear_color(),
        alpha,
    }
}

/// Port of `fixedColorSpec()` (palette.cpp:90-92).
pub fn fixed_color_spec(color: Color) -> ColorSpec {
    ColorSpec {
        role: None,
        fixed: color,
        alpha: 1.0,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::color::rgb::rgba;

    #[test]
    fn every_role_token_round_trips() {
        for entry in &COLOR_ROLE_TOKENS {
            assert_eq!(color_role_from_token(entry.token), Some(entry.role));
            assert_eq!(color_role_token(entry.role), entry.token);
        }
    }

    #[test]
    fn color_role_from_token_trims_and_lowercases() {
        assert_eq!(
            color_role_from_token("  ON_SURFACE  "),
            Some(ColorRole::OnSurface)
        );
        assert_eq!(color_role_from_token("Primary"), Some(ColorRole::Primary));
    }

    #[test]
    fn color_role_from_token_rejects_unknown() {
        assert_eq!(color_role_from_token("not_a_role"), None);
        assert_eq!(color_role_from_token(""), None);
    }

    #[test]
    fn color_spec_from_role_has_no_fixed_color_and_given_alpha() {
        let spec = color_spec_from_role(ColorRole::Secondary, 0.5);
        assert_eq!(spec.role, Some(ColorRole::Secondary));
        assert_eq!(spec.fixed, clear_color());
        assert_eq!(spec.alpha, 0.5);
    }

    #[test]
    fn fixed_color_spec_has_no_role_and_full_alpha() {
        let color = rgba(0.1, 0.2, 0.3, 0.4);
        let spec = fixed_color_spec(color);
        assert_eq!(spec.role, None);
        assert_eq!(spec.fixed, color);
        assert_eq!(spec.alpha, 1.0);
    }

    #[test]
    fn clear_color_spec_matches_cpp_default() {
        let spec = clear_color_spec();
        assert_eq!(spec.role, None);
        assert_eq!(spec.fixed, clear_color());
        assert_eq!(spec.alpha, 1.0);
    }
}
