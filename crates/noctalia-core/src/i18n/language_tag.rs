//! Port of `src/i18n/language_tag.{h,cpp}`.
//!
//! ASCII-only by construction, matching the C++'s `std::isalpha`/`std::tolower`
//! (cast through `unsigned char`, i.e. effectively "C" locale / ASCII) — language
//! tags are themselves ASCII (BCP-47), so this isn't a simplification, just the
//! same assumption stated more directly via `str::is_ascii_*`/`to_ascii_*`
//! instead of leaning on the C locale.

fn is_alpha(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_digit(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_digit())
}

fn lower(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn upper(value: &str) -> String {
    value.to_ascii_uppercase()
}

fn title(value: &str) -> String {
    let lowered = lower(value);
    let mut chars = lowered.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn is_script_subtag(subtag: &str) -> bool {
    subtag.len() == 4 && is_alpha(subtag)
}

fn is_region_subtag(subtag: &str) -> bool {
    (subtag.len() == 2 && is_alpha(subtag)) || (subtag.len() == 3 && is_digit(subtag))
}

fn split(tag: &str) -> Vec<&str> {
    tag.split('-').collect()
}

fn append_unique(out: &mut Vec<String>, value: String) {
    if value.is_empty() {
        return;
    }
    if !out.contains(&value) {
        out.push(value);
    }
}

struct ParsedTag {
    parts: Vec<String>,
    language: String,
    script: String,
    region: String,
    script_index: usize,
}

fn parse_tag(tag: &str) -> ParsedTag {
    let parts: Vec<String> = split(tag).into_iter().map(String::from).collect();
    let mut parsed = ParsedTag {
        parts,
        language: String::new(),
        script: String::new(),
        region: String::new(),
        script_index: 0,
    };
    if parsed.parts.is_empty() {
        return parsed;
    }

    parsed.language = parsed.parts[0].clone();
    for i in 1..parsed.parts.len() {
        let part = parsed.parts[i].clone();
        if parsed.script.is_empty() && is_script_subtag(&part) {
            parsed.script = part;
            parsed.script_index = i;
        } else if parsed.region.is_empty() && is_region_subtag(&part) {
            parsed.region = part;
        }
    }
    parsed
}

fn inferred_chinese_script(language: &str, region: &str) -> String {
    if language != "zh" {
        return String::new();
    }

    const SIMPLIFIED_REGIONS: [&str; 3] = ["CN", "SG", "MY"];
    const TRADITIONAL_REGIONS: [&str; 3] = ["TW", "HK", "MO"];
    if SIMPLIFIED_REGIONS.contains(&region) {
        return "Hans".to_string();
    }
    if TRADITIONAL_REGIONS.contains(&region) {
        return "Hant".to_string();
    }
    String::new()
}

fn with_inserted_script(parsed: &ParsedTag, script: &str) -> String {
    if script.is_empty() {
        return String::new();
    }
    let mut parts = parsed.parts.clone();
    parts.insert(1, script.to_string());
    parts.join("-")
}

fn without_script(parsed: &ParsedTag) -> String {
    if parsed.script_index == 0 {
        return String::new();
    }
    let mut parts = parsed.parts.clone();
    parts.remove(parsed.script_index);
    parts.join("-")
}

fn script_only(parsed: &ParsedTag, script: &str) -> String {
    if script.is_empty() {
        return String::new();
    }
    format!("{}-{script}", parsed.language)
}

/// Normalizes a raw locale/language identifier (POSIX locale form like
/// `zh_CN.UTF-8`, or a bare BCP-47-ish tag) into canonical `language-Script-REGION`
/// casing, or `""` if the input has no usable language (empty, `C`, `POSIX`, or a
/// malformed subtag).
pub fn normalize_language_tag(raw: &str) -> String {
    let mut stripped = raw.to_string();
    if let Some(pos) = stripped.find('.') {
        stripped.truncate(pos);
    }
    if let Some(pos) = stripped.find('@') {
        stripped.truncate(pos);
    }
    let stripped: String = stripped
        .chars()
        .map(|c| if c == '_' { '-' } else { c })
        .collect();

    if stripped.is_empty() || stripped == "C" || stripped == "POSIX" {
        return String::new();
    }

    let raw_parts = split(&stripped);
    if raw_parts.is_empty() || raw_parts[0].is_empty() {
        return String::new();
    }

    let mut parts: Vec<String> = Vec::with_capacity(raw_parts.len());
    parts.push(lower(raw_parts[0]));
    for part in &raw_parts[1..] {
        if part.is_empty() {
            return String::new();
        }
        if is_script_subtag(part) {
            parts.push(title(part));
        } else if is_region_subtag(part) {
            parts.push(upper(part));
        } else {
            parts.push(lower(part));
        }
    }

    parts.join("-")
}

/// Ordered list of catalog filenames (without extension) to try for `raw`, most
/// specific first — e.g. `zh_CN.UTF-8` → `["zh-Hans-CN", "zh-CN", "zh-Hans",
/// "zh"]`, inferring the Han script from the region when the tag doesn't name one
/// explicitly. Empty if `raw` has no usable language (see [`normalize_language_tag`]).
pub fn catalog_language_candidates(raw: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    let normalized = normalize_language_tag(raw);
    if normalized.is_empty() {
        return candidates;
    }

    let parsed = parse_tag(&normalized);
    if parsed.language.is_empty() {
        return candidates;
    }

    let mut script = parsed.script.clone();
    if script.is_empty() && !parsed.region.is_empty() {
        script = inferred_chinese_script(&parsed.language, &parsed.region);
        append_unique(&mut candidates, with_inserted_script(&parsed, &script));
    }

    append_unique(&mut candidates, normalized);

    if !parsed.script.is_empty() && !parsed.region.is_empty() {
        append_unique(&mut candidates, without_script(&parsed));
    }

    append_unique(&mut candidates, script_only(&parsed, &script));
    append_unique(&mut candidates, parsed.language.clone());
    candidates
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // Verbatim from tests/i18n_language_tag_test.cpp.

    #[test]
    fn normalizes_posix_locale() {
        assert_eq!(normalize_language_tag("zh_CN.UTF-8"), "zh-CN");
    }

    #[test]
    fn canonicalizes_region_case() {
        assert_eq!(normalize_language_tag("pt-br"), "pt-BR");
    }

    #[test]
    fn canonicalizes_script_case() {
        assert_eq!(normalize_language_tag("zh_hans_cn"), "zh-Hans-CN");
    }

    #[test]
    fn ignores_c_locale() {
        assert_eq!(normalize_language_tag("C.UTF-8"), "");
    }

    #[test]
    fn infers_simplified_chinese_from_china() {
        assert_eq!(
            catalog_language_candidates("zh_CN.UTF-8"),
            vec!["zh-Hans-CN", "zh-CN", "zh-Hans", "zh"]
        );
    }

    #[test]
    fn infers_traditional_chinese_from_taiwan() {
        assert_eq!(
            catalog_language_candidates("zh_TW"),
            vec!["zh-Hant-TW", "zh-TW", "zh-Hant", "zh"]
        );
    }

    #[test]
    fn keeps_explicit_chinese_script_first() {
        assert_eq!(
            catalog_language_candidates("zh-Hans-CN"),
            vec!["zh-Hans-CN", "zh-CN", "zh-Hans", "zh"]
        );
    }

    #[test]
    fn uses_generic_language_fallback_for_regional_locale() {
        assert_eq!(
            catalog_language_candidates("pt_BR.UTF-8"),
            vec!["pt-BR", "pt"]
        );
    }

    #[test]
    fn does_not_infer_scripts_for_non_chinese_locales() {
        assert_eq!(catalog_language_candidates("en_US"), vec!["en-US", "en"]);
    }

    #[test]
    fn does_not_build_candidates_for_the_c_locale() {
        assert_eq!(catalog_language_candidates("C.UTF-8"), Vec::<String>::new());
    }
}
