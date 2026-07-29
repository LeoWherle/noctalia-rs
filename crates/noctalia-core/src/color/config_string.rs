//! Port of `config/color_spec.{h,cpp}` (task 2.1.1): parsing/serializing a
//! user-facing config color value (a palette role token or a hex color) to
//! and from a [`ColorSpec`]. The actual bodies live in `config_types.cpp`
//! (`color_spec.h` only declares them) — `colorSpecFromConfigString`
//! (config_types.cpp:521-523) delegates to the anonymous-namespace
//! `parseColorSpecString` (config_types.cpp:39-49), and
//! `colorSpecToConfigString` (config_types.cpp:541-548) delegates to
//! `colorToConfigString` (config_types.cpp:526-538).

use thiserror::Error;

use crate::color::rgb::{Color, format_rgb_hex, trim_c_whitespace, try_parse_hex_color};
use crate::color::spec::{
    ColorSpec, color_role_from_token, color_role_token, color_spec_from_role, fixed_color_spec,
};

/// Port of `colorSpecError()` (config_types.cpp:27-37): the message
/// `parseColorSpecString` raises on a value that's neither a role token nor
/// a hex color.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{context}invalid color value \"{raw}\" (expected a color role token or hex color)")]
pub struct ColorSpecParseError {
    raw: String,
    context: String,
}

/// Parses a user-facing color value: either a palette role token or a hex
/// color. `context` is prefixed to the error message (e.g. a config key
/// path) when non-empty, matching `colorSpecError`'s `context + ": "`
/// prefixing. Port of `colorSpecFromConfigString`
/// (config_types.cpp:39-49,521-523).
pub fn color_spec_from_config_string(
    raw: &str,
    context: &str,
) -> Result<ColorSpec, ColorSpecParseError> {
    let trimmed = trim_c_whitespace(raw);
    if let Some(color) = try_parse_hex_color(trimmed) {
        return Ok(fixed_color_spec(color));
    }
    if let Some(role) = color_role_from_token(trimmed) {
        return Ok(color_spec_from_role(role, 1.0));
    }
    let context = if context.is_empty() {
        String::new()
    } else {
        format!("{context}: ")
    };
    Err(ColorSpecParseError {
        raw: raw.to_string(),
        context,
    })
}

/// Port of `colorByteForExport()` (config_types.cpp:526, anonymous
/// namespace) — the same round-to-nearest-byte formula as `format_rgb_hex`'s
/// private `toByte`, kept as its own copy since the C++ doesn't share it
/// between `color.cpp` and `config_types.cpp` either.
fn color_byte_for_export(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Port of `colorToConfigString()` (config_types.cpp:528-538, anonymous
/// namespace): opaque colors serialize as `formatRgbHex`'s 6-digit form,
/// anything with alpha as an 8-digit `#RRGGBBAA`.
fn color_to_config_string(color: Color) -> String {
    if color.a >= 0.999 {
        return format_rgb_hex(color);
    }
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color_byte_for_export(color.r),
        color_byte_for_export(color.g),
        color_byte_for_export(color.b),
        color_byte_for_export(color.a)
    )
}

/// Serializes a color spec to its stable config representation (palette role
/// token or hex). Port of `colorSpecToConfigString`
/// (config_types.cpp:541-548).
pub fn color_spec_to_config_string(spec: &ColorSpec) -> String {
    if let Some(role) = spec.role {
        return color_role_token(role).to_string();
    }
    let mut color = spec.fixed;
    color.a *= spec.alpha;
    color_to_config_string(color)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::color::rgb::rgba;
    use crate::color::spec::{COLOR_ROLE_TOKENS, ColorRole, clear_color_spec};

    #[test]
    fn every_role_token_round_trips_through_config_string() {
        for entry in &COLOR_ROLE_TOKENS {
            let spec = color_spec_from_config_string(entry.token, "").expect("valid role token");
            assert_eq!(spec.role, Some(entry.role));
            assert_eq!(color_spec_to_config_string(&spec), entry.token);
        }
    }

    #[test]
    fn hex_strings_parse_as_fixed_specs() {
        for raw in ["#E6B450", "#0B0E14", "#fff", "#0f08"] {
            let spec = color_spec_from_config_string(raw, "").expect("valid hex");
            assert_eq!(spec.role, None);
        }
    }

    #[test]
    fn opaque_fixed_spec_serializes_as_6_digit_hex() {
        let spec = color_spec_from_config_string("#E6B450", "").expect("valid hex");
        assert_eq!(color_spec_to_config_string(&spec), "#E6B450");
    }

    #[test]
    fn translucent_fixed_spec_serializes_as_8_digit_hex() {
        let spec = color_spec_from_config_string("#0f08", "").expect("valid hex");
        assert_eq!(color_spec_to_config_string(&spec), "#00FF0088");
    }

    #[test]
    fn role_token_is_case_insensitive_and_trims_whitespace() {
        let spec = color_spec_from_config_string("  Primary  ", "").expect("valid role token");
        assert_eq!(spec.role, Some(ColorRole::Primary));
    }

    #[test]
    fn invalid_value_reports_context_and_raw_value() {
        let err = color_spec_from_config_string("not-a-color", "bar.background").unwrap_err();
        assert_eq!(
            err.to_string(),
            "bar.background: invalid color value \"not-a-color\" (expected a color role token or hex color)"
        );
    }

    #[test]
    fn invalid_value_without_context_omits_prefix() {
        let err = color_spec_from_config_string("not-a-color", "").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid color value \"not-a-color\" (expected a color role token or hex color)"
        );
    }

    #[test]
    fn clear_color_spec_serializes_as_transparent_black_hex() {
        assert_eq!(
            color_spec_to_config_string(&clear_color_spec()),
            "#00000000"
        );
    }

    #[test]
    fn fixed_spec_alpha_multiplier_is_applied_before_serializing() {
        let spec = ColorSpec {
            role: None,
            fixed: rgba(1.0, 1.0, 1.0, 1.0),
            alpha: 0.5,
        };
        assert_eq!(color_spec_to_config_string(&spec), "#FFFFFF80");
    }
}
