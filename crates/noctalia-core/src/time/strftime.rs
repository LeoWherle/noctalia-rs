//! Port of the formatting-engine half of `src/time/time_format.cpp`:
//! `normalizeFormatEscapes`, `shouldUseStrftimeCompat`, `formatStrftimeRaw`,
//! `formatStrftimeWithUnixSeconds`, `formatStrftimeCompat`, and the public
//! `formatStrftime`. See [`super`]'s module doc comment for why this is
//! built on `libc::strftime` rather than `jiff`'s formatter.

use std::ffi::CString;

/// Port of `formatStrftime`: a direct `strftime` call with no brace-compat
/// bridging and no `%s` substitution (real callers of this one — e.g.
/// `calendar_tab.cpp`'s `"%B"`/`"%a"` — never pass either).
pub fn format_strftime(fmt: &str, tm: &libc::tm) -> String {
    format_strftime_raw(fmt, tm)
}

/// Port of `normalizeFormatEscapes`: turns the literal two-character escape
/// `\n` (as typed in a config string) into a real newline. Non-overlapping
/// left-to-right replacement, same as the C++'s manual scan.
pub(super) fn normalize_format_escapes(fmt: &str) -> String {
    fmt.replace("\\n", "\n")
}

/// Port of `shouldUseStrftimeCompat`.
fn should_use_strftime_compat(fmt: &str) -> bool {
    fmt.contains("%-") || (fmt.contains('%') && (!fmt.contains('{') || fmt.contains("{:")))
}

/// Port of `formatStrftimeRaw`: grows the output buffer geometrically (six
/// attempts, matching the C++) until `strftime` reports a non-empty write,
/// or gives up and returns an empty string.
pub(super) fn format_strftime_raw(fmt: &str, tm: &libc::tm) -> String {
    // `fmt` reaching here with an interior NUL is unrepresentable as the
    // NUL-terminated C string `strftime` requires; no real caller does this
    // (config-provided format strings, never binary data).
    let Ok(c_fmt) = CString::new(fmt) else {
        return String::new();
    };

    let mut size = (fmt.len() * 4 + 16).max(64);
    for _ in 0..6 {
        let mut buf = vec![0u8; size];
        // SAFETY: `buf` is a valid, live, correctly-sized buffer; `c_fmt` is
        // NUL-terminated; `tm` is a valid, initialized `libc::tm` live for
        // the call. `strftime` only reads `c_fmt`/`tm` and writes at most
        // `buf.len()` bytes into `buf`.
        let written =
            unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c_fmt.as_ptr(), tm) };
        if written > 0 || fmt.is_empty() {
            buf.truncate(written);
            return String::from_utf8_lossy(&buf).into_owned();
        }
        size *= 2;
    }
    String::new()
}

