//! Port of `tests/i18n_supported_languages_test.cpp`: every entry in
//! `i18n::SUPPORTED_LANGUAGES` must have a matching `assets/translations/*.json`
//! catalog, and every catalog under `assets/translations` must have a matching
//! entry in `SUPPORTED_LANGUAGES` — kept as its own test binary (rather than a
//! unit test inside `i18n::service`) because it needs the real repo `assets/`
//! directory, matching the C++ test's own use of `NOCTALIA_SOURCE_ASSETS_DIR`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::fs;

use noctalia_core::files::paths;
use noctalia_core::i18n::SUPPORTED_LANGUAGES;

#[test]
fn supported_languages_and_catalog_files_match_exactly() {
    let translations_dir = paths::assets_root().join("translations");

    let supported: HashSet<&str> = SUPPORTED_LANGUAGES.iter().map(|lang| lang.code).collect();

    for lang in &SUPPORTED_LANGUAGES {
        let path = translations_dir.join(format!("{}.json", lang.code));
        assert!(
            path.is_file(),
            "supported language has no catalog: {}",
            lang.code
        );
    }

    let entries = fs::read_dir(&translations_dir).expect("failed to read translations dir");
    for entry in entries {
        let path = entry.expect("failed to read dir entry").path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let code = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("non-UTF-8 catalog filename");
        assert!(
            supported.contains(code),
            "catalog is missing from SUPPORTED_LANGUAGES: {code}"
        );
    }
}
