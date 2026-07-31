//! Port of `src/system/desktop_entry.{cpp,h}` (task 5.5.1 + 5.5.2). 5.5.1 landed the
//! `DesktopEntry`/`DesktopAction` struct shapes only; this extends the module with the registry
//! side: `.desktop` file INI parsing, XDG directory scanning, and an inotify-watched
//! reload/version cache (`scanDesktopEntries`/`desktopEntries`/`desktopEntriesSnapshot`/
//! `desktopEntriesVersion`/`desktopEntryWatchFd`/`checkDesktopEntryReload`/
//! `refreshDesktopEntriesIfSourcesChanged`).
//!
//! No C++ test exists for this file (confirmed: no `desktop_entry_test.cpp` in `tests/`) — the
//! task's own done bar is fixture-driven parse tests plus a real-inotify reload test, same
//! precedent as task 1.2's `file_watcher`.
//!
//! Structural divergence from the C++ (recorded, not incidental): the C++'s `DesktopEntryCache`
//! is a private, anonymous-namespace class reached only through a single function-local `static
//! DesktopEntryCache instance` (`cache()`). Since there is no C++ test for this file, that
//! singleton was never independently testable either — this port instead makes
//! [`DesktopEntryCache`] a public, freestanding type (mirroring `core::files::FileWatcher`'s own
//! shape from task 1.2), with the free functions below backing onto one process-global instance
//! behind a `Mutex`, same role as the C++'s `cache()`. Tests construct their own
//! `DesktopEntryCache` instances directly, sidestepping the fact that XDG_DATA_HOME/inotify state
//! on one shared global singleton isn't safely testable under `cargo test`'s multithreaded runner.
//! `DesktopEntryCache` gained a `Drop` impl (closes the inotify fd, removes watches); the C++'s
//! singleton gets the same cleanup for free via its `static`'s process-exit destructor (which
//! *does* run, unlike a Rust `static`'s value, which never drops) — inconsequential either way
//! since the OS reclaims the fd at exit regardless, but per-test instances here now clean up
//! properly between runs, which the C++ singleton was never in a position to need.
//!
//! The C++'s `m_entries`/`m_entriesMutex` split matters for real: `entriesSnapshot()` is called
//! from a plugin script worker thread (`src/scripting/luau_host.cpp`'s `luau_appIconPath`) and is
//! deliberately built so that call never blocks on an in-progress `refreshIfNeeded()` rescan —
//! only the final pointer swap is guarded. This port keeps that property: the shared entry list
//! lives in its own `Arc<Mutex<Arc<Vec<DesktopEntry>>>>` handle (`DesktopEntryCache::entries`),
//! independent of the outer per-instance state a scan mutates; the process-global
//! `desktop_entries_snapshot()` free function below reads straight from a second, separately
//! initialized static (`shared_entries_handle`) rather than through `cache()`'s `Mutex`, so it
//! can never contend with a scan holding that outer lock.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use noctalia_core::log::Logger;

const LOG: Logger = Logger::new("desktop_entry");

/// Port of `DesktopAction` (a `[Desktop Action ...]` group in a `.desktop` file, e.g. "New
/// Window").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub exec: String,
}

/// Port of `DesktopEntry`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopEntry {
    pub id: String,
    pub path: String,
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub exec: String,
    pub icon: String,
    pub categories: String,
    pub keywords: String,
    pub startup_wm_class: String,
    pub working_dir: String,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,

    // Pre-lowercased for matching.
    pub name_lower: String,
    pub generic_name_lower: String,
    pub keywords_lower: String,
    pub categories_lower: String,
    pub startup_wm_class_lower: String,
    pub id_lower: String,
    pub exec_lower: String,

    // Desktop file actions (e.g. "New Window", "New Private Window").
    pub actions: Vec<DesktopAction>,
}

/// Port of the anonymous namespace's `parseDesktopBool`.
fn parse_desktop_bool(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower == "true" || lower == "1" || lower == "yes"
}

/// Port of `splitMultipleDesktopStrings`.
fn split_multiple_desktop_strings(parsed_values: &mut Vec<String>, full_value: &str) {
    let mut start = 0usize;
    let bytes = full_value.as_bytes();
    while start < bytes.len() {
        let delimiter = full_value[start..].find(';').map(|i| start + i);
        let token = match delimiter {
            Some(d) => &full_value[start..d],
            None => &full_value[start..],
        };
        if !token.is_empty() {
            parsed_values.push(token.to_string());
        }
        match delimiter {
            Some(d) => start = d + 1,
            None => break,
        }
    }
}

/// Port of `shouldShowOnCurrentDesktop`.
fn should_show_on_current_desktop(only_show_in: &[String], not_show_in: &[String]) -> bool {
    let visible_by_default = only_show_in.is_empty();
    let Ok(current_desktop) = std::env::var("XDG_CURRENT_DESKTOP") else {
        return visible_by_default;
    };
    if current_desktop.is_empty() {
        return visible_by_default;
    }

    let mut start = 0usize;
    while start <= current_desktop.len() {
        let delimiter = current_desktop[start..].find(':').map(|i| start + i);
        let token = match delimiter {
            Some(d) => &current_desktop[start..d],
            None => &current_desktop[start..],
        };
        if !token.is_empty() {
            if only_show_in.iter().any(|s| s == token) {
                return true;
            } else if not_show_in.iter().any(|s| s == token) {
                return false;
            }
        }
        match delimiter {
            Some(d) => start = d + 1,
            None => break,
        }
    }
    visible_by_default
}

