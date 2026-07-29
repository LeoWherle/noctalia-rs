//! IPC command argument parsing helpers.
//! Port of `src/ipc/ipc_arg_parse.h`.
//!
//! `src/ipc/ipc_arg_parse.h` builds on two generic helpers from `src/util/string_utils.h`
//! (`splitWhitespace`, `parseDotDecimal<float>`), which hasn't been ported yet — `string_utils.h`
//! is a ~30-function grab bag with no owning task of its own, out of scope for IPC protocol +
//! server. Following the precedent in `noctalia_core::process::systemd` (`generate_uuid_v4`,
//! task 1.6.5): the two functions this module actually needs are reimplemented privately below,
//! byte-for-byte matching the C++ (including its ASCII-only, `std::isspace`-in-the-"C"-locale
//! whitespace set, which is a strict subset of — and therefore not directly replaceable by —
//! Rust's Unicode-aware `str::trim`/`split_whitespace`), to be deleted in favor of a shared port
//! once `string_utils.h` lands for a real owning task.

/// Matches `std::isspace` in the (always-active, un-localized) "C" locale: space, tab, LF,
/// vertical tab, form feed, CR. Notably NOT the same set as Rust's `u8::is_ascii_whitespace`,
/// which excludes vertical tab (0x0B).
fn is_c_isspace(byte: u8) -> bool {
    matches!(byte, b' ' | 0x09 | 0x0a | 0x0b | 0x0c | 0x0d)
}

/// Port of `StringUtils::trim`, restricted to the one call site here needs (`parseDotDecimal`).
fn c_trim(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_c_isspace(bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && is_c_isspace(bytes[end - 1]) {
        end -= 1;
    }
    &text[start..end]
}

/// Port of `StringUtils::parseDotDecimal<float>`.
fn parse_dot_decimal_f32(text: &str) -> Option<f32> {
    let trimmed = c_trim(text);
    if trimmed.is_empty() {
        return None;
    }
    // std::from_chars (unlike Rust's f32::from_str) does not accept a leading '+' sign; reject
    // it here to match.
    if trimmed.starts_with('+') {
        return None;
    }
    let value: f32 = trimmed.parse().ok()?;
    value.is_finite().then_some(value)
}

/// Port of `StringUtils::splitWhitespace`, used here as `noctalia::ipc::splitWords`.
#[must_use]
pub fn split_words(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && is_c_isspace(bytes[i]) {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && !is_c_isspace(bytes[i]) {
            i += 1;
        }
        // Splits only occur at single-byte ASCII whitespace, so `start..i` always lands on a
        // UTF-8 char boundary even when the token itself contains multi-byte characters.
        result.push(text[start..i].to_string());
    }
    result
}

/// Parses `token` as either a normalized `[0, 1]` amount or a percentage (bare number or with a
/// trailing `%`), returning the normalized `[0, max_percent / 100]` value. Port of
/// `noctalia::ipc::parseNormalizedOrPercent`.
#[must_use]
pub fn parse_normalized_or_percent(token: &str, max_percent: f32) -> Option<f32> {
    let mut value = token.to_string();
    let mut is_percent = false;
    if value.ends_with('%') {
        is_percent = true;
        value.pop();
    }
    if value.is_empty() {
        return None;
    }

    let amount = parse_dot_decimal_f32(&value)?;

    if is_percent || !value.contains('.') {
        if amount < 0.0 || amount > max_percent {
            return None;
        }
        return Some(amount / 100.0);
    }

    if (0.0..=1.0).contains(&amount) {
        return Some(amount);
    }
    if amount > 1.0 && amount <= max_percent {
        return Some(amount / 100.0);
    }

    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn split_words_collapses_runs_and_trims_ends() {
        assert_eq!(
            split_words("  next \t prev\n"),
            vec!["next".to_string(), "prev".to_string()]
        );
        assert_eq!(split_words(""), Vec::<String>::new());
        assert_eq!(split_words("   "), Vec::<String>::new());
        assert_eq!(split_words("one"), vec!["one".to_string()]);
    }

    #[test]
    fn split_words_treats_vertical_tab_as_whitespace_like_c_isspace() {
        assert_eq!(
            split_words("a\x0bb"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn split_words_preserves_multibyte_tokens() {
        assert_eq!(split_words("héllo wörld"), vec!["héllo", "wörld"]);
    }

    #[test]
    fn parse_normalized_or_percent_accepts_bare_fraction() {
        assert_eq!(parse_normalized_or_percent("0.5", 100.0), Some(0.5));
    }

    #[test]
    fn parse_normalized_or_percent_treats_dotless_number_as_percent() {
        assert_eq!(parse_normalized_or_percent("50", 100.0), Some(0.5));
    }

    #[test]
    fn parse_normalized_or_percent_accepts_trailing_percent_sign() {
        assert_eq!(parse_normalized_or_percent("50%", 100.0), Some(0.5));
    }

    #[test]
    fn parse_normalized_or_percent_rejects_dotless_number_above_max() {
        assert_eq!(parse_normalized_or_percent("150", 100.0), None);
    }

    #[test]
    fn parse_normalized_or_percent_treats_out_of_unit_range_decimal_as_percent() {
        // Has a '.' and isn't marked with a trailing '%', so it skips the first branch; since
        // 1.5 isn't in [0, 1] it falls through to the second branch and is divided by 100 too.
        assert_eq!(parse_normalized_or_percent("1.5", 100.0), Some(0.015));
    }

    #[test]
    fn parse_normalized_or_percent_honors_custom_max_percent() {
        assert_eq!(parse_normalized_or_percent("120", 150.0), Some(1.2));
        assert_eq!(parse_normalized_or_percent("160", 150.0), None);
    }

    #[test]
    fn parse_normalized_or_percent_rejects_negative_and_empty_and_bare_percent() {
        assert_eq!(parse_normalized_or_percent("-5", 100.0), None);
        assert_eq!(parse_normalized_or_percent("", 100.0), None);
        assert_eq!(parse_normalized_or_percent("%", 100.0), None);
    }

    #[test]
    fn parse_normalized_or_percent_rejects_unparseable_and_leading_plus() {
        assert_eq!(parse_normalized_or_percent("abc", 100.0), None);
        // std::from_chars rejects a leading '+' unlike Rust's f32::from_str.
        assert_eq!(parse_normalized_or_percent("+5", 100.0), None);
    }
}
