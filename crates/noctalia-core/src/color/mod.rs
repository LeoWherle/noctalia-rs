//! Color primitives (task 2.1.1): `Color`, `ColorRole`/`ColorSpec`, and their
//! config-string round-trip, shared by both Phase 2 (`noctalia-config`) and
//! Phase 3 (`noctalia-theme`, task 3.1). Ported from `render/core/color.h`,
//! the non-`Palette` half of `ui/palette.h`, and `config/color_spec.h`
//! (whose bodies live in `config_types.cpp`).
//!
//! Deliberately out of scope: `Palette` and palette-scheme generation (the
//! live `palette` global, `colorForRole`, `resolveColorSpec`,
//! `isLightPalette`, `lerpPalette`, `Signal<> paletteChanged()`) — those need
//! the process-wide palette singleton, which is task 3.1's `noctalia-theme`
//! territory. `ColorSpec` here is the small, palette-agnostic POD
//! (`role: Option<ColorRole>, fixed: Color, alpha: f32`) both future crates
//! build on; resolving a `ColorSpec` against a live palette stays task 3.1's
//! job.

mod config_string;
mod rgb;
mod spec;

pub use config_string::{
    ColorSpecParseError, color_spec_from_config_string, color_spec_to_config_string,
};
pub use rgb::{
    Color, HexColorError, brighten, clear_color, format_rgb_hex, hex, hsl, hsv, lerp_color,
    lerp_hsv, readable_text_color_for_background, relative_luminance, rgb_hex, rgb_to_hsv, rgba,
    rgba_hex, try_parse_css_color, try_parse_hex_color, with_alpha,
};
pub use spec::{
    COLOR_ROLE_TOKENS, ColorRole, ColorRoleToken, ColorSpec, clear_color_spec,
    color_role_from_token, color_role_token, color_spec_from_role, fixed_color_spec,
};