#[derive(Debug, Clone, Default)]
struct LocaleInfo {
    lang: String,
    country: String,
}

/// Port of `parseLocale`.
fn parse_locale() -> LocaleInfo {
    let mut info = LocaleInfo::default();
    let Some(lang_env) = std::env::var("LANG")
        .ok()
        .or_else(|| std::env::var("LC_MESSAGES").ok())
    else {
        return info;
    };

    let mut sv: &str = &lang_env;
    if let Some(dot) = sv.find('.') {
        sv = &sv[..dot];
    }
    if let Some(at) = sv.find('@') {
        sv = &sv[..at];
    }

    if let Some(underscore) = sv.find('_') {
        info.lang = sv[..underscore].to_string();
        info.country = sv.to_string();
    } else {
        info.lang = sv.to_string();
    }
    info
}

/// C++ caches `parseLocale()`'s result in a function-local `static` inside `parseDesktopFile`,
/// computed once per process (the environment doesn't change mid-run); this `OnceLock` mirrors
/// that at the one real call site (`scan_desktop_entries`) instead. `parse_desktop_file` itself
/// still takes a plain `&LocaleInfo` parameter, so tests can supply an arbitrary locale directly
/// without touching process env vars or this cache.
fn cached_locale() -> &'static LocaleInfo {
    static LOCALE: OnceLock<LocaleInfo> = OnceLock::new();
    LOCALE.get_or_init(parse_locale)
}

/// Port of `extractLocalizedValue`. Returns an empty string when no localized variant matches —
/// same as the C++, including the (unfixed) ambiguity this creates against a genuinely empty
/// localized value (e.g. `Name[fr]=`), which the C++ treats identically to "not present".
fn extract_localized_value(line: &str, key: &str, locale: &LocaleInfo) -> String {
    if !locale.country.is_empty() {
        let loc_key = format!("{key}[{}]=", locale.country);
        if line.len() > loc_key.len() && line.starts_with(&loc_key) {
            return line[loc_key.len()..].to_string();
        }
    }
    if !locale.lang.is_empty() {
        let loc_key = format!("{key}[{}]=", locale.lang);
        if line.len() > loc_key.len() && line.starts_with(&loc_key) {
            return line[loc_key.len()..].to_string();
        }
    }
    String::new()
}

#[derive(Debug, Clone, Default)]
struct ActionData {
    name: String,
    exec: String,
    localized_name: String,
}

fn flush_current_action(
    current_action_id: &mut String,
    current_action_data: &mut ActionData,
    action_map: &mut HashMap<String, ActionData>,
) {
    if current_action_id.is_empty() {
        return;
    }
    if !current_action_data.localized_name.is_empty() {
        current_action_data.name = current_action_data.localized_name.clone();
    }
    if !current_action_data.name.is_empty() && !current_action_data.exec.is_empty() {
        action_map.insert(
            current_action_id.clone(),
            std::mem::take(current_action_data),
        );
    }
    current_action_id.clear();
    *current_action_data = ActionData::default();
}

