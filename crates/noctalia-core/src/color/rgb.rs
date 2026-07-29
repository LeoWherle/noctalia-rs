//! Port of `render/core/color.{h,cpp}` (task 2.1.1): the `Color` RGBA POD and
//! its free-function toolbox (hex/CSS parsing and formatting, HSV/HSL
//! conversion, luminance, blending). Deliberately excludes `Palette`/palette
//! scheme generation (`ui/palette.h`'s `palette` global, `colorForRole`,
//! `resolveColorSpec`, `isLightPalette`) — those need the live palette
//! singleton, which is task 3.1's `noctalia-theme` territory, not this crate.

use thiserror::Error;

/// RGBA color; every channel is a float in `[0, 1]`. Port of `Color`
/// (color.h:9-14). C++ compares fields with a raw `==`; `PartialEq` here does
/// the same (no epsilon tolerance), matching `operator==`'s exactness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
    }
}

/// Port of `rgba()` (color.h:20).
pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

/// Port of `clearColor()` (palette.h:55): fully transparent black.
pub fn clear_color() -> Color {
    rgba(0.0, 0.0, 0.0, 0.0)
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Maps a 0-255 channel byte to a `[0, 1]` float. Port of `colorByte()`
/// (color.h:23).
fn color_byte(value: u32) -> f32 {
    value as f32 / 255.0
}

/// Builds a `Color` from a packed `0xRRGGBB` integer (alpha = 1). Port of
/// `rgbHex()` (color.h:26-33).
pub fn rgb_hex(value: u32) -> Color {
    Color {
        r: color_byte((value >> 16) & 0xFF),
        g: color_byte((value >> 8) & 0xFF),
        b: color_byte(value & 0xFF),
        a: 1.0,
    }
}

/// Builds a `Color` from a packed `0xRRGGBBAA` integer. Port of `rgbaHex()`
/// (color.h:36-43).
pub fn rgba_hex(value: u32) -> Color {
    Color {
        r: color_byte((value >> 24) & 0xFF),
        g: color_byte((value >> 16) & 0xFF),
        b: color_byte((value >> 8) & 0xFF),
        a: color_byte(value & 0xFF),
    }
}

/// Port of `hex()`'s failure modes (color.h:46-104): both `hexDigit` and `hex`
/// itself throw `std::invalid_argument` in the C++; this is the `Result`
/// equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HexColorError {
    #[error("hex color must start with '#'")]
    MissingHash,
    #[error("invalid hex digit")]
    InvalidDigit,
    #[error("unsupported hex color format")]
    UnsupportedFormat,
}

/// One hex digit (`'0'-'9'`, `'a'-'f'`, `'A'-'F'`) to its value. Port of
/// `hexDigit()` (color.h:46-57).
fn hex_digit(c: u8) -> Result<u32, HexColorError> {
    match c {
        b'0'..=b'9' => Ok(u32::from(c - b'0')),
        b'a'..=b'f' => Ok(10 + u32::from(c - b'a')),
        b'A'..=b'F' => Ok(10 + u32::from(c - b'A')),
        _ => Err(HexColorError::InvalidDigit),
    }
}

/// Port of `hexByte()` (color.h:59).
fn hex_byte(high: u8, low: u8) -> Result<u32, HexColorError> {
    Ok((hex_digit(high)? << 4) | hex_digit(low)?)
}

/// Parses `"#rgb"`, `"#rgba"`, `"#rrggbb"`, or `"#rrggbbaa"` (literal `'#'`
/// required). Port of `hex()` (color.h:62-104). Indexes by byte, not `char`,
/// matching the C++'s single-byte-per-hex-digit assumption — a non-ASCII
/// byte simply fails `hex_digit` rather than panicking on a UTF-8 boundary.
pub fn hex(value: &str) -> Result<Color, HexColorError> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes[0] != b'#' {
        return Err(HexColorError::MissingHash);
    }

    match bytes.len() {
        4 => Ok(Color {
            r: color_byte(hex_digit(bytes[1])? * 17),
            g: color_byte(hex_digit(bytes[2])? * 17),
            b: color_byte(hex_digit(bytes[3])? * 17),
            a: 1.0,
        }),
        5 => Ok(Color {
            r: color_byte(hex_digit(bytes[1])? * 17),
            g: color_byte(hex_digit(bytes[2])? * 17),
            b: color_byte(hex_digit(bytes[3])? * 17),
            a: color_byte(hex_digit(bytes[4])? * 17),
        }),
        7 => Ok(Color {
            r: color_byte(hex_byte(bytes[1], bytes[2])?),
            g: color_byte(hex_byte(bytes[3], bytes[4])?),
            b: color_byte(hex_byte(bytes[5], bytes[6])?),
            a: 1.0,
        }),
        9 => Ok(Color {
            r: color_byte(hex_byte(bytes[1], bytes[2])?),
            g: color_byte(hex_byte(bytes[3], bytes[4])?),
            b: color_byte(hex_byte(bytes[5], bytes[6])?),
            a: color_byte(hex_byte(bytes[7], bytes[8])?),
        }),
        _ => Err(HexColorError::UnsupportedFormat),
    }
}

