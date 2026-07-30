//! Port of `src/system/app_identity.{cpp,h}` (task 5.5.1): matching a running window/app id
//! against the parsed `.desktop` entry list (task 5.5.2, not yet ported — callers here take the
//! entry list as a parameter, same as the C++ signatures).

use std::collections::HashSet;

use crate::brightness::is_c_isspace_byte;
use crate::desktop_entry::DesktopEntry;
use crate::internal_app_metadata::apply_metadata_to_desktop_entry;

/// Port of `ResolvedRunningApp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRunningApp {
    pub running_app_id: String,
    pub running_lower: String,
    pub entry: DesktopEntry,
}

/// Port of the anonymous namespace's `identityKey`: strips `.`, `-`, `_`, and `std::isspace`
/// bytes, lowercasing the rest. Operates byte-wise (like the C++'s `unsigned char` iteration) so
/// non-ASCII UTF-8 sequences pass through unmodified rather than being torn.
fn identity_key(value: &str) -> String {
    let mut key = Vec::with_capacity(value.len());
    for byte in value.bytes() {
        if byte == b'.' || byte == b'-' || byte == b'_' || is_c_isspace_byte(byte) {
            continue;
        }
        key.push(byte.to_ascii_lowercase());
    }
    String::from_utf8_lossy(&key).into_owned()
}

/// Port of `identityKeyMatches`.
fn identity_key_matches(value_key: &str, candidate: &str) -> bool {
    if candidate.is_empty() {
        return false;
    }
    value_key == identity_key(candidate)
}

/// Port of `appIdTail`.
fn app_id_tail(app_key: &str) -> &str {
    let mut tail = app_key;
    if let Some(slash) = tail.rfind('/')
        && slash + 1 < tail.len()
    {
        tail = &tail[slash + 1..];
    }
    if let Some(dot) = tail.rfind('.')
        && dot + 1 < tail.len()
    {
        tail = &tail[dot + 1..];
    }
    tail
}

/// Port of `findDesktopEntryByIdTail`.
fn find_desktop_entry_by_id_tail(
    app_key: &str,
    all_entries: &[DesktopEntry],
) -> Option<DesktopEntry> {
    let app_lower = app_key.to_ascii_lowercase();
    let tail_lower = app_id_tail(app_key).to_ascii_lowercase();
    if tail_lower.is_empty() || tail_lower == app_lower {
        return None;
    }

    let candidates: Vec<&DesktopEntry> = all_entries
        .iter()
        .filter(|entry| app_id_tail(&entry.id).to_ascii_lowercase() == tail_lower)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }

    let mut best: Option<&DesktopEntry> = None;
    for entry in candidates {
        if desktop_entry_matches_lower(entry, &app_lower) {
            if best.is_some() {
                return None;
            }
            best = Some(entry);
        }
    }
    best.cloned()
}

struct DesktopEntryResolution {
    entry: DesktopEntry,
    matched_desktop_entry: bool,
}

/// Port of `resolveRunningDesktopEntryWithStatus`.
fn resolve_running_desktop_entry_with_status(
    running_app_id: &str,
    all_entries: &[DesktopEntry],
) -> DesktopEntryResolution {
    if let Some(mut matched) = find_desktop_entry(running_app_id, all_entries) {
        if running_app_id.starts_with("steam_app_") && matched.startup_wm_class.is_empty() {
            matched.startup_wm_class = running_app_id.to_string();
        }
        return DesktopEntryResolution {
            entry: matched,
            matched_desktop_entry: true,
        };
    }

    let mut fallback = DesktopEntry {
        id: running_app_id.to_string(),
        name: running_app_id.to_string(),
        name_lower: running_app_id.to_ascii_lowercase(),
        ..Default::default()
    };
    apply_metadata_to_desktop_entry(&mut fallback);

    DesktopEntryResolution {
        entry: fallback,
        matched_desktop_entry: false,
    }
}

/// Port of `matchesLower`.
pub fn matches_lower(
    value_lower: &str,
    id_lower: &str,
    startup_wm_class_lower: &str,
    name_lower: &str,
) -> bool {
    if value_lower.is_empty() {
        return false;
    }
    let value_key = identity_key(value_lower);
    value_lower == id_lower
        || value_lower == startup_wm_class_lower
        || value_lower == name_lower
        || (!value_key.is_empty()
            && (identity_key_matches(&value_key, id_lower)
                || identity_key_matches(&value_key, startup_wm_class_lower)))
}

/// Port of `desktopEntryMatchesLower`.
pub fn desktop_entry_matches_lower(entry: &DesktopEntry, value_lower: &str) -> bool {
    matches_lower(
        value_lower,
        &entry.id.to_ascii_lowercase(),
        &entry.startup_wm_class.to_ascii_lowercase(),
        &entry.name_lower,
    )
}