/// Port of `parseDesktopFile`. The C++ appends directly to a caller-owned `entries` vector; this
/// returns `Option<DesktopEntry>` instead (the caller pushes it) — a mechanical signature change
/// only, not a behavior change: exactly the same filtering decides `None` vs `Some`.
fn parse_desktop_file(filepath: &Path, locale: &LocaleInfo) -> Option<DesktopEntry> {
    let mut file = std::fs::File::open(filepath).ok()?;
    let mut raw = Vec::new();
    // `std::ifstream`/`std::getline` never validate encoding — they copy raw bytes into a
    // `std::string`, so a single non-UTF-8 byte anywhere in the file must not abort parsing the
    // rest of it. Reading the whole file up front and lossily re-decoding each line (rather than
    // `BufRead::lines()`, which yields `Err` and stops dead at the first invalid-UTF-8 line) keeps
    // that behavior: every comparison this feeds is against a plain-ASCII literal (section
    // headers, key names), which a `U+FFFD` replacement inside a *value* never touches.
    {
        use std::io::Read as _;
        if file.read_to_end(&mut raw).is_err() {
            return None;
        }
    }
    let lines: Vec<String> = raw
        .split(|&b| b == b'\n')
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect();

    let mut entry = DesktopEntry {
        path: filepath.to_string_lossy().into_owned(),
        id: filepath
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Default::default()
    };

    let mut in_desktop_entry = false;
    let mut in_action = false;
    let mut localized_name = String::new();
    let mut localized_generic_name = String::new();
    let mut localized_comment = String::new();
    let mut ty = String::new();

    let mut only_show_in: Vec<String> = Vec::new();
    let mut not_show_in: Vec<String> = Vec::new();

    let mut action_order: Vec<String> = Vec::new();
    let mut action_map: HashMap<String, ActionData> = HashMap::new();
    let mut current_action_id = String::new();
    let mut current_action_data = ActionData::default();

    for mut line in lines {
        while matches!(line.as_bytes().last(), Some(b'\r' | b' ' | b'\t')) {
            line.pop();
        }

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') {
            flush_current_action(
                &mut current_action_id,
                &mut current_action_data,
                &mut action_map,
            );
            in_desktop_entry = false;
            in_action = false;

            if line == "[Desktop Entry]" {
                in_desktop_entry = true;
            } else if line.len() > 17 && line.starts_with("[Desktop Action ") && line.ends_with(']')
            {
                current_action_id = line[16..line.len() - 1].to_string();
                if !current_action_id.is_empty() {
                    in_action = true;
                }
            }
            continue;
        }

        if in_action {
            let loc_name = extract_localized_value(&line, "Name", locale);
            if !loc_name.is_empty() {
                current_action_data.localized_name = loc_name;
                continue;
            }
            let Some(eq) = line.find('=') else { continue };
            let key = &line[..eq];
            let value = &line[eq + 1..];
            if key == "Name" {
                current_action_data.name = value.to_string();
            } else if key == "Exec" {
                current_action_data.exec = value.to_string();
            }
            continue;
        }

        if !in_desktop_entry {
            continue;
        }

        let loc_name = extract_localized_value(&line, "Name", locale);
        if !loc_name.is_empty() {
            localized_name = loc_name;
            continue;
        }
        let loc_generic_name = extract_localized_value(&line, "GenericName", locale);
        if !loc_generic_name.is_empty() {
            localized_generic_name = loc_generic_name;
            continue;
        }
        let loc_comment = extract_localized_value(&line, "Comment", locale);
        if !loc_comment.is_empty() {
            localized_comment = loc_comment;
            continue;
        }

        let Some(eq) = line.find('=') else { continue };
        let key = &line[..eq];
        let value = &line[eq + 1..];

        match key {
            "Type" => ty = value.to_string(),
            "Name" => entry.name = value.to_string(),
            "GenericName" => entry.generic_name = value.to_string(),
            "Comment" => entry.comment = value.to_string(),
            "Exec" => entry.exec = value.to_string(),
            "Icon" => entry.icon = value.to_string(),
            "Categories" => entry.categories = value.to_string(),
            "Keywords" => entry.keywords = value.to_string(),
            "StartupWMClass" => entry.startup_wm_class = value.to_string(),
            "NoDisplay" => entry.no_display = parse_desktop_bool(value),
            "Hidden" => entry.hidden = parse_desktop_bool(value),
            "Path" => entry.working_dir = value.to_string(),
            "Terminal" => entry.terminal = parse_desktop_bool(value),
            "OnlyShowIn" => split_multiple_desktop_strings(&mut only_show_in, value),
            "NotShowIn" => split_multiple_desktop_strings(&mut not_show_in, value),
            "Actions" => split_multiple_desktop_strings(&mut action_order, value),
            _ => {}
        }
    }

    flush_current_action(
        &mut current_action_id,
        &mut current_action_data,
        &mut action_map,
    );

    if ty != "Application"
        || entry.no_display
        || entry.hidden
        || entry.name.is_empty()
        || !should_show_on_current_desktop(&only_show_in, &not_show_in)
    {
        return None;
    }

    if !localized_name.is_empty() {
        entry.name = localized_name;
    }
    if !localized_generic_name.is_empty() {
        entry.generic_name = localized_generic_name;
    }
    if !localized_comment.is_empty() {
        entry.comment = localized_comment;
    }

    entry.name_lower = entry.name.to_ascii_lowercase();
    entry.generic_name_lower = entry.generic_name.to_ascii_lowercase();
    entry.keywords_lower = entry.keywords.to_ascii_lowercase();
    entry.categories_lower = entry.categories.to_ascii_lowercase();
    entry.startup_wm_class_lower = entry.startup_wm_class.to_ascii_lowercase();
    entry.id_lower = entry.id.to_ascii_lowercase();
    entry.exec_lower = entry.exec.to_ascii_lowercase();

    for id in &action_order {
        if let Some(data) = action_map.get(id) {
            entry.actions.push(DesktopAction {
                id: id.clone(),
                name: data.name.clone(),
                exec: data.exec.clone(),
            });
        }
    }

    Some(entry)
}

/// Port of `xdgDataDirs`.
fn xdg_data_dirs() -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut append_dir = |dir: String| {
        if dir.is_empty() {
            return;
        }
        if seen.insert(dir.clone()) {
            dirs.push(dir);
        }
    };

    match std::env::var("XDG_DATA_HOME") {
        Ok(home) if !home.is_empty() => append_dir(home),
        _ => {
            if let Ok(user_home) = std::env::var("HOME") {
                append_dir(format!("{user_home}/.local/share"));
            }
        }
    }

    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS")
        && !data_dirs.is_empty()
    {
        let mut start = 0usize;
        while start < data_dirs.len() {
            match data_dirs[start..].find(':') {
                Some(i) => {
                    append_dir(data_dirs[start..start + i].to_string());
                    start += i + 1;
                }
                None => {
                    append_dir(data_dirs[start..].to_string());
                    break;
                }
            }
        }
    }

    // Keep canonical system directories as a safety net for partial env setups.
    append_dir("/usr/local/share".to_string());
    append_dir("/usr/share".to_string());

    dirs
}

/// Recursive `.desktop` file collection under `dir`, standing in for the C++'s
/// `fs::recursive_directory_iterator`. The recursion gate uses `DirEntry::file_type` (an
/// `lstat`, no symlink follow) matching `recursive_directory_iterator`'s default
/// `directory_options` (does not descend into symlinked directories, avoiding cycles); the
/// leaf-file test uses `fs::metadata` (follows symlinks), matching
/// `directory_entry::is_regular_file()` — same follows-vs-doesn't-follow split
/// `core::files::directory_scanner::scan` already documents for its own `is_directory` check.
fn collect_desktop_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for dir_entry in read_dir.flatten() {
        let path = dir_entry.path();
        if let Ok(file_type) = dir_entry.file_type()
            && file_type.is_dir()
        {
            collect_desktop_files(&path, out);
            continue;
        }
        let is_regular_file = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
        if is_regular_file && path.extension().and_then(|e| e.to_str()) == Some("desktop") {
            out.push(path);
        }
    }
}

