//! Port of `src/i18n/i18n.{h,cpp}`.
//!
//! C++'s `tr`/`trp` take a variadic `Args&&...` pack of alternating name/value
//! pairs (`i18n::tr("key", "name", value, ...)`), dispatched at compile time
//! through `argToString`'s `if constexpr`. Rust has no variadic templates, so
//! this takes the pairs as a `&[(&str, String)]` slice instead — callers format
//! their own values (`value.to_string()`/`format!(...)`) rather than leaning on
//! an `argToString`-equivalent trait dispatch. No real call site exists yet
//! (the UI phases that call `i18n::tr` port much later), so this is the
//! straightforward Rust shape rather than a speculative macro; revisit the
//! ergonomics once the first real caller lands.

use super::service;

/// Converts a snake_case config/wire identifier into the dash-case segment used
/// in translation keys (e.g. `"art_size"` → `"art-size"`).
pub fn key_segment(id: &str) -> String {
    id.replace('_', "-")
}

/// Single-pass scanner: copies `tmpl` through, and on `{` looks for the matching
/// `}` and substitutes the named value if present in `args`. A name with no
/// match, or an unterminated `{`, is left in the output verbatim so mismatches
/// stay visible on screen. Linear search over `args` beats a map: `args` is
/// always small (typically ≤ 3 entries).
fn interpolate(tmpl: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(tmpl.len());
    let mut rest = tmpl;

    while let Some(brace_pos) = rest.find('{') {
        out.push_str(&rest[..brace_pos]);
        let after_brace = &rest[brace_pos + 1..];

        let Some(close_pos) = after_brace.find('}') else {
            out.push_str(&rest[brace_pos..]);
            rest = "";
            break;
        };

        let name = &after_brace[..close_pos];
        match args.iter().find(|(arg_name, _)| *arg_name == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after_brace[close_pos + 1..];
    }

    out.push_str(rest);
    out
}

/// Looks up `key` in the active translation catalog and substitutes `args`
/// (alternating name/value pairs in the C++; a slice of pairs here — see the
/// module doc comment). A missing or empty translation renders as `"!!key!!"`
/// so untranslated strings are visible.
pub fn tr(key: &str, args: &[(&str, String)]) -> String {
    let raw = service::lookup(key).unwrap_or_default();
    if raw.is_empty() {
        return format!("!!{key}!!");
    }
    if args.is_empty() {
        raw
    } else {
        interpolate(&raw, args)
    }
}

/// Pluralized lookup: `count == 1` uses `key` as-is, anything else uses
/// `"<key>-plural"`. Either way `"count"` is added to `args` as a formatted
/// name/value pair.
pub fn trp(key: &str, count: i64, args: &[(&str, String)]) -> String {
    let mut full_args = Vec::with_capacity(args.len() + 1);
    full_args.push(("count", count.to_string()));
    full_args.extend_from_slice(args);

    if count == 1 {
        tr(key, &full_args)
    } else {
        let plural_key = format!("{key}-plural");
        tr(&plural_key, &full_args)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn key_segment_replaces_underscores_with_dashes() {
        assert_eq!(key_segment("art_size"), "art-size");
        assert_eq!(key_segment("already-dashed"), "already-dashed");
        assert_eq!(key_segment(""), "");
    }

    #[test]
    fn interpolate_substitutes_known_names() {
        let args = [("name", "top".to_string())];
        assert_eq!(
            interpolate("entities.bar.label: {name}", &args),
            "entities.bar.label: top"
        );
    }

    #[test]
    fn interpolate_leaves_unknown_names_literal() {
        let args: [(&str, String); 0] = [];
        assert_eq!(interpolate("hello {name}", &args), "hello {name}");
    }

    #[test]
    fn interpolate_appends_unterminated_brace_verbatim() {
        let args: [(&str, String); 0] = [];
        assert_eq!(interpolate("broken {name", &args), "broken {name");
    }

    #[test]
    fn interpolate_handles_multiple_substitutions() {
        let args = [("a", "1".to_string()), ("b", "2".to_string())];
        assert_eq!(interpolate("{a}-{b}-{a}", &args), "1-2-1");
    }

    #[test]
    fn tr_renders_missing_key_as_bang_bang() {
        assert_eq!(
            tr("definitely.not.a.real.key", &[]),
            "!!definitely.not.a.real.key!!"
        );
    }

    #[test]
    fn trp_appends_count_and_picks_plural_key_form() {
        // No catalog defines this key either way, so both resolve through the
        // missing-key path — but with different rendered keys, proving the
        // singular/plural key selection itself (independent of catalog
        // contents).
        assert_eq!(
            trp("definitely.not.a.real.key", 1, &[]),
            "!!definitely.not.a.real.key!!"
        );
        assert_eq!(
            trp("definitely.not.a.real.key", 2, &[]),
            "!!definitely.not.a.real.key-plural!!"
        );
    }
}
