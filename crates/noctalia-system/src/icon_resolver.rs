//! Port of `src/system/icon_resolver.{cpp,h}` (task 5.5.4): freedesktop icon-theme resolution —
//! `index.theme` parsing, theme-inheritance walking, size-aware icon lookup, and a process-wide
//! theme-change poll shared by every `IconResolver` instance.
//!
//! This is the first GIO binding in the migration. Per the Phase A dependency strategy ("use
//! maintained binding crates where they exist"), it binds `GSettings` via the `gio`/`glib` crates
//! from the gtk-rs project rather than hand-rolling a `gio-sys` FFI shim: those crates already
//! wrap the exact C API the C++ calls (`g_settings_schema_source_get_default`/
//! `_lookup`/`g_settings_schema_has_key`/`g_settings_new`/`g_settings_get_string`), are actively
//! maintained, and build offline against the same `gio-2.0` pkg-config file the C++ already links
//! (confirmed present, transitively, via `glib` in `nix/rust-devshell.nix` — `pkg-config
//! --cflags --libs gio-2.0` resolves cleanly in the dev shell; no devshell change needed). This is
//! still an FFI-first Phase A port, not a Phase B pure-Rust swap: `gio-sys`/`glib-sys` are thin
//! generated bindings over the real C library, not a reimplementation.
//!
//! Two small local ports of not-yet-migrated `StringUtils` functions (`string_utils.h`), scoped
//! to this module only — same precedent as task 1.6.5's local `generate_uuid_v4`:
//! - `trim_and_unquote` reuses [`crate::brightness::c_trim`] (already verified byte-identical to
//!   `StringUtils::trim`'s `std::isspace`-based trim) for the trim half, and a local `unquote`
//!   port for the other half.
//! - `unquote` iterates `Vec<char>` rather than the C++'s byte-indexed `std::string_view`; the
//!   only byte(s) it ever treats specially are the ASCII quote characters and the ASCII backslash,
//!   so char-vs-byte iteration is behaviorally inert for any valid UTF-8 input (a multi-byte
//!   character can never itself equal one of those ASCII code points). The C++'s `text.size() < 2`
//!   early-return check (byte length) becomes a `chars.len() < 2` check (char count) here — again
//!   inert: for any input where a byte-length-2+/char-length-1 mismatch could occur (i.e. a single
//!   multi-byte character), the C++ path would proceed past the length check but then always fail
//!   the quote-character comparison anyway (its `front()`/`back()` are non-ASCII bytes that can
//!   never equal `'"'`/`'\''`), landing on the same "return unchanged" outcome either way.
//! - `parse_leading_stoi` locally re-implements just enough of `std::stoi`'s grammar (optional
//!   leading whitespace/sign, then a digit run; throws — caught by the C++'s `catch (...) {}` and
//!   left as a no-op — on no digits or on an out-of-`i32`-range result) to port `parseIndexTheme`'s
//!   two `try { entry.size/maxSize = std::stoi(value); } catch (...) {}` sites faithfully. Real
//!   `index.theme` `Size`/`MaxSize` values are always clean decimal integers per the freedesktop
//!   icon theme spec, so the full grammar match is a completeness measure, not a reachability
//!   concern in practice.
//!
//! `signatureFor`'s per-path modification-time marker is a genuine, recorded representational
//! divergence: the C++ uses `fs::last_write_time(path).time_since_epoch().count()`, a raw tick
//! count in an unspecified, platform-defined `file_clock` unit. Rust's `SystemTime` has no such
//! raw representation; this port instead formats nanoseconds since `UNIX_EPOCH` (or, for a
//! (unrealistic) pre-1970 mtime, a `-`-prefixed nanosecond count the other direction). The two
//! encodings are numerically different, but the property `signatureFor` actually needs — the
//! string changes if and only if the directory's mtime changes, and is otherwise stable — holds
//! identically for both.
//!
//! `IconResolver::resolve` returns an owned `String` here instead of the C++'s `const
//! std::string&` (a reference into either the resolver's cache or a member "empty" sentinel
//! string). This is a structural divergence, not a behavioral one — the resolver's own lookup/
//! caching logic, including which calls populate `m_cache`/`m_missingCache` and in what order, is
//! unchanged; only the ownership of the returned value differs, which no caller (including the
//! ported test) can observe.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::brightness::c_trim;