/// Port of `scanDesktopEntries`.
pub fn scan_desktop_entries() -> Vec<DesktopEntry> {
    let mut entries: Vec<DesktopEntry> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let locale = cached_locale();

    for data_dir in xdg_data_dirs() {
        let app_dir = Path::new(&data_dir).join("applications");
        if !app_dir.is_dir() {
            continue;
        }

        let mut files = Vec::new();
        collect_desktop_files(&app_dir, &mut files);
        for path in files {
            let id = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Hidden/NoDisplay files still claim their ID so user-local overrides can suppress
            // lower-priority system entries — the reservation happens before parsing decides
            // whether the entry is actually kept.
            if !seen_ids.insert(id) {
                continue;
            }
            if let Some(entry) = parse_desktop_file(&path, locale) {
                entries.push(entry);
            }
        }
    }

    entries.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));
    entries
}

const WATCH_MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CLOSE_WRITE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_ATTRIB;

fn setup_watch_fd() -> i32 {
    // SAFETY: inotify_init1 is a plain syscall wrapper; IN_NONBLOCK|IN_CLOEXEC are valid flags.
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        LOG.warn(format_args!(
            "inotify_init1 failed, desktop entry hot reload disabled"
        ));
    }
    fd
}

/// Port of the anonymous namespace's `DesktopEntryCache` class. See the module doc comment for
/// why this is a public, freestanding type rather than a hidden singleton.
pub struct DesktopEntryCache {
    /// Mirrors the C++'s `m_entries`/`m_entriesMutex` split: this inner `Mutex` guards only the
    /// pointer swap, never a full rescan, so `entries_snapshot`/`entries_handle` readers never
    /// block behind `refresh_if_needed`.
    entries: Arc<Mutex<Arc<Vec<DesktopEntry>>>>,
    version: u64,
    inotify_fd: i32,
    dirty: bool,
    watches: HashMap<i32, String>,
    watched_paths: HashSet<String>,
    source_signature: String,
}

impl Default for DesktopEntryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopEntryCache {
    pub fn new() -> Self {
        Self::with_entries(Arc::new(Mutex::new(Arc::new(Vec::new()))))
    }

    fn with_entries(entries: Arc<Mutex<Arc<Vec<DesktopEntry>>>>) -> Self {
        Self {
            entries,
            version: 0,
            inotify_fd: setup_watch_fd(),
            dirty: true,
            watches: HashMap::new(),
            watched_paths: HashSet::new(),
            source_signature: String::new(),
        }
    }