/// Port of `withAlpha()` (color.h:106-109).
pub fn with_alpha(color: Color, alpha: f32) -> Color {
    rgba(color.r, color.g, color.b, clamp01(alpha))
}

/// Scales rgb by `amount` (1 = unchanged, >1 brighter), clamped to `[0, 1]`;
/// alpha unchanged. Port of `brighten()` (color.h:112-120).
pub fn brighten(color: Color, amount: f32) -> Color {
    Color {
        r: clamp01(color.r * amount),
        g: clamp01(color.g * amount),
        b: clamp01(color.b * amount),
        a: color.a,
    }
}

/// Linear blend from `a` to `b`; `t` in `[0, 1]`. Port of `lerpColor()`
/// (color.h:123-130).
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Hue in turns (`[0, 1)`, wrapped); saturation/value/alpha in `[0, 1]`. Port
/// of `hsv()` (color.cpp:24-64).
pub fn hsv(h: f32, s: f32, v: f32, a: f32) -> Color {
    let h = h - h.floor();
    let saturation = s.clamp(0.0, 1.0);
    let value = v.clamp(0.0, 1.0);
    let chroma = value * saturation;
    let hh = h * 6.0;
    let x = chroma * (1.0 - ((hh % 2.0) - 1.0).abs());

    // C++ does `static_cast<int>(hh) % 6` on a non-negative `hh` (h was
    // wrapped into [0,1) above, so hh is in [0,6)); `as i32 % 6` matches for
    // every value in that range, including the hh==6.0 boundary case (h
    // exactly 0 after wrapping gives hh==0.0, not 6.0, so that edge never
    // actually arises, but the truncating cast matches regardless).
    let (rp, gp, bp) = match (hh as i32) % 6 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };

    let m = value - chroma;
    rgba(rp + m, gp + m, bp + m, a.clamp(0.0, 1.0))
}

/// Hue in degrees (wrapped to `[0, 360)`); saturation/lightness/alpha in
/// `[0, 1]`. Port of `hsl()` (color.cpp:66-102).
pub fn hsl(h: f32, s: f32, l: f32, a: f32) -> Color {
    let mut h = h % 360.0;
    if h < 0.0 {
        h += 360.0;
    }
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);

    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = chroma * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - chroma / 2.0;

    let (rp, gp, bp) = if h < 60.0 {
        (chroma, x, 0.0)
    } else if h < 120.0 {
        (x, chroma, 0.0)
    } else if h < 180.0 {
        (0.0, chroma, x)
    } else if h < 240.0 {
        (0.0, x, chroma)
    } else if h < 300.0 {
        (x, 0.0, chroma)
    } else {
        (chroma, 0.0, x)
    };

    rgba(rp + m, gp + m, bp + m, a.clamp(0.0, 1.0))
}

/// Decomposes rgb into hue (turns, `[0, 1)`), saturation, and value (each
/// `[0, 1]`). Port of `rgbToHsv()` (color.cpp:104-132).
pub fn rgb_to_hsv(rgb: Color) -> (f32, f32, f32) {
    let max_channel = rgb.r.max(rgb.g).max(rgb.b);
    let min_channel = rgb.r.min(rgb.g).min(rgb.b);
    let delta = max_channel - min_channel;

    let v = max_channel;
    if max_channel <= 1e-6 {
        return (0.0, 0.0, v);
    }

    let s = delta / max_channel;
    if delta <= 1e-6 {
        return (0.0, s, v);
    }

    let mut h = if max_channel == rgb.r {
        (rgb.g - rgb.b) / delta + if rgb.g < rgb.b { 6.0 } else { 0.0 }
    } else if max_channel == rgb.g {
        (rgb.b - rgb.r) / delta + 2.0
    } else {
        (rgb.r - rgb.g) / delta + 4.0
    };

    h /= 6.0;
    h -= h.floor();
    (h, s, v)
}