/// Port of `IconSearchDir`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IconSearchDir {
    pub path: String,
    pub size: i32,
    pub scalable: bool,
}

fn push_unique_dir(dirs: &mut Vec<IconSearchDir>, dir: IconSearchDir) {
    if dir.path.is_empty() {
        return;
    }
    if !dirs.iter().any(|d| d.path == dir.path) {
        dirs.push(dir);
    }
}

/// Port of the anonymous namespace's `sizeFromDirName`: the nominal size encoded in a well-known
/// subdir name like `"48x48"` or `"scalable"`.
fn size_from_dir_name(dir_name: &str) -> i32 {
    if dir_name.contains("scalable") {
        return 0;
    }
    let mut size = 0i32;
    for c in dir_name.chars() {
        if c.is_ascii_digit() {
            size = size * 10 + i32::from(c as u8 - b'0');
        } else if size > 0 {
            break;
        }
    }
    size
}

/// Port of `StringUtils::unquote` — see the module doc comment for the char-vs-byte iteration
/// note.
fn unquote(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return text.to_string();
    }
    let front = chars[0];
    let back = chars[chars.len() - 1];
    if (front == '"' && back == '"') || (front == '\'' && back == '\'') {
        let inner = &chars[1..chars.len() - 1];
        let mut result = String::with_capacity(inner.len());
        let mut i = 0usize;
        while i < inner.len() {
            if inner[i] == '\\' && i + 1 < inner.len() {
                i += 1;
            }
            result.push(inner[i]);
            i += 1;
        }
        return result;
    }
    text.to_string()
}

/// Port of the anonymous namespace's `trimAndUnquote`.
fn trim_and_unquote(value: &str) -> String {
    unquote(c_trim(value))
}

/// Minimal local port of just enough of `std::stoi`'s grammar (leading whitespace, optional
/// sign, then a digit run) for `parseIndexTheme`'s two `try { ... = std::stoi(value); } catch
/// (...) {}` sites — see the module doc comment for why this isn't shared with a not-yet-ported
/// `string_utils`. Returns `None` (leave the field unchanged, matching the C++'s `catch (...) {}`
/// no-op) when no digits are found, or when the parsed value doesn't fit in an `i32`.
fn parse_leading_stoi(value: &str) -> Option<i32> {
    let bytes = c_trim(value).as_bytes();
    let mut i = 0usize;
    let mut negative = false;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        negative = bytes[i] == b'-';
        i += 1;
    }
    let digits_start = i;
    let mut magnitude: i64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        magnitude = magnitude * 10 + i64::from(bytes[i] - b'0');
        i += 1;
        if magnitude > i64::from(i32::MAX) + 1 {
            // Already out of i32 range regardless of what follows; stop early rather than
            // growing `magnitude` unboundedly on a pathological input.
            break;
        }
    }
    if i == digits_start {
        return None;
    }
    let signed = if negative { -magnitude } else { magnitude };
    if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
        return None;
    }
    Some(signed as i32)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if value.is_empty() {
        return;
    }
    if !values.contains(&value) {
        values.push(value);
    }
}

/// Port of the anonymous namespace's `splitList`.
fn split_list(value: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    loop {
        let next = value[start..].find(separator).map(|i| i + start);
        let part = match next {
            Some(n) => &value[start..n],
            None => &value[start..],
        };
        let trimmed = trim_and_unquote(part);
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
        match next {
            Some(n) => start = n + separator.len_utf8(),
            None => break,
        }
    }
    parts
}

/// Port of the anonymous namespace's `xdgDataDirs`. Note this uses `std::env::var_os`/checks
/// emptiness the same way the C++ checks `home[0] != '\0'` — distinct from
/// `read_gtk_theme_candidates`' `HOME` check below, which (matching the C++) does not require
/// non-empty.
fn xdg_data_dirs() -> Vec<String> {
    let mut dirs = Vec::new();

    match std::env::var("XDG_DATA_HOME") {
        Ok(data_home) if !data_home.is_empty() => push_unique(&mut dirs, data_home),
        _ => {
            if let Ok(home) = std::env::var("HOME")
                && !home.is_empty()
            {
                push_unique(&mut dirs, format!("{home}/.local/share"));
            }
        }
    }

    match std::env::var("XDG_DATA_DIRS") {
        Ok(data_dirs) if !data_dirs.is_empty() => {
            for dir in split_list(&data_dirs, ':') {
                push_unique(&mut dirs, dir);
            }
        }
        _ => {
            push_unique(&mut dirs, "/usr/local/share".to_string());
            push_unique(&mut dirs, "/usr/share".to_string());
        }
    }

    dirs
}