/// Port of `findDesktopEntry`.
pub fn find_desktop_entry(app_key: &str, all_entries: &[DesktopEntry]) -> Option<DesktopEntry> {
    if app_key.is_empty() {
        return None;
    }

    let app_lower = app_key.to_ascii_lowercase();
    for entry in all_entries {
        if desktop_entry_matches_lower(entry, &app_lower) {
            return Some(entry.clone());
        }
    }

    if let Some(matched) = find_desktop_entry_by_id_tail(app_key, all_entries) {
        return Some(matched);
    }

    if !app_key.starts_with("steam_app_") {
        return None;
    }

    let steam_id = &app_key["steam_app_".len()..];
    if steam_id.is_empty() {
        return None;
    }
    let run_game_token = format!("rungameid/{steam_id}");

    for entry in all_entries {
        if entry.startup_wm_class.to_ascii_lowercase() == app_lower {
            return Some(entry.clone());
        }
        if entry.exec.contains(&run_game_token) {
            return Some(entry.clone());
        }
    }

    None
}

/// Port of `resolveRunningDesktopEntry`.
pub fn resolve_running_desktop_entry(
    running_app_id: &str,
    all_entries: &[DesktopEntry],
) -> DesktopEntry {
    resolve_running_desktop_entry_with_status(running_app_id, all_entries).entry
}