/// Blends from `a` to `b` through HSV space on the shortest hue path; `t` in
/// `[0, 1]`. Port of `lerpHsv()` (color.cpp:134-156).
pub fn lerp_hsv(a: Color, b: Color, t: f32) -> Color {
    let (mut h0, s0, v0) = rgb_to_hsv(a);
    let (mut h1, s1, v1) = rgb_to_hsv(b);

    // Hue is undefined at negligible chroma; borrow the other endpoint's hue
    // to avoid spurious tints.
    const CHROMA_EPSILON: f32 = 1e-6;
    if s0 * v0 <= CHROMA_EPSILON {
        h0 = h1;
    }
    if s1 * v1 <= CHROMA_EPSILON {
        h1 = h0;
    }

    let mut h_delta = h1 - h0;
    if h_delta > 0.5 {
        h_delta -= 1.0;
    } else if h_delta < -0.5 {
        h_delta += 1.0;
    }
    hsv(
        h0 + h_delta * t,
        s0 + (s1 - s0) * t,
        v0 + (v1 - v0) * t,
        a.a + (b.a - a.a) * t,
    )
}

fn linearized_color_channel(channel: f32) -> f32 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.039_28 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of `color`, in `[0, 1]`. Port of
/// `relativeLuminance()` (color.cpp:158-162).
pub fn relative_luminance(color: Color) -> f32 {
    0.2126 * linearized_color_channel(color.r)
        + 0.7152 * linearized_color_channel(color.g)
        + 0.0722 * linearized_color_channel(color.b)
}

/// Returns opaque black or white, whichever reads better on `background`.
/// Port of `readableTextColorForBackground()` (color.cpp:164-166).
pub fn readable_text_color_for_background(background: Color) -> Color {
    if relative_luminance(background) > 0.179 {
        rgba(0.0, 0.0, 0.0, 1.0)
    } else {
        rgba(1.0, 1.0, 1.0, 1.0)
    }
}