/// Port of the anonymous namespace's `iconBaseDirs`.
fn icon_base_dirs(data_dirs: &[String]) -> Vec<String> {
    let mut roots = Vec::new();
    if let Some(first) = data_dirs.first() {
        push_unique(&mut roots, format!("{first}/icons"));
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        push_unique(&mut roots, format!("{home}/.icons"));
    }
    let start = if data_dirs.is_empty() { 0 } else { 1 };
    for dir in &data_dirs[start.min(data_dirs.len())..] {
        push_unique(&mut roots, format!("{dir}/icons"));
    }
    roots
}

/// Port of the anonymous namespace's `pixmapDirs`.
fn pixmap_dirs(data_dirs: &[String]) -> Vec<String> {
    let mut roots = Vec::new();
    for dir in data_dirs {
        push_unique(&mut roots, format!("{dir}/pixmaps"));
    }
    roots
}

#[derive(Debug, Clone, Default)]
struct IconThemePlan {
    base_dirs: Vec<String>,
    search_dirs: Vec<IconSearchDir>,
    pixmap_dirs: Vec<String>,
    active_theme: String,
    signature: String,
}

/// Port of the anonymous namespace's `signatureFor` — see the module doc comment for the
/// modification-time marker's representational divergence from the C++.
fn signature_for(plan: &IconThemePlan) -> String {
    let mut signature = plan.active_theme.clone();
    signature.push('\n');
    let append_path = |signature: &mut String, kind: &str, path: &str| {
        signature.push_str(kind);
        signature.push_str(path);
        signature.push(':');
        signature.push_str(&last_write_marker(path));
        signature.push('\n');
    };
    for root in &plan.base_dirs {
        append_path(&mut signature, "root:", root);
    }
    for dir in &plan.search_dirs {
        append_path(&mut signature, "theme:", &dir.path);
    }
    for dir in &plan.pixmap_dirs {
        append_path(&mut signature, "pixmap:", dir);
    }
    signature
}

fn last_write_marker(path: &str) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return "missing".to_string();
    };
    match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(since_epoch) => since_epoch.as_nanos().to_string(),
        Err(before_epoch) => format!("-{}", before_epoch.duration().as_nanos()),
    }
}

/// Port of the anonymous namespace's `GSettingsDeleter`/`iconSettings`: a lazily initialized
/// `GSettings` for `org.gnome.desktop.interface`, `None` if the schema (or its `icon-theme` key)
/// isn't installed. Built on `gio`'s own ref-counted `Settings` wrapper rather than a hand-rolled
/// deleter — see the module doc comment for why `gio`/`glib` are the Phase A choice here.
///
/// Divergence from the C++'s process-wide `static`: `gio::Settings` doesn't implement
/// `Send`/`Sync` (gtk-rs only marks GObject wrappers thread-safe when their GIR metadata says so,
/// and `GSettings` isn't), so a single instance can't be shared across threads behind a plain
/// `static`. This caches one `Settings` per thread instead (`thread_local!`), which is
/// behaviorally identical — every thread's instance talks to the same schema/key/backend — just
/// constructed once per thread rather than once per process.
fn icon_settings() -> Option<gio::Settings> {
    thread_local! {
        static SETTINGS: std::cell::OnceCell<Option<gio::Settings>> = const { std::cell::OnceCell::new() };
    }
    SETTINGS.with(|cell| {
        cell.get_or_init(|| {
            let source = gio::SettingsSchemaSource::default()?;
            let schema = source.lookup("org.gnome.desktop.interface", true)?;
            if !schema.has_key("icon-theme") {
                return None;
            }
            Some(gio::Settings::new("org.gnome.desktop.interface"))
        })
        .clone()
    })
}

