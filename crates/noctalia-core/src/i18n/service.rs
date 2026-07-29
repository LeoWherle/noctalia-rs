//! Port of `src/i18n/i18n_service.{h,cpp}`.
//!
//! C++'s `Service` is a Meyer's singleton (`static Service& instance()`),
//! unsynchronized because the C++ codebase only ever touches it from the UI
//! thread. Rust has no unsynchronized-global-singleton idiom without `unsafe`,
//! so this uses the same `LazyLock<Mutex<T>>` pattern as `core::log`'s
//! `FILE_STATE` instead — see [`super`]'s module doc comment for the resulting
//! `lookup`/`language` signature change (owned `String`s, not `string_view`s).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use crate::files::paths;
use crate::log::Logger;

use super::language_tag;

const LOG: Logger = Logger::new("i18n");

type Catalog = HashMap<String, String>;

pub struct LanguageOption {
    pub code: &'static str,
    pub display_name: &'static str,
}

pub const SUPPORTED_LANGUAGES: [LanguageOption; 22] = [
    LanguageOption {
        code: "be",
        display_name: "Беларуская",
    },
    LanguageOption {
        code: "be-Latn",
        display_name: "Biełaruskaja (Łacinka)",
    },
    LanguageOption {
        code: "ca",
        display_name: "Català",
    },
    LanguageOption {
        code: "cs",
        display_name: "Čeština",
    },
    LanguageOption {
        code: "de",
        display_name: "Deutsch",
    },
    LanguageOption {
        code: "en",
        display_name: "English",
    },
    LanguageOption {
        code: "es",
        display_name: "Español",
    },
    LanguageOption {
        code: "fr",
        display_name: "Français",
    },
    LanguageOption {
        code: "gl-ES",
        display_name: "Galego",
    },
    LanguageOption {
        code: "hu",
        display_name: "Magyar",
    },
    LanguageOption {
        code: "it",
        display_name: "Italiano",
    },
    LanguageOption {
        code: "ku",
        display_name: "Kurdî",
    },
    LanguageOption {
        code: "nl",
        display_name: "Nederlands",
    },
    LanguageOption {
        code: "nn",
        display_name: "Norsk nynorsk",
    },
    LanguageOption {
        code: "pl",
        display_name: "Polski",
    },
    LanguageOption {
        code: "pt-BR",
        display_name: "Português (Brasil)",
    },
    LanguageOption {
        code: "ru",
        display_name: "Русский",
    },
    LanguageOption {
        code: "sv",
        display_name: "Svenska",
    },
    LanguageOption {
        code: "tr",
        display_name: "Türkçe",
    },
    LanguageOption {
        code: "uk-UA",
        display_name: "Українська",
    },
    LanguageOption {
        code: "vi",
        display_name: "Tiếng Việt",
    },
    LanguageOption {
        code: "zh-Hans",
        display_name: "简体中文",
    },
];

fn flatten(node: &serde_json::Map<String, serde_json::Value>, prefix: &str, out: &mut Catalog) {
    for (key, value) in node {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            serde_json::Value::Object(obj) => flatten(obj, &path, out),
            serde_json::Value::String(s) => {
                out.insert(path, s.clone());
            }
            _ => {}
        }
    }
}

/// Loads and flattens `translations/<lang>.json` into a dotted-key catalog, or
/// `None` if the file doesn't exist, isn't valid UTF-8/JSON, or isn't a JSON
/// object — matching `Service::loadCatalog`'s `bool` return, minus the
/// `std::string&` out-param (Rust just returns the value).
fn load_catalog(lang: &str) -> Option<Catalog> {
    let path = paths::asset_path(&format!("translations/{lang}.json"));
    let text = std::fs::read_to_string(&path).ok()?;

    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(e) => {
            LOG.error(format_args!("failed to parse {}: {e}", path.display()));
            return None;
        }
    };
    let serde_json::Value::Object(map) = json else {
        LOG.warn(format_args!(
            "catalog {} is not a JSON object",
            path.display()
        ));
        return None;
    };

    let mut out = Catalog::new();
    flatten(&map, "", &mut out);
    Some(out)
}

/// Reads `$LC_ALL`/`$LC_MESSAGES`/`$LANG` in that order; `""` if none are set.
fn detect_system_language() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var)
            && !value.is_empty()
        {
            return language_tag::normalize_language_tag(&value);
        }
    }
    String::new()
}