/// Formats as `"#RRGGBB"` (alpha dropped). Port of `formatRgbHex()`
/// (color.cpp:168-174).
pub fn format_rgb_hex(color: Color) -> String {
    fn to_byte(channel: f32) -> u8 {
        (channel.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    format!(
        "#{:02X}{:02X}{:02X}",
        to_byte(color.r),
        to_byte(color.g),
        to_byte(color.b)
    )
}

/// C's `isspace` under the "C" locale: space, `\t`, `\n`, `\v`, `\f`, `\r`.
/// Deliberately narrower than Rust's `char::is_ascii_whitespace` (which
/// excludes `\v`) and than `str::trim` (which is Unicode-whitespace-aware) —
/// used everywhere this module trims input the way `StringUtils::trim`/the
/// inline `isspace` loops in `color.cpp` do, so a stray `\v` in a config
/// string round-trips identically to the C++.
fn is_c_isspace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')
}

pub(crate) fn trim_c_whitespace(s: &str) -> &str {
    s.trim_matches(is_c_isspace)
}

/// Like [`hex`] but non-throwing (returns `None`), trims whitespace, and
/// tolerates a missing `'#'`. Port of `tryParseHexColor()` (color.cpp:176-198).
pub fn try_parse_hex_color(input: &str) -> Option<Color> {
    let trimmed = trim_c_whitespace(input);
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if trimmed.starts_with('#') {
        trimmed.to_string()
    } else {
        format!("#{trimmed}")
    };
    hex(&normalized).ok()
}

/// Parses a CSS-style color string, returning `None` if it is not a color.
/// Accepts `"#rgb[a]"`/`"#rrggbb[aa]"` (literal `'#'` required), plus
/// `rgb()`/`rgba()`/`hsl()`/`hsla()` in either comma-separated (legacy) or
/// space-separated (CSS Color 4, with `/` alpha) form. Channels take number
/// or percentage; hue takes an optional `deg`/`grad`/`rad`/`turn` unit. Port
/// of `tryParseCssColor()` (color.cpp:200-406).
pub fn try_parse_css_color(text: &str) -> Option<Color> {
    let trimmed = trim_c_whitespace(text);
    if trimmed.is_empty() {
        return None;
    }

    // Hex requires a literal '#'. Use `hex()` rather than
    // `try_parse_hex_color()`: the latter inserts a missing '#', which would
    // treat plain words like "facade" as colors.
    if let Some(rest) = trimmed.strip_prefix('#') {
        return hex(&format!("#{rest}")).ok();
    }

    let lower = trimmed.to_ascii_lowercase();
    let lv = lower.as_str();

    let is_rgba = lv.starts_with("rgba(") && lv.ends_with(')');
    let is_rgb = !is_rgba && lv.starts_with("rgb(") && lv.ends_with(')');
    let is_hsla = lv.starts_with("hsla(") && lv.ends_with(')');
    let is_hsl = !is_hsla && lv.starts_with("hsl(") && lv.ends_with(')');

    if is_rgb || is_rgba {
        let prefix_len = if is_rgba { 5 } else { 4 };
        let inner = &lv[prefix_len..lv.len() - 1];
        return parse_rgb_inner(inner);
    }

    if is_hsl || is_hsla {
        let prefix_len = if is_hsla { 5 } else { 4 };
        let inner = &lv[prefix_len..lv.len() - 1];
        return parse_hsl_inner(inner);
    }

    None
}

fn skip_spaces(sv: &str) -> &str {
    sv.trim_start_matches(' ')
}

/// Between components: an optional comma (legacy) or just whitespace (CSS
/// Color 4).
fn consume_separator(sv: &str) -> &str {
    let sv = skip_spaces(sv);
    let sv = sv.strip_prefix(',').unwrap_or(sv);
    skip_spaces(sv)
}

/// Before alpha: a comma (legacy) or a slash (CSS Color 4) is required.
fn consume_alpha_separator(sv: &str) -> Option<&str> {
    let sv = skip_spaces(sv);
    let rest = sv.strip_prefix(',').or_else(|| sv.strip_prefix('/'))?;
    Some(skip_spaces(rest))
}

/// Parses a leading floating-point number, returning the value and the
/// remaining unconsumed text. Port of the `std::from_chars` calls scattered
/// through `tryParseCssColor` (color.cpp): unlike `std::from_chars`, Rust has
/// no direct "parse a float prefix" primitive, so this scans the longest
/// valid-looking numeric prefix by hand and defers to `str::parse`.
///
/// Two deliberate divergences from a naive port, both checked against a
/// compiled `std::from_chars` probe rather than assumed:
/// - No leading `+` on the mantissa: `std::from_chars`'s float grammar
///   (cppreference: "a plus sign is not permitted, other than as an exponent
///   sign") rejects `"+1.5"` outright, so this only accepts a leading `-`
///   here — a leading `+` still gets left in `rest` and fails whatever
///   consumes it next, same end result as the C++ rejecting the whole
///   component. The exponent's own sign (`"1e+3"`) still accepts `+`, since
///   that's part of the same grammar C++ accepts.
/// - `"nan"`/`"inf"`/`"infinity"` are never matched (no digit ever seen, so
///   `saw_digit` stays false): `std::from_chars` parses these as valid
///   floats and the C++'s subsequent `v < 0 || v > 255`-style range checks
///   don't reject NaN (a NaN comparison is always false), so a channel
///   token of `"nan"` silently produces a `NaN` channel value in the C++.
///   Not replicated here — nothing in this crate's ported call sites
///   (`colorSpecFromConfigString` only ever calls `tryParseHexColor`, never
///   `tryParseCssColor`) depends on that behavior, and there's no test in
///   the C++ suite locking it in either.
fn parse_leading_f32(sv: &str) -> Option<(f32, &str)> {
    let bytes = sv.as_bytes();
    let mut end = 0;
    if end < bytes.len() && bytes[end] == b'-' {
        end += 1;
    }
    let mut saw_digit = false;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        saw_digit = true;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            saw_digit = true;
        }
    }
    if !saw_digit {
        return None;
    }
    // Exponent, e.g. "1e3" or "1e+3" — `std::from_chars` for float accepts
    // both signs here even though it rejects a leading `+` on the mantissa.
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut exp_end = end + 1;
        if exp_end < bytes.len() && (bytes[exp_end] == b'+' || bytes[exp_end] == b'-') {
            exp_end += 1;
        }
        let exp_digits_start = exp_end;
        while exp_end < bytes.len() && bytes[exp_end].is_ascii_digit() {
            exp_end += 1;
        }
        if exp_end > exp_digits_start {
            end = exp_end;
        }
    }
    let value: f32 = sv[..end].parse().ok()?;
    Some((value, &sv[end..]))
}