/// Port of the anonymous namespace's `readGSettingsIconTheme`.
fn read_gsettings_icon_theme() -> Option<String> {
    use gio::prelude::SettingsExt as _;
    let settings = icon_settings()?;
    let value = trim_and_unquote(settings.string("icon-theme").as_str());
    if value.is_empty() { None } else { Some(value) }
}

/// Port of the anonymous namespace's `readGtkThemeCandidates`: theme name candidates in priority
/// order (GSettings, then GTK3 ini, then GTK4 ini).
fn read_gtk_theme_candidates() -> Vec<String> {
    let mut candidates = Vec::new();

    if let Some(value) = read_gsettings_icon_theme() {
        candidates.push(value);
    }

    // Matches the C++'s `home != nullptr` check exactly (unlike `xdg_data_dirs`/`icon_base_dirs`
    // above, this does not additionally require `HOME` to be non-empty).
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        for cfg in [
            "/.config/gtk-3.0/settings.ini",
            "/.config/gtk-4.0/settings.ini",
        ] {
            let Ok(contents) = std::fs::read_to_string(format!("{home}{cfg}")) else {
                continue;
            };
            for line in contents.lines() {
                if !line.starts_with("gtk-icon-theme-name") {
                    continue;
                }
                let Some(eq) = line.find('=') else {
                    continue;
                };
                let value = trim_and_unquote(&line[eq + 1..]);
                if !value.is_empty() {
                    candidates.push(value);
                }
            }
        }
    }

    candidates
}

#[derive(Debug, Clone, Default)]
struct DirEntry {
    size: i32,
    max_size: i32,
    scalable: bool,
}