    pub fn entries(&mut self) -> Arc<Vec<DesktopEntry>> {
        self.refresh_if_needed();
        Arc::clone(&self.entries.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Worker-thread-safe shared snapshot. Deliberately non-refreshing — freshness stays driven
    /// by the main thread's poll/reload path. Only ever contends with `refresh_if_needed`'s brief
    /// pointer swap, never with a whole rescan.
    pub fn entries_snapshot(&self) -> Arc<Vec<DesktopEntry>> {
        Arc::clone(&self.entries.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Exposes the shared, independently lockable entries handle — used by the process-global
    /// singleton so `desktop_entries_snapshot()` can read without ever touching `cache()`'s outer
    /// `Mutex`. See the module doc comment.
    pub fn entries_handle(&self) -> Arc<Mutex<Arc<Vec<DesktopEntry>>>> {
        Arc::clone(&self.entries)
    }

    pub fn version(&mut self) -> u64 {
        self.refresh_if_needed();
        self.version
    }

    pub fn watch_fd(&self) -> i32 {
        self.inotify_fd
    }

    pub fn check_sources_changed(&mut self) {
        if self.compute_source_signature() != self.source_signature {
            self.dirty = true;
        }
    }

    pub fn check_reload(&mut self) {
        if self.inotify_fd < 0 {
            return;
        }

        // `inotify_event` needs 4-byte alignment; see `core::files::file_watcher::dispatch`'s
        // identical `AlignedBuf` wrapper and SAFETY reasoning for why a plain `[u8; N]` isn't
        // enough on its own.
        #[repr(C, align(4))]
        struct AlignedBuf([u8; 4096]);

        let mut buf = AlignedBuf([0u8; 4096]);
        let header_size = std::mem::size_of::<libc::inotify_event>();
        let mut changed = false;

        loop {
            // SAFETY: `buf` is a valid, 4-byte-aligned buffer of the given length;
            // `self.inotify_fd` is a valid fd owned by this cache (checked >= 0 above).
            let n = unsafe { libc::read(self.inotify_fd, buf.0.as_mut_ptr().cast(), buf.0.len()) };
            if n <= 0 {
                break;
            }
            let n = n as usize;

            let mut offset = 0usize;
            while offset + header_size <= n {
                // SAFETY: the kernel writes complete, 4-byte-aligned `inotify_event` records
                // back to back; `offset + header_size <= n` guarantees the fixed-size header is
                // fully present, and `buf` itself is 4-byte aligned per `AlignedBuf`.
                let event = unsafe { &*(buf.0.as_ptr().add(offset).cast::<libc::inotify_event>()) };
                if event.mask & libc::IN_IGNORED != 0 {
                    self.watches.remove(&event.wd);
                } else {
                    changed = true;
                }
                offset += header_size + event.len as usize;
            }
        }

        if changed {
            self.dirty = true;
        }
    }

    fn refresh_if_needed(&mut self) {
        if !self.dirty {
            return;
        }

        let scanned = Arc::new(scan_desktop_entries());
        {
            let mut guard = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            *guard = scanned;
        }
        self.rebuild_watches();
        self.source_signature = self.compute_source_signature();
        self.dirty = false;
        self.version += 1;
    }

    /// Signature of the resolved application source directories: canonical path plus
    /// device/inode/mtime. The canonical path and inode change when a Nix profile generation is
    /// swapped; the directory mtime changes when entries are added or removed in place.
    ///
    /// Divergence: uses `fs::canonicalize` (fails on a nonexistent path) rather than the C++'s
    /// `fs::weakly_canonical` (best-effort on a nonexistent path too) — falling back to the
    /// unresolved path on failure either way. Since a `stat` failure right below already appends
    /// a `:missing` marker whenever the path doesn't (yet) exist, the exact textual form of an
    /// unresolvable path doesn't change whether the signature reflects "missing" — only whether
    /// two different not-yet-existing paths could theoretically collide in the `path` prefix,
    /// which the trailing `:missing` marker already disambiguates from the "exists" case, and
    /// which is not reachable for any two directories this function is actually called with
    /// (each `data_dir` is already distinct going in).
    fn compute_source_signature(&self) -> String {
        let mut sig = String::new();
        let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        sig.push_str("xdg_current_desktop=");
        sig.push_str(if current_desktop.is_empty() {
            "<unset>"
        } else {
            &current_desktop
        });
        sig.push('\n');

        for data_dir in xdg_data_dirs() {
            let app_dir = Path::new(&data_dir).join("applications");
            let resolved = std::fs::canonicalize(&app_dir).unwrap_or_else(|_| app_dir.clone());
            let path = resolved.to_string_lossy().into_owned();

            sig.push_str(&path);
            match std::fs::metadata(&path) {
                Ok(meta) => {
                    sig.push(':');
                    sig.push_str(&meta.dev().to_string());
                    sig.push(':');
                    sig.push_str(&meta.ino().to_string());
                    sig.push(':');
                    sig.push_str(&meta.mtime().to_string());
                    sig.push(':');
                    sig.push_str(&meta.mtime_nsec().to_string());
                }
                Err(_) => sig.push_str(":missing"),
            }
            sig.push('\n');
        }
        sig
    }

    fn clear_watches(&mut self) {
        if self.inotify_fd >= 0 {
            for &wd in self.watches.keys() {
                // SAFETY: each `wd` was returned by a prior successful `inotify_add_watch` on
                // this fd and is only removed once, here.
                unsafe {
                    libc::inotify_rm_watch(self.inotify_fd, wd);
                }
            }
        }
        self.watches.clear();
        self.watched_paths.clear();
    }

    fn rebuild_watches(&mut self) {
        self.clear_watches();
        if self.inotify_fd < 0 {
            return;
        }

        for data_dir in xdg_data_dirs() {
            let app_dir = Path::new(&data_dir).join("applications");
            if !app_dir.is_dir() {
                continue;
            }
            self.add_watch(&app_dir);
            self.add_watches_recursive(&app_dir);
        }
    }

    fn add_watches_recursive(&mut self, dir: &Path) {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return;
        };
        for dir_entry in read_dir.flatten() {
            let path = dir_entry.path();
            // The C++'s `it->is_directory(ec)` follows symlinks (matched here by `fs::metadata`,
            // same reasoning as `collect_desktop_files`), so a symlinked subdirectory still gets
            // its own watch — but `recursive_directory_iterator`'s default `directory_options`
            // does not descend *into* a symlinked directory (avoids cycles), matched here by
            // gating recursion on `file_type()` (an `lstat`, no follow) instead.
            let is_directory = std::fs::metadata(&path).is_ok_and(|m| m.is_dir());
            if is_directory {
                self.add_watch(&path);
            }
            if dir_entry.file_type().is_ok_and(|t| t.is_dir()) {
                self.add_watches_recursive(&path);
            }
        }
    }

    fn add_watch(&mut self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        if !self.watched_paths.insert(key.clone()) {
            return;
        }

        let Ok(path_c) = std::ffi::CString::new(key.clone().into_bytes()) else {
            return;
        };
        // SAFETY: `path_c` is a valid NUL-terminated C string alive for the call; `inotify_fd`
        // is a valid, open inotify file descriptor (checked >= 0 by the caller).
        let wd = unsafe { libc::inotify_add_watch(self.inotify_fd, path_c.as_ptr(), WATCH_MASK) };
        if wd < 0 {
            return;
        }
        self.watches.insert(wd, key);
    }
}

impl Drop for DesktopEntryCache {
    fn drop(&mut self) {
        self.clear_watches();
        if self.inotify_fd >= 0 {
            // SAFETY: `inotify_fd` is owned by this cache and not touched again.
            unsafe {
                libc::close(self.inotify_fd);
            }
        }
    }
}

static ENTRIES_HANDLE: OnceLock<Arc<Mutex<Arc<Vec<DesktopEntry>>>>> = OnceLock::new();

fn shared_entries_handle() -> Arc<Mutex<Arc<Vec<DesktopEntry>>>> {
    Arc::clone(ENTRIES_HANDLE.get_or_init(|| Arc::new(Mutex::new(Arc::new(Vec::new())))))
}

static CACHE: OnceLock<Mutex<DesktopEntryCache>> = OnceLock::new();

fn cache() -> &'static Mutex<DesktopEntryCache> {
    CACHE.get_or_init(|| Mutex::new(DesktopEntryCache::with_entries(shared_entries_handle())))
}

/// Port of `desktopEntries`.
pub fn desktop_entries() -> Arc<Vec<DesktopEntry>> {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries()
}

/// Port of `desktopEntriesSnapshot`. Reads straight from the shared handle rather than through
/// `cache()`'s `Mutex`, so a worker thread calling this never blocks behind an in-progress
/// `desktop_entries()` rescan on the main thread — see the module doc comment.
pub fn desktop_entries_snapshot() -> Arc<Vec<DesktopEntry>> {
    Arc::clone(
        &shared_entries_handle()
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    )
}

/// Port of `desktopEntriesVersion`.
pub fn desktop_entries_version() -> u64 {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .version()
}

/// Port of `desktopEntryWatchFd`.
pub fn desktop_entry_watch_fd() -> i32 {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .watch_fd()
}

/// Port of `checkDesktopEntryReload`.
pub fn check_desktop_entry_reload() {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .check_reload();
}

/// Port of `refreshDesktopEntriesIfSourcesChanged`.
pub fn refresh_desktop_entries_if_sources_changed() {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .check_sources_changed();
}

/// Shared lock for every test in this crate that mutates `XDG_DATA_HOME`/`XDG_DATA_DIRS`/`HOME`/
/// `LANG`/`LC_MESSAGES`/`XDG_CURRENT_DESKTOP` — `cargo test` runs a module's tests concurrently
/// within one process, so every such test holds this lock for its full risky window (same pattern
/// as `noctalia_core::process::test_support::ENV_MUTATION_LOCK`). `icon_resolver`'s test module
/// reuses this lock too (its `IconResolver` tests mutate `HOME`/`XDG_DATA_HOME`/`XDG_DATA_DIRS`),
/// rather than declaring a second, uncoordinated lock over the same process-global env vars.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::test_support::ENV_LOCK;
    use super::*;
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    fn make_temp_dir(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    fn write_desktop_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("failed to write desktop file");
        path
    }

    #[test]
    fn parses_a_basic_application_entry() {
        let dir = make_temp_dir("noctalia-desktop-entry-basic");
        let path = write_desktop_file(
            &dir,
            "myapp.desktop",
            "[Desktop Entry]\nType=Application\nName=My App\nExec=myapp %U\nIcon=myapp-icon\n",
        );

        let entry =
            parse_desktop_file(&path, &LocaleInfo::default()).expect("entry should be kept");
        assert_eq!(entry.id, "myapp");
        assert_eq!(entry.name, "My App");
        assert_eq!(entry.exec, "myapp %U");
        assert_eq!(entry.icon, "myapp-icon");
        assert_eq!(entry.name_lower, "my app");
        assert!(!entry.terminal);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn survives_an_invalid_utf8_byte_mid_file() {
        // `std::ifstream`/`std::getline` never validate encoding, so a single non-UTF-8 byte
        // anywhere in the file (e.g. a stray legacy-encoded translated comment) must not abort
        // parsing of the rest of it — see `parse_desktop_file`'s doc comment.
        let dir = make_temp_dir("noctalia-desktop-entry-invalid-utf8");
        let path = dir.join("badutf8.desktop");
        let mut contents = b"[Desktop Entry]\nType=Application\n".to_vec();
        contents.extend_from_slice(b"Comment=bad-byte-\xffhere\n");
        contents.extend_from_slice(b"Name=Still Parsed\nExec=stillparsed\n");
        std::fs::write(&path, &contents).expect("failed to write desktop file");

        let entry = parse_desktop_file(&path, &LocaleInfo::default())
            .expect("entry after the bad byte should still be kept");
        assert_eq!(entry.name, "Still Parsed");
        assert_eq!(entry.exec, "stillparsed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_non_application_type() {
        let dir = make_temp_dir("noctalia-desktop-entry-nonapp");
        let path = write_desktop_file(
            &dir,
            "link.desktop",
            "[Desktop Entry]\nType=Link\nName=A Link\nURL=https://example.com\n",
        );
        assert!(parse_desktop_file(&path, &LocaleInfo::default()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_no_display_and_hidden() {
        let dir = make_temp_dir("noctalia-desktop-entry-nodisplay");
        let no_display = write_desktop_file(
            &dir,
            "nodisplay.desktop",
            "[Desktop Entry]\nType=Application\nName=Hidden From Menu\nExec=x\nNoDisplay=true\n",
        );
        let hidden = write_desktop_file(
            &dir,
            "hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=Truly Hidden\nExec=x\nHidden=yes\n",
        );
        assert!(parse_desktop_file(&no_display, &LocaleInfo::default()).is_none());
        assert!(parse_desktop_file(&hidden, &LocaleInfo::default()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_missing_name() {
        let dir = make_temp_dir("noctalia-desktop-entry-noname");
        let path = write_desktop_file(
            &dir,
            "noname.desktop",
            "[Desktop Entry]\nType=Application\nExec=x\n",
        );
        assert!(parse_desktop_file(&path, &LocaleInfo::default()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_show_in_and_not_show_in_gate_on_current_desktop() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = make_temp_dir("noctalia-desktop-entry-showin");
        let only = write_desktop_file(
            &dir,
            "only.desktop",
            "[Desktop Entry]\nType=Application\nName=Only GNOME\nExec=x\nOnlyShowIn=GNOME;KDE;\n",
        );
        let not = write_desktop_file(
            &dir,
            "not.desktop",
            "[Desktop Entry]\nType=Application\nName=Not Sway\nExec=x\nNotShowIn=Sway;\n",
        );

        // SAFETY: guarded by ENV_LOCK for this test's whole window; no other test in this
        // module touches XDG_CURRENT_DESKTOP concurrently.
        unsafe {
            std::env::set_var("XDG_CURRENT_DESKTOP", "Sway");
        }
        assert!(
            parse_desktop_file(&only, &LocaleInfo::default()).is_none(),
            "OnlyShowIn=GNOME;KDE should hide on Sway"
        );
        assert!(
            parse_desktop_file(&not, &LocaleInfo::default()).is_none(),
            "NotShowIn=Sway should hide on Sway"
        );

        // SAFETY: see above.
        unsafe {
            std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
        }
        assert!(parse_desktop_file(&only, &LocaleInfo::default()).is_some());
        assert!(parse_desktop_file(&not, &LocaleInfo::default()).is_some());

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("XDG_CURRENT_DESKTOP");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn localized_name_overrides_default_by_lang_and_country() {
        let dir = make_temp_dir("noctalia-desktop-entry-locale");
        let path = write_desktop_file(
            &dir,
            "loc.desktop",
            "[Desktop Entry]\nType=Application\nName=Default Name\nName[fr]=Nom Francais\nName[fr_CA]=Nom Canadien\nExec=x\n",
        );

        let fr = LocaleInfo {
            lang: "fr".to_string(),
            country: String::new(),
        };
        let entry = parse_desktop_file(&path, &fr).expect("kept");
        assert_eq!(entry.name, "Nom Francais");

        let fr_ca = LocaleInfo {
            lang: "fr".to_string(),
            country: "fr_CA".to_string(),
        };
        let entry = parse_desktop_file(&path, &fr_ca).expect("kept");
        assert_eq!(entry.name, "Nom Canadien");

        let entry = parse_desktop_file(&path, &LocaleInfo::default()).expect("kept");
        assert_eq!(entry.name, "Default Name");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_actions_in_declared_order_and_skips_incomplete_ones() {
        let dir = make_temp_dir("noctalia-desktop-entry-actions");
        let path = write_desktop_file(
            &dir,
            "actions.desktop",
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Browser\n\
             Exec=browser\n\
             Actions=new-window;new-private-window;incomplete;\n\
             \n\
             [Desktop Action new-window]\n\
             Name=New Window\n\
             Exec=browser --new-window\n\
             \n\
             [Desktop Action new-private-window]\n\
             Name=New Private Window\n\
             Exec=browser --incognito\n\
             \n\
             [Desktop Action incomplete]\n\
             Name=No Exec Here\n",
        );

        let entry = parse_desktop_file(&path, &LocaleInfo::default()).expect("kept");
        assert_eq!(entry.actions.len(), 2);
        assert_eq!(entry.actions[0].id, "new-window");
        assert_eq!(entry.actions[0].name, "New Window");
        assert_eq!(entry.actions[0].exec, "browser --new-window");
        assert_eq!(entry.actions[1].id, "new-private-window");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn xdg_data_dirs_orders_home_first_dedups_and_always_appends_system_dirs() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home_dir = make_temp_dir("noctalia-desktop-entry-xdg-home");
        let home_str = home_dir.to_string_lossy().into_owned();

        // SAFETY: guarded by ENV_LOCK.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &home_str);
            std::env::set_var("XDG_DATA_DIRS", format!("{home_str}:/usr/share:/opt/extra"));
        }

        let dirs = xdg_data_dirs();
        assert_eq!(dirs[0], home_str, "XDG_DATA_HOME must come first");
        assert_eq!(
            dirs.iter().filter(|d| **d == home_str).count(),
            1,
            "duplicate of XDG_DATA_HOME within XDG_DATA_DIRS must be deduped"
        );
        assert!(dirs.contains(&"/opt/extra".to_string()));
        assert!(dirs.contains(&"/usr/local/share".to_string()));
        assert!(dirs.contains(&"/usr/share".to_string()));
        assert_eq!(
            dirs.iter().filter(|d| **d == "/usr/share").count(),
            1,
            "the always-appended system dir must not duplicate one already listed"
        );

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_DATA_DIRS");
        }
        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn scan_dedupes_by_id_first_occurrence_wins_even_when_hidden() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home_dir = make_temp_dir("noctalia-desktop-entry-scan-dedupe-home");
        let second_dir = make_temp_dir("noctalia-desktop-entry-scan-dedupe-second");
        let home_apps = home_dir.join("applications");
        let second_apps = second_dir.join("applications");
        std::fs::create_dir_all(&home_apps).expect("failed to create applications dir");
        std::fs::create_dir_all(&second_apps).expect("failed to create applications dir");

        // `home_dir` (XDG_DATA_HOME) is scanned before `second_dir` (XDG_DATA_DIRS). Its
        // "dup.desktop" is Hidden, so it claims the id "dup" but contributes no visible entry.
        // `second_dir` has a perfectly valid, visible "dup.desktop" of its own — proving the id
        // reservation (not just "no C++-visible entry was ever produced") is what suppresses it,
        // this must still not appear anywhere in the scan.
        write_desktop_file(
            &home_apps,
            "dup.desktop",
            "[Desktop Entry]\nType=Application\nName=Hidden Dup\nExec=x\nHidden=true\n",
        );
        write_desktop_file(
            &second_apps,
            "dup.desktop",
            "[Desktop Entry]\nType=Application\nName=Should Not Appear\nExec=x\n",
        );
        write_desktop_file(
            &home_apps,
            "visible.desktop",
            "[Desktop Entry]\nType=Application\nName=Zebra App\nExec=x\n",
        );
        write_desktop_file(
            &home_apps,
            "another.desktop",
            "[Desktop Entry]\nType=Application\nName=Apple App\nExec=x\n",
        );

        // SAFETY: guarded by ENV_LOCK.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", home_dir.to_string_lossy().into_owned());
            std::env::set_var("XDG_DATA_DIRS", second_dir.to_string_lossy().into_owned());
        }

        let entries = scan_desktop_entries();
        assert!(
            !entries.iter().any(|e| e.id == "dup"),
            "a Hidden entry's id must claim \"dup\" even though it produced no visible entry, \
             blocking the second directory's genuinely visible \"dup.desktop\" from appearing"
        );
        // Sorted by name_lower: "Apple App" < "Zebra App".
        let ids: Vec<&str> = entries
            .iter()
            .filter(|e| e.id == "another" || e.id == "visible")
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["another", "visible"],
            "entries must sort by name_lower"
        );

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_DATA_DIRS");
        }
        let _ = std::fs::remove_dir_all(&second_dir);
        let _ = std::fs::remove_dir_all(&home_dir);
    }

    fn wait_for(cache: &mut DesktopEntryCache, expected_version: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            cache.check_reload();
            if cache.version() >= expected_version {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cache_reloads_on_real_inotify_events_and_bumps_version() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home_dir = make_temp_dir("noctalia-desktop-entry-reload");
        let apps_dir = home_dir.join("applications");
        std::fs::create_dir_all(&apps_dir).expect("failed to create applications dir");
        write_desktop_file(
            &apps_dir,
            "first.desktop",
            "[Desktop Entry]\nType=Application\nName=First App\nExec=x\n",
        );

        // SAFETY: guarded by ENV_LOCK.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", home_dir.to_string_lossy().into_owned());
            std::env::set_var("XDG_DATA_DIRS", "");
        }

        let mut cache = DesktopEntryCache::new();
        assert!(
            cache.watch_fd() >= 0,
            "inotify_init1 should succeed in a test sandbox"
        );

        let entries = cache.entries();
        assert!(entries.iter().any(|e| e.id == "first"));
        let version_after_first_scan = cache.version();

        write_desktop_file(
            &apps_dir,
            "second.desktop",
            "[Desktop Entry]\nType=Application\nName=Second App\nExec=x\n",
        );
        // Force a close-write on the watched app dir (a plain create+write via `write_desktop_file`
        // already triggers IN_CREATE/IN_CLOSE_WRITE, but flush explicitly for determinism).
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(apps_dir.join("second.desktop"))
                .expect("reopen for flush");
            f.flush().expect("flush");
        }

        assert!(
            wait_for(
                &mut cache,
                version_after_first_scan + 1,
                Duration::from_secs(2)
            ),
            "cache did not reload after a new .desktop file appeared"
        );
        let entries = cache.entries();
        assert!(entries.iter().any(|e| e.id == "first"));
        assert!(entries.iter().any(|e| e.id == "second"));

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_DATA_DIRS");
        }
        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn check_sources_changed_detects_a_swapped_source_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home_a = make_temp_dir("noctalia-desktop-entry-swap-a");
        let home_b = make_temp_dir("noctalia-desktop-entry-swap-b");
        std::fs::create_dir_all(home_a.join("applications")).expect("mkdir a");
        std::fs::create_dir_all(home_b.join("applications")).expect("mkdir b");
        write_desktop_file(
            &home_a.join("applications"),
            "a.desktop",
            "[Desktop Entry]\nType=Application\nName=A App\nExec=x\n",
        );
        write_desktop_file(
            &home_b.join("applications"),
            "b.desktop",
            "[Desktop Entry]\nType=Application\nName=B App\nExec=x\n",
        );

        // SAFETY: guarded by ENV_LOCK.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", home_a.to_string_lossy().into_owned());
            std::env::set_var("XDG_DATA_DIRS", "");
        }

        let mut cache = DesktopEntryCache::new();
        let entries = cache.entries();
        assert!(entries.iter().any(|e| e.id == "a"));
        let version_before = cache.version();

        // Simulate a Nix-profile-style swap: point XDG_DATA_HOME somewhere else entirely.
        // Plain inotify watches on the old directory can't see this; `check_sources_changed`
        // is the mechanism meant to catch it.
        // SAFETY: see above.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", home_b.to_string_lossy().into_owned());
        }
        cache.check_sources_changed();
        let entries = cache.entries();
        assert!(
            cache.version() > version_before,
            "version should bump after a source swap"
        );
        assert!(entries.iter().any(|e| e.id == "b"));

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_DATA_DIRS");
        }
        let _ = std::fs::remove_dir_all(&home_a);
        let _ = std::fs::remove_dir_all(&home_b);
    }
}