/// Channel: number in `[0, 255]` or percentage in `[0, 100]%`, normalized to
/// `[0, 1]`.
fn parse_channel(sv: &str) -> Option<(f32, &str)> {
    let sv = skip_spaces(sv);
    let (v, rest) = parse_leading_f32(sv)?;
    let (result, rest) = if let Some(rest) = rest.strip_prefix('%') {
        if !(0.0..=100.0).contains(&v) {
            return None;
        }
        (v / 100.0, rest)
    } else {
        if !(0.0..=255.0).contains(&v) {
            return None;
        }
        (v / 255.0, rest)
    };
    Some((result, skip_spaces(rest)))
}

/// Alpha: number in `[0, 1]` or percentage in `[0, 100]%`.
fn parse_alpha(sv: &str) -> Option<(f32, &str)> {
    let sv = skip_spaces(sv);
    let (v, rest) = parse_leading_f32(sv)?;
    let (result, rest) = if let Some(rest) = rest.strip_prefix('%') {
        (v / 100.0, rest)
    } else {
        (v, rest)
    };
    if !(0.0..=1.0).contains(&result) {
        return None;
    }
    Some((result, skip_spaces(rest)))
}

fn parse_rgb_inner(inner: &str) -> Option<Color> {
    let (r, rest) = parse_channel(inner)?;
    let rest = consume_separator(rest);
    let (g, rest) = parse_channel(rest)?;
    let rest = consume_separator(rest);
    let (b, rest) = parse_channel(rest)?;
    let rest = skip_spaces(rest);

    let (a, rest) = if rest.is_empty() {
        (1.0, rest)
    } else {
        let rest = consume_alpha_separator(rest)?;
        parse_alpha(rest)?
    };
    if !rest.is_empty() {
        return None;
    }
    Some(rgba(r, g, b, a))
}

/// Hue: number with an optional angle unit; result in degrees (`hsl()` wraps
/// the range).
fn parse_hue(sv: &str) -> Option<(f32, &str)> {
    let sv = skip_spaces(sv);
    let (mut v, rest) = parse_leading_f32(sv)?;
    let rest = if let Some(rest) = rest.strip_prefix("deg") {
        rest
    } else if let Some(rest) = rest.strip_prefix("grad") {
        v *= 360.0 / 400.0;
        rest
    } else if let Some(rest) = rest.strip_prefix("rad") {
        v *= 180.0 / std::f32::consts::PI;
        rest
    } else if let Some(rest) = rest.strip_prefix("turn") {
        v *= 360.0;
        rest
    } else {
        rest
    };
    Some((v, skip_spaces(rest)))
}

/// Saturation/lightness: percentage in `[0, 100]%`, normalized to `[0, 1]`.
fn parse_percent(sv: &str) -> Option<(f32, &str)> {
    let sv = skip_spaces(sv);
    let (v, rest) = parse_leading_f32(sv)?;
    let rest = rest.strip_prefix('%')?;
    if !(0.0..=100.0).contains(&v) {
        return None;
    }
    Some((v / 100.0, skip_spaces(rest)))
}