/// Port of the anonymous namespace's `parseIndexTheme`: parses `index.theme` and returns
/// (subdir paths sorted by preference — scalable/large dirs first, parent theme names).
fn parse_index_theme(theme_root: &str) -> (Vec<IconSearchDir>, Vec<String>) {
    let Ok(contents) = std::fs::read_to_string(format!("{theme_root}/index.theme")) else {
        return (Vec::new(), Vec::new());
    };

    let mut dir_names: Vec<String> = Vec::new();
    let mut inherits: Vec<String> = Vec::new();
    let mut dir_map: HashMap<String, DirEntry> = HashMap::new();
    let mut current_section = String::new();

    for raw_line in contents.split('\n') {
        let mut line = raw_line;
        while line.ends_with('\r') || line.ends_with(' ') {
            line = &line[..line.len() - 1];
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[') {
            current_section = section.strip_suffix(']').unwrap_or(section).to_string();
            continue;
        }
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = &line[..eq];
        let value = &line[eq + 1..];

        if current_section == "Icon Theme" {
            if key == "Directories" {
                for name in split_list(value, ',') {
                    dir_map.entry(name.clone()).or_default();
                    dir_names.push(name);
                }
            } else if key == "Inherits" {
                inherits.extend(split_list(value, ','));
            }
        } else if !current_section.is_empty()
            && let Some(entry) = dir_map.get_mut(&current_section)
        {
            if key == "Size" {
                if let Some(size) = parse_leading_stoi(value) {
                    entry.size = size;
                }
            } else if key == "Type" {
                entry.scalable = value == "Scalable" || value == "Threshold";
            } else if key == "MaxSize"
                && let Some(max_size) = parse_leading_stoi(value)
            {
                entry.max_size = max_size;
            }
        }
    }

    // Sort dirs: scalable first, then by size descending (MaxSize first, then Size as a
    // tiebreaker). `sort_by_key` is stable, matching `std::ranges::stable_sort`.
    //
    // Uses `get` (non-consuming), not `remove`: `dirNames` can contain the same name more than
    // once if `Directories=` lists it twice, and the C++'s `dirMap[a]`/`dirMap[currentSection]`
    // (a plain, non-consuming map lookup) returns the same fully-populated entry for every
    // occurrence. An earlier version of this port used `remove`, which handed back a real entry
    // for the first occurrence of a repeated name but a zeroed default for every occurrence after
    // it — a real divergence from the C++ for that (unusual but spec-legal) input.
    let mut entries: Vec<(String, DirEntry)> = dir_names
        .into_iter()
        .map(|name| {
            let entry = dir_map.get(&name).cloned().unwrap_or_default();
            (name, entry)
        })
        .collect();
    entries.sort_by_key(|(_, e)| {
        (
            std::cmp::Reverse(e.scalable),
            std::cmp::Reverse(e.max_size),
            std::cmp::Reverse(e.size),
        )
    });

    let sorted_paths: Vec<IconSearchDir> = entries
        .into_iter()
        .map(|(name, entry)| IconSearchDir {
            path: name,
            size: entry.size,
            scalable: entry.scalable,
        })
        .collect();

    (sorted_paths, inherits)
}

const FALLBACK_THEME_SUBDIRS: [&str; 6] = [
    "/scalable/apps/",
    "/256x256/apps/",
    "/128x128/apps/",
    "/64x64/apps/",
    "/48x48/apps/",
    "/32x32/apps/",
];

/// Port of the anonymous namespace's `buildThemeSearchPaths`.
fn build_theme_search_paths(
    theme_name: &str,
    base_dirs: &[String],
    visited: &mut HashSet<String>,
    search_dirs: &mut Vec<IconSearchDir>,
) {
    if visited.contains(theme_name) {
        return;
    }
    visited.insert(theme_name.to_string());

    for base in base_dirs {
        let theme_root = format!("{base}/{theme_name}");
        if !Path::new(&theme_root).is_dir() {
            continue;
        }

        let (dirs, inherits) = parse_index_theme(&theme_root);

        if dirs.is_empty() {
            // No index.theme — fall back to common paths so the theme isn't silently skipped.
            for subdir in FALLBACK_THEME_SUBDIRS {
                push_unique_dir(
                    search_dirs,
                    IconSearchDir {
                        path: format!("{theme_root}{subdir}"),
                        size: size_from_dir_name(subdir),
                        scalable: subdir.contains("scalable"),
                    },
                );
            }
        } else {
            for dir in &dirs {
                push_unique_dir(
                    search_dirs,
                    IconSearchDir {
                        path: format!("{theme_root}/{}/", dir.path),
                        size: dir.size,
                        scalable: dir.scalable,
                    },
                );
            }
        }

        for parent in &inherits {
            build_theme_search_paths(parent, base_dirs, visited, search_dirs);
        }
    }
}

/// Port of the anonymous namespace's `buildThemePlan`.
fn build_theme_plan() -> IconThemePlan {
    let data_dirs = xdg_data_dirs();
    let base_dirs = icon_base_dirs(&data_dirs);
    let pixmap_dirs_list = pixmap_dirs(&data_dirs);

    let mut visited: HashSet<String> = HashSet::new();
    let mut search_dirs: Vec<IconSearchDir> = Vec::new();
    let mut active_theme = String::new();

    // Use the first candidate theme that actually exists on disk.
    for candidate in read_gtk_theme_candidates() {
        let exists = base_dirs
            .iter()
            .any(|base| Path::new(&format!("{base}/{candidate}")).is_dir());
        if exists {
            active_theme = candidate.clone();
            build_theme_search_paths(&candidate, &base_dirs, &mut visited, &mut search_dirs);
            break;
        }
    }

    // hicolor is the mandatory base theme — always include it last.
    build_theme_search_paths("hicolor", &base_dirs, &mut visited, &mut search_dirs);

    let mut plan = IconThemePlan {
        base_dirs,
        search_dirs,
        pixmap_dirs: pixmap_dirs_list,
        active_theme,
        signature: String::new(),
    };
    plan.signature = signature_for(&plan);
    plan
}

/// Shared across every `IconResolver` instance; every access goes through this `Mutex`. Port of
/// the anonymous namespace's function-local `static IconThemeState`/`iconThemeState()` — a
/// top-level `static` with a `const`-evaluable initializer plays the same "lazily built on first
/// real use" role here, since Rust statics (unlike C++ function-local `static`s) don't need a
/// runtime-computed initial value to be deferred: the expensive `build_theme_plan()` call still
/// only happens inside `ensure_theme_state_locked`, gated on `initialized`.
struct IconThemeState {
    initialized: bool,
    generation: u64,
    plan: IconThemePlan,
}

static ICON_THEME_STATE: Mutex<IconThemeState> = Mutex::new(IconThemeState {
    initialized: false,
    generation: 1,
    plan: IconThemePlan {
        base_dirs: Vec::new(),
        search_dirs: Vec::new(),
        pixmap_dirs: Vec::new(),
        active_theme: String::new(),
        signature: String::new(),
    },
});

/// Port of `ensureThemeStateLocked`. Requires `state`'s mutex to already be held.
fn ensure_theme_state_locked(state: &mut IconThemeState) {
    if !state.initialized {
        state.plan = build_theme_plan();
        state.initialized = true;
    }
}

/// Port of `IconResolver::checkThemeChanged`.
pub fn check_theme_changed() -> bool {
    let mut state = ICON_THEME_STATE.lock().unwrap_or_else(|p| p.into_inner());
    let next = build_theme_plan();
    if !state.initialized {
        state.plan = next;
        state.initialized = true;
        return false;
    }
    if next.signature == state.plan.signature {
        return false;
    }
    state.plan = next;
    state.generation += 1;
    true
}

/// Port of `IconResolver::themeGeneration`.
pub fn theme_generation() -> u64 {
    let mut state = ICON_THEME_STATE.lock().unwrap_or_else(|p| p.into_inner());
    ensure_theme_state_locked(&mut state);
    state.generation
}

/// Port of `IconResolver`. See the module doc comment for why `resolve` returns an owned
/// `String` rather than the C++'s `const std::string&`.
pub struct IconResolver {
    cache: HashMap<String, String>,
    missing_cache: HashSet<String>,
    search_dirs: Vec<IconSearchDir>,
    pixmap_dirs: Vec<String>,
    generation: u64,
    cache_missing: bool,
}

impl Default for IconResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl IconResolver {
    /// Port of the default `IconResolver()` constructor.
    pub fn new() -> Self {
        Self::with_cache_missing(false)
    }

    /// Port of `explicit IconResolver(bool cacheMissing)`.
    pub fn with_cache_missing(cache_missing: bool) -> Self {
        let mut resolver = Self {
            cache: HashMap::new(),
            missing_cache: HashSet::new(),
            search_dirs: Vec::new(),
            pixmap_dirs: Vec::new(),
            generation: 0,
            cache_missing,
        };
        resolver.rebuild();
        resolver
    }

    /// Port of `IconResolver::rebuild`.
    fn rebuild(&mut self) {
        let mut state = ICON_THEME_STATE.lock().unwrap_or_else(|p| p.into_inner());
        ensure_theme_state_locked(&mut state);
        self.search_dirs = state.plan.search_dirs.clone();
        self.pixmap_dirs = state.plan.pixmap_dirs.clone();
        self.cache.clear();
        self.missing_cache.clear();
        self.generation = state.generation;
    }

    /// Port of `IconResolver::ensureFresh`.
    fn ensure_fresh(&mut self) {
        if self.generation != theme_generation() {
            self.rebuild();
        }
    }

    /// Port of `IconResolver::resolve`.
    pub fn resolve(&mut self, icon_name: &str, target_size: i32) -> String {
        if icon_name.is_empty() {
            return String::new();
        }
        self.ensure_fresh();
        let key = format!("{icon_name}\u{1f}{}", target_size.max(0));
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let can_cache_missing = self.cache_missing && !icon_name.starts_with('/');
        if can_cache_missing && self.missing_cache.contains(&key) {
            return String::new();
        }
        let icon = self.find_icon(icon_name, target_size);
        if icon.is_empty() {
            if can_cache_missing {
                self.missing_cache.insert(key);
            }
            return String::new();
        }
        self.cache.insert(key, icon.clone());
        icon
    }

    /// Port of `IconResolver::invalidateMissingCache`.
    pub fn invalidate_missing_cache(&mut self) {
        self.missing_cache.clear();
    }

    /// Port of `IconResolver::findIcon`.
    fn find_icon(&self, name: &str, target_size: i32) -> String {
        // Absolute path — use directly.
        if name.starts_with('/') {
            return if Path::new(name).exists() {
                name.to_string()
            } else {
                String::new()
            };
        }

        if target_size <= 0 {
            // Legacy behavior: search dirs are pre-sorted scalable-first then largest; return the
            // first match. Callers with no size budget keep this exactly.
            for dir in &self.search_dirs {
                for ext in [".svg", ".png"] {
                    let path = format!("{}{name}{ext}", dir.path);
                    if Path::new(&path).exists() {
                        return path;
                    }
                }
            }
        } else {
            // Size-aware: a vector icon is crisp at any size, so an SVG always wins (first match
            // honors theme inheritance order).
            for dir in &self.search_dirs {
                let svg = format!("{}{name}.svg", dir.path);
                if Path::new(&svg).exists() {
                    return svg;
                }
            }

            // Among bitmaps, prefer the smallest theme size that is still >= the requested size
            // (gentle downscale); otherwise the largest available (least upscaling). Unknown-size
            // dirs are a last resort.
            let mut best = String::new();
            let mut best_size = 0i32;
            let mut best_is_upscale = true;
            for dir in &self.search_dirs {
                let png = format!("{}{name}.png", dir.path);
                if !Path::new(&png).exists() {
                    continue;
                }
                let size = dir.size;
                let is_upscale = size < target_size; // size 0 (unknown) counts as upscale
                let better = if best.is_empty() {
                    true
                } else if best_is_upscale != is_upscale {
                    !is_upscale // a downscale source always beats an upscale one
                } else if is_upscale {
                    size > best_size // upscaling: the bigger the source the better
                } else {
                    size < best_size // downscaling: the closer to target the better
                };
                if better {
                    best = png;
                    best_size = size;
                    best_is_upscale = is_upscale;
                }
            }
            if !best.is_empty() {
                return best;
            }
        }

        // Fallback: pixmaps.
        for dir in &self.pixmap_dirs {
            for ext in [".svg", ".png"] {
                let path = format!("{dir}/{name}{ext}");
                if Path::new(&path).exists() {
                    return path;
                }
            }
        }

        String::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::desktop_entry::test_support::ENV_LOCK;

    fn make_temp_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "noctalia-icon-resolver-test-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    /// Ported from `tests/icon_resolver_test.cpp`'s `main` verbatim (one test, matching the C++
    /// test's single-function shape and its dependency on a shared, cumulative `IconResolver`
    /// instance across assertions).
    #[test]
    fn matches_cpp_icon_resolver_test() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let root = make_temp_dir("root");
        let icon_dir = root.join("icons/hicolor/scalable/apps");
        std::fs::create_dir_all(&icon_dir).expect("create icon dir");

        let old_home = std::env::var_os("HOME");
        let old_data_home = std::env::var_os("XDG_DATA_HOME");
        let old_data_dirs = std::env::var_os("XDG_DATA_DIRS");
        // SAFETY: guarded by `ENV_LOCK` for this test's whole window.
        unsafe {
            std::env::set_var("HOME", &root);
            std::env::set_var("XDG_DATA_HOME", &root);
            std::env::set_var("XDG_DATA_DIRS", &root);
        }

        let mut resolver = IconResolver::with_cache_missing(true);

        let invalidated_icon = icon_dir.join("invalidated-icon.svg");
        assert!(
            resolver.resolve("invalidated-icon", 32).is_empty(),
            "initial missing icon should not resolve"
        );
        std::fs::write(&invalidated_icon, "<svg/>").expect("write invalidated icon");
        assert!(
            resolver.resolve("invalidated-icon", 32).is_empty(),
            "named icon miss should be cached"
        );
        resolver.invalidate_missing_cache();
        assert_eq!(
            resolver.resolve("invalidated-icon", 32),
            invalidated_icon.to_string_lossy(),
            "invalidating misses should discover a newly created icon"
        );

        let polled_icon = icon_dir.join("polled-icon.svg");
        assert!(
            resolver.resolve("polled-icon", 32).is_empty(),
            "second initial icon miss should be cached"
        );
        std::fs::write(&polled_icon, "<svg/>").expect("write polled icon");
        assert!(
            check_theme_changed(),
            "theme poll should detect icon directory changes"
        );
        assert_eq!(
            resolver.resolve("polled-icon", 32),
            polled_icon.to_string_lossy(),
            "theme generation change should invalidate cached misses"
        );

        let absolute_icon = root.join("absolute-icon.svg");
        assert!(
            resolver
                .resolve(&absolute_icon.to_string_lossy(), 32)
                .is_empty(),
            "missing absolute icon should not resolve"
        );
        std::fs::write(&absolute_icon, "<svg/>").expect("write absolute icon");
        assert_eq!(
            resolver.resolve(&absolute_icon.to_string_lossy(), 32),
            absolute_icon.to_string_lossy(),
            "absolute icon misses should not be cached"
        );

        // SAFETY: guarded by `ENV_LOCK`.
        unsafe {
            match old_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match old_data_home {
                Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match old_data_dirs {
                Some(v) => std::env::set_var("XDG_DATA_DIRS", v),
                None => std::env::remove_var("XDG_DATA_DIRS"),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn size_from_dir_name_reads_leading_digits() {
        assert_eq!(size_from_dir_name("48x48/apps"), 48);
        assert_eq!(size_from_dir_name("scalable/apps"), 0);
        assert_eq!(size_from_dir_name("apps"), 0);
        assert_eq!(size_from_dir_name("256x256@2/apps"), 256);
    }

    #[test]
    fn unquote_strips_matching_quotes_and_backslash_escapes() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'hello'"), "hello");
        assert_eq!(unquote(r#""a\"b""#), "a\"b");
        assert_eq!(unquote("unquoted"), "unquoted");
        assert_eq!(unquote("x"), "x");
    }

    #[test]
    fn parse_leading_stoi_matches_stoi_semantics() {
        assert_eq!(parse_leading_stoi("48"), Some(48));
        assert_eq!(parse_leading_stoi("  48px"), Some(48));
        assert_eq!(parse_leading_stoi("-1"), Some(-1));
        assert_eq!(parse_leading_stoi("not-a-number"), None);
        assert_eq!(parse_leading_stoi(""), None);
        assert_eq!(parse_leading_stoi("99999999999999999999"), None);
    }

    #[test]
    fn split_list_trims_and_unquotes_each_part() {
        assert_eq!(
            split_list("a, \"b\", ,c", ','),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn parse_index_theme_sorts_scalable_and_larger_dirs_first() {
        let dir = make_temp_dir("index-theme");
        std::fs::write(
            dir.join("index.theme"),
            "[Icon Theme]\n\
             Directories=32x32/apps,scalable/apps,64x64/apps\n\
             Inherits=hicolor,minimal\n\
             \n\
             [32x32/apps]\n\
             Size=32\n\
             Type=Fixed\n\
             \n\
             [scalable/apps]\n\
             Size=48\n\
             Type=Scalable\n\
             MaxSize=512\n\
             \n\
             [64x64/apps]\n\
             Size=64\n\
             Type=Fixed\n",
        )
        .expect("write index.theme");

        let (dirs, inherits) = parse_index_theme(&dir.to_string_lossy());
        assert_eq!(
            dirs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(),
            vec!["scalable/apps", "64x64/apps", "32x32/apps"],
            "scalable first, then descending size"
        );
        assert!(dirs[0].scalable);
        assert_eq!(inherits, vec!["hicolor".to_string(), "minimal".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_index_theme_returns_empty_when_file_is_missing() {
        let (dirs, inherits) = parse_index_theme("/nonexistent/theme/root");
        assert!(dirs.is_empty());
        assert!(inherits.is_empty());
    }

    #[test]
    fn parse_index_theme_keeps_metadata_when_a_directory_name_repeats() {
        // `dirMap[a]` in the C++ is a non-consuming lookup, so every occurrence of a repeated
        // name in `Directories=` sees the same fully-populated entry. A `HashMap::remove`-based
        // port would zero out every occurrence after the first — this is the regression case for
        // that bug.
        let dir = make_temp_dir("index-theme-dup");
        std::fs::write(
            dir.join("index.theme"),
            "[Icon Theme]\n\
             Directories=dupdir,dupdir\n\
             \n\
             [dupdir]\n\
             Size=48\n\
             MaxSize=512\n\
             Type=Scalable\n",
        )
        .expect("write index.theme");

        let (dirs, _inherits) = parse_index_theme(&dir.to_string_lossy());
        assert_eq!(
            dirs.len(),
            2,
            "both occurrences of the repeated name are kept"
        );
        for entry in &dirs {
            assert_eq!(entry.path, "dupdir");
            assert_eq!(entry.size, 48);
            assert!(
                entry.scalable,
                "every occurrence should see the same populated entry"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
