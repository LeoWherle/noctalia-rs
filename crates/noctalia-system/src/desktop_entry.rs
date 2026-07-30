//! Port of the `DesktopEntry`/`DesktopAction` struct shapes from `src/system/desktop_entry.h`
//! (task 5.5.1). The registry side of that header — `scanDesktopEntries`/`desktopEntries`/
//! `desktopEntriesSnapshot`/`desktopEntriesVersion`/`desktopEntryWatchFd`/
//! `checkDesktopEntryReload`/`refreshDesktopEntriesIfSourcesChanged`, i.e. the INI parsing, XDG
//! directory scan, and inotify-watched reload/cache — is task 5.5.2 and is not ported here. See
//! MIGRATION_PLAN.md task 5.5's split for the full breakdown.

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