fn parse_hsl_inner(inner: &str) -> Option<Color> {
    let (h, rest) = parse_hue(inner)?;
    let rest = consume_separator(rest);
    let (s, rest) = parse_percent(rest)?;
    let rest = consume_separator(rest);
    let (l, rest) = parse_percent(rest)?;
    let rest = skip_spaces(rest);

    let (a, rest) = if rest.is_empty() {
        (1.0, rest)
    } else {
        let rest = consume_alpha_separator(rest)?;
        parse_alpha(rest)?
    };
    if !rest.is_empty() {
        return None;
    }
    Some(hsl(h, s, l, a))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hex_extracts_channels() {
        let c = rgb_hex(0x1A2B3C);
        assert_eq!(c.r, color_byte(0x1A));
        assert_eq!(c.g, color_byte(0x2B));
        assert_eq!(c.b, color_byte(0x3C));
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn rgba_hex_extracts_channels_including_alpha() {
        let c = rgba_hex(0x1A2B3C4D);
        assert_eq!(c.r, color_byte(0x1A));
        assert_eq!(c.g, color_byte(0x2B));
        assert_eq!(c.b, color_byte(0x3C));
        assert_eq!(c.a, color_byte(0x4D));
    }

    #[test]
    fn hex_requires_hash_prefix() {
        assert_eq!(hex("fff"), Err(HexColorError::MissingHash));
        assert_eq!(hex(""), Err(HexColorError::MissingHash));
    }

    #[test]
    fn hex_rejects_bad_digit() {
        assert_eq!(hex("#fgf"), Err(HexColorError::InvalidDigit));
    }

    #[test]
    fn hex_rejects_unsupported_length() {
        assert_eq!(hex("#ff"), Err(HexColorError::UnsupportedFormat));
        assert_eq!(hex("#fffff"), Err(HexColorError::UnsupportedFormat));
    }

    #[test]
    fn hex_3_digit_expands_by_repetition() {
        assert_eq!(hex("#fff").unwrap(), rgba(1.0, 1.0, 1.0, 1.0));
        assert_eq!(hex("#000").unwrap(), rgba(0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn hex_4_digit_expands_alpha_too() {
        let c = hex("#0f08").unwrap();
        assert_eq!(c, rgba(0.0, color_byte(0xFF), 0.0, color_byte(0x88)));
    }

    // Representative literal hex strings actually used as builtin-palette
    // constants in src/theme/builtin_palettes.cpp (example.toml itself has no
    // literal hex color values — only role tokens/comments — so this is the
    // closest thing to "hex strings from the config surface").
    #[test]
    fn hex_6_and_8_digit_round_trip_builtin_palette_colors() {
        for &(raw, r, g, b) in &[
            ("#E6B450", 0xE6, 0xB4, 0x50),
            ("#0B0E14", 0x0B, 0x0E, 0x14),
            ("#AAD94C", 0xAA, 0xD9, 0x4C),
            ("#39BAE6", 0x39, 0xBA, 0xE6),
            ("#D95757", 0xD9, 0x57, 0x57),
        ] {
            let c = hex(raw).expect("valid 6-digit hex");
            assert_eq!(c, rgb_hex((r << 16) | (g << 8) | b));
            assert_eq!(format_rgb_hex(c), raw);

            let raw8 = format!("{raw}CC");
            let c8 = hex(&raw8).expect("valid 8-digit hex");
            assert_eq!(c8, rgba_hex(((r << 16) | (g << 8) | b) << 8 | 0xCC));
        }
    }

    #[test]
    fn with_alpha_clamps() {
        let c = with_alpha(rgba(0.1, 0.2, 0.3, 1.0), 5.0);
        assert_eq!(c.a, 1.0);
        let c = with_alpha(rgba(0.1, 0.2, 0.3, 1.0), -5.0);
        assert_eq!(c.a, 0.0);
    }

    #[test]
    fn brighten_scales_and_clamps_rgb_only() {
        let c = brighten(rgba(0.5, 0.5, 0.5, 0.4), 3.0);
        assert_eq!(c.r, 1.0);
        assert_eq!(c.g, 1.0);
        assert_eq!(c.b, 1.0);
        assert_eq!(c.a, 0.4);
    }

    #[test]
    fn lerp_color_interpolates_every_channel() {
        let a = rgba(0.0, 0.0, 0.0, 0.0);
        let b = rgba(1.0, 1.0, 1.0, 1.0);
        assert_eq!(lerp_color(a, b, 0.25), rgba(0.25, 0.25, 0.25, 0.25));
    }

    #[test]
    fn hsv_primary_hues_match_pure_channels() {
        assert_eq!(hsv(0.0, 1.0, 1.0, 1.0), rgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(hsv(1.0 / 3.0, 1.0, 1.0, 1.0), rgba(0.0, 1.0, 0.0, 1.0));
        assert_eq!(hsv(2.0 / 3.0, 1.0, 1.0, 1.0), rgba(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn hsv_zero_saturation_is_gray() {
        let c = hsv(0.3, 0.0, 0.7, 1.0);
        assert_eq!(c.r, 0.7);
        assert_eq!(c.g, 0.7);
        assert_eq!(c.b, 0.7);
    }

    #[test]
    fn hsl_primary_hues_match_pure_channels() {
        assert_eq!(hsl(0.0, 1.0, 0.5, 1.0), rgba(1.0, 0.0, 0.0, 1.0));
        assert_eq!(hsl(120.0, 1.0, 0.5, 1.0), rgba(0.0, 1.0, 0.0, 1.0));
        assert_eq!(hsl(240.0, 1.0, 0.5, 1.0), rgba(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn hsl_negative_hue_wraps() {
        assert_eq!(hsl(-360.0, 1.0, 0.5, 1.0), hsl(0.0, 1.0, 0.5, 1.0));
    }

    #[test]
    fn rgb_to_hsv_round_trips_through_hsv() {
        let original = rgba(0.2, 0.6, 0.9, 1.0);
        let (h, s, v) = rgb_to_hsv(original);
        let round_tripped = hsv(h, s, v, 1.0);
        assert!((round_tripped.r - original.r).abs() < 1e-5);
        assert!((round_tripped.g - original.g).abs() < 1e-5);
        assert!((round_tripped.b - original.b).abs() < 1e-5);
    }

    #[test]
    fn rgb_to_hsv_black_has_zero_hue_and_saturation() {
        assert_eq!(rgb_to_hsv(rgba(0.0, 0.0, 0.0, 1.0)), (0.0, 0.0, 0.0));
    }

    #[test]
    fn lerp_hsv_endpoints_match_inputs() {
        let a = rgba(1.0, 0.0, 0.0, 1.0);
        let b = rgba(0.0, 0.0, 1.0, 1.0);
        let at_0 = lerp_hsv(a, b, 0.0);
        let at_1 = lerp_hsv(a, b, 1.0);
        assert!((at_0.r - a.r).abs() < 1e-5 && (at_0.b - a.b).abs() < 1e-5);
        assert!((at_1.r - b.r).abs() < 1e-5 && (at_1.b - b.b).abs() < 1e-5);
    }

    #[test]
    fn lerp_hsv_takes_shortest_hue_path() {
        // Red (hue 0) to magenta (hue ~0.833 turns / 300deg): going backward
        // through violet (shortest path) should never pass through green/cyan.
        let red = hsv(0.0, 1.0, 1.0, 1.0);
        let magenta = hsv(300.0 / 360.0, 1.0, 1.0, 1.0);
        let mid = lerp_hsv(red, magenta, 0.5);
        assert!(
            mid.g < 0.1,
            "shortest hue path from red to magenta shouldn't pass through green: {mid:?}"
        );
    }

    #[test]
    fn relative_luminance_black_and_white() {
        assert_eq!(relative_luminance(rgba(0.0, 0.0, 0.0, 1.0)), 0.0);
        assert!((relative_luminance(rgba(1.0, 1.0, 1.0, 1.0)) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn readable_text_color_picks_black_on_light_white_on_dark() {
        assert_eq!(
            readable_text_color_for_background(rgba(1.0, 1.0, 1.0, 1.0)),
            rgba(0.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            readable_text_color_for_background(rgba(0.0, 0.0, 0.0, 1.0)),
            rgba(1.0, 1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn format_rgb_hex_drops_alpha_and_uppercases() {
        assert_eq!(format_rgb_hex(rgba(1.0, 0.0, 0.5019608, 0.1)), "#FF0080");
    }

    #[test]
    fn try_parse_hex_color_tolerates_missing_hash_and_whitespace() {
        assert_eq!(
            try_parse_hex_color("  fff  "),
            Some(rgba(1.0, 1.0, 1.0, 1.0))
        );
        assert_eq!(try_parse_hex_color("#000"), Some(rgba(0.0, 0.0, 0.0, 1.0)));
        assert_eq!(try_parse_hex_color("not-a-color"), None);
        assert_eq!(try_parse_hex_color(""), None);
        assert_eq!(try_parse_hex_color("   "), None);
    }

    #[test]
    fn try_parse_hex_color_respects_c_isspace_not_rust_ascii_whitespace() {
        // '\x0B' (vertical tab) is C isspace but NOT Rust's
        // char::is_ascii_whitespace — this specifically exercises that gap.
        assert_eq!(
            try_parse_hex_color("\x0B#fff\x0B"),
            Some(rgba(1.0, 1.0, 1.0, 1.0))
        );
    }

    #[test]
    fn try_parse_css_color_rejects_bare_words_without_hash() {
        // "facade" would parse as a color under try_parse_hex_color's
        // missing-'#'-tolerance; tryParseCssColor must not make that mistake.
        assert_eq!(try_parse_css_color("facade"), None);
    }

    #[test]
    fn try_parse_css_color_hex_forms() {
        assert_eq!(try_parse_css_color("#fff"), Some(rgba(1.0, 1.0, 1.0, 1.0)));
        assert_eq!(
            try_parse_css_color("#000000"),
            Some(rgba(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn try_parse_css_color_rgb_legacy_comma_form() {
        assert_eq!(
            try_parse_css_color("rgb(255, 0, 128)"),
            Some(rgba(1.0, 0.0, color_byte(128), 1.0))
        );
    }

    #[test]
    fn try_parse_css_color_rgba_legacy_comma_form_with_alpha() {
        let c = try_parse_css_color("rgba(255, 0, 128, 0.5)").expect("valid rgba");
        assert_eq!(c.r, 1.0);
        assert_eq!(c.b, color_byte(128));
        assert_eq!(c.a, 0.5);
    }

    #[test]
    fn try_parse_css_color_rgb_css4_space_slash_form() {
        let c = try_parse_css_color("rgb(255 0 128 / 50%)").expect("valid CSS4 rgb");
        assert_eq!(c.r, 1.0);
        assert_eq!(c.a, 0.5);
    }

    #[test]
    fn try_parse_css_color_rgb_percentage_channels() {
        assert_eq!(
            try_parse_css_color("rgb(100%, 0%, 50%)"),
            Some(rgba(1.0, 0.0, 0.5, 1.0))
        );
    }

    #[test]
    fn try_parse_css_color_rgb_out_of_range_channel_fails() {
        assert_eq!(try_parse_css_color("rgb(256, 0, 0)"), None);
        assert_eq!(try_parse_css_color("rgb(101%, 0%, 0%)"), None);
    }

    #[test]
    fn try_parse_css_color_hsl_legacy_and_css4_forms() {
        assert_eq!(
            try_parse_css_color("hsl(0, 100%, 50%)"),
            Some(rgba(1.0, 0.0, 0.0, 1.0))
        );
        let c = try_parse_css_color("hsla(0 100% 50% / 50%)").expect("valid hsla");
        assert_eq!(c.r, 1.0);
        assert_eq!(c.a, 0.5);
    }

    #[test]
    fn try_parse_css_color_hsl_hue_units() {
        let base = try_parse_css_color("hsl(120deg, 100%, 50%)").expect("deg");
        let turn = try_parse_css_color("hsl(0.3333333turn, 100%, 50%)").expect("turn");
        assert!((turn.g - base.g).abs() < 1e-4);
        let grad = try_parse_css_color("hsl(133.33333grad, 100%, 50%)").expect("grad");
        assert!((grad.g - base.g).abs() < 1e-3);
    }

    #[test]
    fn try_parse_css_color_trailing_garbage_fails() {
        // Trailing text outside the parens fails the outer starts_with/ends_with
        // check entirely (same as the C++'s isRgb/isRgba/isHsl/isHsla gate).
        assert_eq!(try_parse_css_color("rgb(255, 0, 128) extra"), None);
        // Trailing text *inside* the parens, after the last recognized
        // component, exercises the inner "rest must be fully consumed" check.
        assert_eq!(try_parse_css_color("rgb(255, 0, 128 extra)"), None);
        assert_eq!(try_parse_css_color("hsl(0, 100%, 50%, extra)"), None);
    }

    #[test]
    fn try_parse_css_color_unknown_function_fails() {
        assert_eq!(try_parse_css_color("lab(50% 40 59.5)"), None);
    }

    #[test]
    fn try_parse_css_color_rejects_leading_plus_on_mantissa() {
        // std::from_chars's float grammar rejects a leading '+' outside the
        // exponent; a naive port using Rust's plus-tolerant str::parse would
        // wrongly accept these.
        assert_eq!(try_parse_css_color("rgb(+128, 0, 0)"), None);
        assert_eq!(try_parse_css_color("hsl(+120, 100%, 50%)"), None);
    }

    #[test]
    fn try_parse_css_color_accepts_leading_plus_in_exponent() {
        // Unlike the mantissa sign above, std::from_chars does accept a '+'
        // exponent sign (e.g. "1e+2"), and so does this parser.
        assert_eq!(
            try_parse_css_color("rgb(1e+2, 0, 0)"),
            try_parse_css_color("rgb(100, 0, 0)")
        );
    }

    #[test]
    fn try_parse_css_color_rejects_nan_and_inf_tokens() {
        // Deliberate divergence from std::from_chars (which parses these as
        // valid floats, letting a NaN channel silently through the C++'s
        // range checks): documented on parse_leading_f32, not replicated
        // here since no ported call site depends on it.
        assert_eq!(try_parse_css_color("rgb(nan, 0, 0)"), None);
        assert_eq!(try_parse_css_color("rgb(inf, 0, 0)"), None);
        assert_eq!(try_parse_css_color("hsl(infinity, 100%, 50%)"), None);
    }
}