/// Port of `resolveRunningApps`.
pub fn resolve_running_apps(
    running_app_ids: &[String],
    all_entries: &[DesktopEntry],
) -> Vec<ResolvedRunningApp> {
    let mut resolved = Vec::with_capacity(running_app_ids.len());
    let mut seen: HashSet<String> = HashSet::with_capacity(running_app_ids.len());

    for running_app_id in running_app_ids {
        let running_lower = running_app_id.to_ascii_lowercase();
        let resolution = resolve_running_desktop_entry_with_status(running_app_id, all_entries);
        let mut dedupe_key = if resolution.matched_desktop_entry {
            resolution.entry.id.to_ascii_lowercase()
        } else {
            running_lower.clone()
        };
        if dedupe_key.is_empty() {
            dedupe_key = running_lower.clone();
        }
        if !seen.insert(dedupe_key) {
            continue;
        }

        resolved.push(ResolvedRunningApp {
            running_app_id: running_app_id.clone(),
            running_lower,
            entry: resolution.entry,
        });
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chat_entry() -> DesktopEntry {
        DesktopEntry {
            id: "sample-chat-desktop".to_string(),
            name: "Sample Chat".to_string(),
            name_lower: "sample chat".to_string(),
            startup_wm_class: "SampleChat".to_string(),
            startup_wm_class_lower: "samplechat".to_string(),
            exec: "sample-chat-desktop".to_string(),
            icon: "sample-chat-desktop".to_string(),
            ..Default::default()
        }
    }

    fn sample_mail_entry() -> DesktopEntry {
        DesktopEntry {
            id: "sample-mail".to_string(),
            name: "Sample Mail".to_string(),
            name_lower: "sample mail".to_string(),
            startup_wm_class: "SampleMail".to_string(),
            startup_wm_class_lower: "samplemail".to_string(),
            exec: "sample-mail".to_string(),
            icon: "sample-mail".to_string(),
            ..Default::default()
        }
    }

    fn display_name_only_entry() -> DesktopEntry {
        DesktopEntry {
            id: "other-app".to_string(),
            name: "Risky Match".to_string(),
            name_lower: "risky match".to_string(),
            startup_wm_class: "OtherApp".to_string(),
            startup_wm_class_lower: "otherapp".to_string(),
            exec: "other-app".to_string(),
            icon: "other-app".to_string(),
            ..Default::default()
        }
    }

    fn easy_effects_entry() -> DesktopEntry {
        DesktopEntry {
            id: "com.github.wwmm.easyeffects".to_string(),
            name: "Easy Effects".to_string(),
            name_lower: "easy effects".to_string(),
            startup_wm_class: "easyeffects".to_string(),
            startup_wm_class_lower: "easyeffects".to_string(),
            exec: "easyeffects".to_string(),
            icon: "easyeffects".to_string(),
            ..Default::default()
        }
    }

    fn duplicate_tail_entry(id: &str, startup_wm_class: &str) -> DesktopEntry {
        let name = id.to_string();
        DesktopEntry {
            id: id.to_string(),
            name: name.clone(),
            name_lower: name.to_ascii_lowercase(),
            startup_wm_class: startup_wm_class.to_string(),
            startup_wm_class_lower: startup_wm_class.to_ascii_lowercase(),
            exec: id.to_string(),
            icon: id.to_string(),
            ..Default::default()
        }
    }

    // 1:1 port of tests/app_identity_test.cpp. The C++ test replaces `internal_apps::
    // metadataForAppId`/`applyMetadataToDesktopEntry` with no-op stand-ins (real `internal_apps`
    // has no test-time seam to swap in Rust); since the fallback paths exercised here never match
    // the one real internal app (`dev.noctalia.Noctalia`), calling the real functions is
    // equivalent to the C++ test's stubs for every case below.
    #[test]
    fn ported_app_identity_test_cpp() {
        let chat = sample_chat_entry();

        assert!(desktop_entry_matches_lower(&chat, "sample-chat-desktop"));
        assert!(desktop_entry_matches_lower(&chat, "samplechat"));
        assert!(desktop_entry_matches_lower(&chat, "sample chat"));
        assert!(desktop_entry_matches_lower(&chat, "sample.chat.desktop"));
        assert!(desktop_entry_matches_lower(&chat, "sample_chat_desktop"));
        assert!(desktop_entry_matches_lower(&chat, "sample chat desktop"));
        assert!(desktop_entry_matches_lower(&chat, "Sample.ChatDesktop"));
        assert!(!desktop_entry_matches_lower(&chat, ""));
        assert!(!desktop_entry_matches_lower(&chat, "sample-calendar"));

        let display_name_only = display_name_only_entry();
        assert!(desktop_entry_matches_lower(
            &display_name_only,
            "risky match"
        ));
        assert!(!desktop_entry_matches_lower(
            &display_name_only,
            "risky.match"
        ));

        let entries = vec![chat.clone()];
        let resolved = resolve_running_desktop_entry("Sample.ChatDesktop", &entries);
        assert_eq!(resolved.id, "sample-chat-desktop");
        assert_eq!(resolved.exec, "sample-chat-desktop");
        assert_eq!(resolved.icon, "sample-chat-desktop");

        let fallback = resolve_running_desktop_entry("Unknown.App", &entries);
        assert_eq!(fallback.id, "Unknown.App");
        assert_eq!(fallback.name, "Unknown.App");
        assert_eq!(fallback.name_lower, "unknown.app");
        assert!(fallback.exec.is_empty());
        assert!(fallback.icon.is_empty());

        // Hidden/NoDisplay entries are excluded at parse time (task 5.5.2), so the resolver never
        // receives one in production and does not re-filter them. If one is present it resolves
        // like any other entry.
        let mut hidden = sample_chat_entry();
        hidden.hidden = true;
        assert_eq!(
            resolve_running_desktop_entry("Sample.ChatDesktop", std::slice::from_ref(&hidden)).id,
            "sample-chat-desktop"
        );

        let mut no_display = sample_chat_entry();
        no_display.no_display = true;
        assert_eq!(
            resolve_running_desktop_entry("Sample.ChatDesktop", std::slice::from_ref(&no_display))
                .id,
            "sample-chat-desktop"
        );

        let multiple_entries = vec![sample_chat_entry(), sample_mail_entry()];
        let resolved_apps = resolve_running_apps(
            &[
                "Sample.ChatDesktop".to_string(),
                "sample-chat-desktop".to_string(),
                "SampleMail".to_string(),
            ],
            &multiple_entries,
        );
        assert_eq!(resolved_apps.len(), 2);
        assert_eq!(resolved_apps[0].entry.id, "sample-chat-desktop");
        assert_eq!(resolved_apps[1].entry.id, "sample-mail");

        let unknown_apps = resolve_running_apps(
            &["Unknown.App".to_string(), "unknown-app".to_string()],
            &multiple_entries,
        );
        assert_eq!(unknown_apps.len(), 2);
        assert_eq!(unknown_apps[0].entry.id, "Unknown.App");
        assert_eq!(unknown_apps[1].entry.id, "unknown-app");

        let easy_effects = easy_effects_entry();
        let kde_resolved = resolve_running_desktop_entry(
            "org.kde.easyeffects",
            std::slice::from_ref(&easy_effects),
        );
        assert_eq!(kde_resolved.id, "com.github.wwmm.easyeffects");
        assert_eq!(kde_resolved.name, "Easy Effects");
        assert_eq!(kde_resolved.icon, "easyeffects");

        let ambiguous_tail = vec![
            duplicate_tail_entry("com.foo.easyeffects", "foo-easyeffects"),
            duplicate_tail_entry("com.bar.easyeffects", "bar-easyeffects"),
        ];
        let ambiguous_resolved =
            resolve_running_desktop_entry("org.kde.easyeffects", &ambiguous_tail);
        assert_eq!(ambiguous_resolved.id, "org.kde.easyeffects");
    }

    // internal_apps coverage with no C++ test counterpart: the C++ test only exercises
    // app_identity's fallback path through stubbed-out internal_apps functions, so the real
    // internal_apps behavior (task 5.5.1's other module) has no C++ test to port from either.
    #[test]
    fn fallback_for_internal_app_id_applies_metadata() {
        let resolved = resolve_running_desktop_entry("dev.noctalia.Noctalia", &[]);
        assert_eq!(resolved.id, "dev.noctalia.Noctalia");
        assert_eq!(resolved.name, "Noctalia");
        assert_eq!(resolved.name_lower, "noctalia");
        assert!(resolved.icon.ends_with("noctalia.svg"));
    }

    #[test]
    fn identity_key_matches_across_separators_and_non_ascii_is_untouched() {
        assert_eq!(
            identity_key("Sample.Chat-Desktop_Two Words"),
            "samplechatdesktoptwowords"
        );
        // A non-ASCII UTF-8 sequence (é = 0xC3 0xA9) must survive byte-wise processing intact.
        assert_eq!(identity_key("café"), "café");
    }
}
