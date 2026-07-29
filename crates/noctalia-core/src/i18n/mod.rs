//! Port of `src/i18n/*` — task 1.8.
//!
//! Three C++ source pairs, three submodules: `language_tag.{h,cpp}` →
//! [`language_tag`] (BCP-47-ish tag normalization and catalog-candidate
//! ordering, tested against `tests/i18n_language_tag_test.cpp`);
//! `i18n_service.{h,cpp}` → `service` (the JSON catalog loader/singleton and
//! [`SUPPORTED_LANGUAGES`], tested against
//! `tests/i18n_supported_languages_test.cpp` — ported as
//! `tests/i18n_supported_languages_test.rs` since it needs the real
//! `assets/translations` directory, not a per-module unit test); `i18n.{h,cpp}`
//! → `translate` (the [`tr`]/[`trp`] lookup-and-interpolate API).
//!
//! `tests/plugin_i18n_test.cpp` exercises `src/scripting/plugin_i18n.h`, which
//! is out of migration scope per CLAUDE.md (the plugin system isn't ported) —
//! not ported here.
//!
//! Two deliberate divergences, both from `service`:
//! - `Service` becomes a `Mutex`-guarded singleton (see `service`'s own doc
//!   comment) rather than C++'s unsynchronized Meyer's singleton, so `lookup`/
//!   `language` return owned `String`s instead of C++'s `std::string_view`
//!   into catalog storage — a lock guard can't outlive the function call.
//! - C++'s `Catalog` uses a hand-rolled transparent-hash/-eq pair so
//!   `unordered_map::find` can take a `string_view` without allocating (pre-
//!   C++20 heterogeneous lookup workaround). Rust's `HashMap<String, String>`
//!   already supports `.get(&str)` natively via `Borrow<str>`, so there's
//!   nothing to port for that part — it's a simplification, not a gap.

pub mod language_tag;
mod service;
mod translate;

pub use service::{LanguageOption, SUPPORTED_LANGUAGES, init, language, lookup, set_language};
pub use translate::{key_segment, tr, trp};