struct Service {
    active: Catalog,
    fallback: Catalog,
    language: String,
}

impl Service {
    fn new() -> Self {
        Self {
            active: Catalog::new(),
            fallback: Catalog::new(),
            language: String::new(),
        }
    }

    fn init(&mut self, preferred_lang: &str) {
        let candidate = if !preferred_lang.is_empty() {
            language_tag::normalize_language_tag(preferred_lang)
        } else {
            detect_system_language()
        };

        // English fallback is always loaded first so lookup() can fall back to
        // it even if the active catalog is English itself.
        match load_catalog("en") {
            Some(catalog) => self.fallback = catalog,
            None => LOG.warn(format_args!("could not load English fallback catalog")),
        }

        if candidate.is_empty() || candidate == "en" {
            self.active = self.fallback.clone();
            self.language = "en".to_string();
            LOG.info(format_args!("language: en"));
            return;
        }

        for lang in language_tag::catalog_language_candidates(&candidate) {
            if let Some(catalog) = load_catalog(&lang) {
                self.active = catalog;
                self.language = lang.clone();
                if lang == candidate {
                    LOG.info(format_args!("language: {}", self.language));
                } else {
                    LOG.info(format_args!(
                        "language: {} (from {candidate})",
                        self.language
                    ));
                }
                return;
            }
        }

        LOG.warn(format_args!(
            "no catalog for '{candidate}', falling back to English"
        ));
        self.active = self.fallback.clone();
        self.language = "en".to_string();
    }

    fn set_language(&mut self, lang: &str) {
        if lang == self.language {
            return;
        }
        self.init(lang);
    }

    fn lookup(&self, dotted_key: &str) -> Option<String> {
        self.active
            .get(dotted_key)
            .or_else(|| self.fallback.get(dotted_key))
            .cloned()
    }
}

static SERVICE: LazyLock<Mutex<Service>> = LazyLock::new(|| Mutex::new(Service::new()));

/// Loads the active catalog. Pass `""` to auto-detect from the standard locale
/// environment (`$LC_ALL`/`$LC_MESSAGES`/`$LANG`). Always loads English as a
/// fallback first; if the detected language is also English, both slots end up
/// with the same catalog contents.
pub fn init(preferred_lang: &str) {
    SERVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .init(preferred_lang);
}

pub fn set_language(lang: &str) {
    SERVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .set_language(lang);
}

pub fn language() -> String {
    SERVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .language
        .clone()
}

/// The translation for `dotted_key` in the active or fallback catalog, or
/// `None` if it exists in neither.
pub fn lookup(dotted_key: &str) -> Option<String> {
    SERVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .lookup(dotted_key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn supported_languages_codes_are_unique() {
        let mut codes: Vec<&str> = SUPPORTED_LANGUAGES.iter().map(|lang| lang.code).collect();
        let original_len = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), original_len);
    }

    #[test]
    fn flatten_produces_dotted_keys_from_nested_objects() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"clipboard": {"title": "Clipboard"}, "top": "Top", "ignored": 1}"#,
        )
        .expect("valid JSON");
        let serde_json::Value::Object(map) = json else {
            unreachable!()
        };
        let mut out = Catalog::new();
        flatten(&map, "", &mut out);

        assert_eq!(out.get("clipboard.title"), Some(&"Clipboard".to_string()));
        assert_eq!(out.get("top"), Some(&"Top".to_string()));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn load_catalog_returns_none_for_a_missing_language() {
        assert!(load_catalog("xx-not-a-real-language").is_none());
    }

    #[test]
    fn init_with_english_loads_the_same_catalog_into_both_slots() {
        let mut service = Service::new();
        service.init("en");
        assert_eq!(service.language, "en");
        assert_eq!(service.active, service.fallback);
        assert!(
            !service.active.is_empty(),
            "en.json should be a real catalog"
        );
        let (key, value) = service
            .active
            .iter()
            .next()
            .expect("checked non-empty above");
        assert_eq!(service.lookup(key), Some(value.clone()));
    }

    #[test]
    fn set_language_is_a_no_op_when_unchanged() {
        let mut service = Service::new();
        service.init("en");
        service.active.insert("marker".to_string(), "x".to_string());
        service.set_language("en");
        // A real re-init would reload from disk and drop the marker; since the
        // language didn't change, set_language must not have called init again.
        assert!(service.active.contains_key("marker"));
    }
}