/// Port of `formatStrftimeWithUnixSeconds`: scans for `%s` (leaving `%%`
/// and every other specifier untouched) and substitutes it with
/// `unix_seconds` directly, running everything else through
/// [`format_strftime_raw`] in chunks around the substitution points.
///
/// Rewritten from the C++'s byte-by-byte `chunk`-accumulation as a pure
/// slicing scan: every split point (`chunk_start`, `i`) lands on an ASCII
/// `%`/`s` byte, which is always a valid UTF-8 char boundary (continuation
/// bytes are never ASCII), so slicing `fmt` directly is equivalent and
/// avoids rebuilding the skipped text byte-by-byte.
pub(super) fn format_strftime_with_unix_seconds(
    fmt: &str,
    tm: &libc::tm,
    unix_seconds: Option<i64>,
) -> String {
    let Some(unix_seconds) = unix_seconds else {
        return format_strftime_raw(fmt, tm);
    };
    if !fmt.contains("%s") {
        return format_strftime_raw(fmt, tm);
    }

    let bytes = fmt.as_bytes();
    let mut out = String::new();
    let mut chunk_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'%' || i + 1 >= bytes.len() {
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            b'%' => i += 2,
            b's' => {
                out.push_str(&format_strftime_raw(&fmt[chunk_start..i], tm));
                out.push_str(&unix_seconds.to_string());
                i += 2;
                chunk_start = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&format_strftime_raw(&fmt[chunk_start..], tm));
    out
}

/// Port of `formatStrftimeCompat`: unwraps a single `{: ... }` chrono-style
/// wrapper (with `{{`/`}}` literal-brace escaping) down to its inner
/// `%`-spec, or treats `fmt` as a bare `strftime` pattern directly. Returns
/// `None` when `fmt` doesn't look like a `strftime` pattern at all (no `%`)
/// — see [`super`]'s module doc comment for why nothing consumes that case
/// today.
pub(super) fn format_strftime_compat(
    fmt: &str,
    tm: &libc::tm,
    unix_seconds: Option<i64>,
) -> Option<String> {
    if !should_use_strftime_compat(fmt) {
        return None;
    }
    if !fmt.contains('{') {
        return Some(format_strftime_with_unix_seconds(fmt, tm, unix_seconds));
    }

    let mut out = String::new();
    let mut formatted_field = false;
    let bytes = fmt.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            out.push('{');
            i += 2;
            continue;
        }
        if bytes[i] == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
            out.push('}');
            i += 2;
            continue;
        }
        if bytes[i] != b'{' {
            // `i` is a char boundary on entry to every iteration (`{{`/`}}`
            // advance by 2 ASCII bytes; the `{`-field branch below advances
            // past an ASCII `{` then to a `str::find`-returned boundary), so
            // stepping one *character* here preserves that invariant.
            let ch = fmt[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let close = fmt[i + 1..].find('}')?;
        let end = i + 1 + close;
        let field = &fmt[i + 1..end];
        let colon = field.find(':')?;
        let spec_full = &field[colon + 1..];
        let first_percent = spec_full.find('%')?;
        let spec = &spec_full[first_percent..];
        out.push_str(&format_strftime_with_unix_seconds(spec, tm, unix_seconds));
        formatted_field = true;
        i = end + 1;
    }

    formatted_field.then_some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tm_for(year: i32, month: i32, day: i32, hour: i32, min: i32, sec: i32) -> libc::tm {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = year - 1900;
        tm.tm_mon = month - 1;
        tm.tm_mday = day;
        tm.tm_hour = hour;
        tm.tm_min = min;
        tm.tm_sec = sec;
        tm
    }

    #[test]
    fn normalize_format_escapes_converts_backslash_n() {
        assert_eq!(normalize_format_escapes(r"a\nb"), "a\nb");
        assert_eq!(normalize_format_escapes(r"a\\nb"), "a\\\nb");
        assert_eq!(normalize_format_escapes("plain"), "plain");
    }

    #[test]
    fn should_use_strftime_compat_matches_c_plus_plus_gate() {
        assert!(should_use_strftime_compat("%H:%M"));
        assert!(should_use_strftime_compat("{:%H:%M}"));
        assert!(should_use_strftime_compat("%-I"));
        assert!(!should_use_strftime_compat("{name}"));
        assert!(!should_use_strftime_compat("plain text"));
    }

    #[test]
    fn format_strftime_raw_renders_a_bare_pattern() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(format_strftime_raw("%Y-%m-%d", &tm), "2026-05-09");
    }

    #[test]
    fn format_strftime_raw_supports_gnu_no_pad_flag() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(format_strftime_raw("%-d", &tm), "9");
    }

    #[test]
    fn format_strftime_with_unix_seconds_substitutes_percent_s() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(
            format_strftime_with_unix_seconds("%s", &tm, Some(1_700_000_000)),
            "1700000000"
        );
        assert_eq!(
            format_strftime_with_unix_seconds("recording_%s.wav", &tm, Some(1_700_000_000)),
            "recording_1700000000.wav"
        );
    }

    #[test]
    fn format_strftime_with_unix_seconds_keeps_escaped_percent_literal() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(
            format_strftime_with_unix_seconds("%%s_%s", &tm, Some(1_700_000_000)),
            "%s_1700000000"
        );
    }

    #[test]
    fn format_strftime_with_unix_seconds_skips_substitution_without_a_value() {
        // No `unix_seconds` means an early return straight to
        // `format_strftime_raw`, whatever glibc's own (unreliable — see the
        // module doc comment) `%s` handling does with it; assert
        // self-consistency with that delegate rather than a magic literal.
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(
            format_strftime_with_unix_seconds("%s", &tm, None),
            format_strftime_raw("%s", &tm)
        );
    }

    #[test]
    fn format_strftime_compat_unwraps_a_brace_wrapped_spec() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(
            format_strftime_compat("{:%H:%M}", &tm, None),
            Some("06:23".to_string())
        );
    }

    #[test]
    fn format_strftime_compat_handles_literal_braces() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(
            format_strftime_compat("{{{:%H}}}", &tm, None),
            Some("{06}".to_string())
        );
    }

    #[test]
    fn format_strftime_compat_returns_none_for_a_pure_field_string() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(format_strftime_compat("{name}", &tm, None), None);
        assert_eq!(format_strftime_compat("plain text", &tm, None), None);
    }

    #[test]
    fn format_strftime_public_fn_matches_raw_engine() {
        let tm = tm_for(2026, 5, 9, 6, 23, 0);
        assert_eq!(format_strftime("%Y-%m-%d", &tm), "2026-05-09");
    }
}
