//! Port of `src/system/internal_app_metadata.{cpp,h}` (task 5.5.1): display name/icon overrides
//! for Noctalia's own built-in windows (e.g. the settings window), applied to a `DesktopEntry`
//! that has no real matching `.desktop` file.

use noctalia_core::files::paths::asset_path;

use crate::desktop_entry::DesktopEntry;

/// Port of `AppMetadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMetadata {
    pub display_name: String,
    pub icon_path: String,
}

/// Port of `InternalAppDefinition`.
#[derive(Debug, Clone, Copy)]
pub struct InternalAppDefinition {
    pub app_id: &'static str,
    pub window_title: &'static str,
    pub display_name: &'static str,
    pub icon_asset_path: &'static str,
}

/// Port of `kInternalApps`.
const INTERNAL_APPS: &[InternalAppDefinition] = &[InternalAppDefinition {
    app_id: "dev.noctalia.Noctalia",
    window_title: "Noctalia Settings",
    display_name: "Noctalia",
    icon_asset_path: "noctalia.svg",
}];

/// Port of `appDefinitionForAppId`.
pub fn app_definition_for_app_id(app_id: &str) -> Option<&'static InternalAppDefinition> {
    INTERNAL_APPS.iter().find(|app| app.app_id == app_id)
}

/// Port of `appDefinitionForWindowTitle`.
pub fn app_definition_for_window_title(
    window_title: &str,
) -> Option<&'static InternalAppDefinition> {
    INTERNAL_APPS
        .iter()
        .find(|app| !app.window_title.is_empty() && app.window_title == window_title)
}

/// Port of `definitionForDesktopEntry`.
pub fn definition_for_desktop_entry(
    entry: &DesktopEntry,
) -> Option<&'static InternalAppDefinition> {
    if let Some(app) = app_definition_for_app_id(&entry.id) {
        return Some(app);
    }
    if !entry.startup_wm_class.is_empty()
        && let Some(app) = app_definition_for_app_id(&entry.startup_wm_class)
    {
        return Some(app);
    }
    app_definition_for_window_title(&entry.name)
}

/// Port of `metadataFromDefinition`.
fn metadata_from_definition(app: &InternalAppDefinition) -> AppMetadata {
    AppMetadata {
        display_name: app.display_name.to_string(),
        icon_path: asset_path(app.icon_asset_path)
            .to_string_lossy()
            .to_string(),
    }
}

/// Port of `metadataForAppId`.
pub fn metadata_for_app_id(app_id: &str) -> Option<AppMetadata> {
    app_definition_for_app_id(app_id).map(metadata_from_definition)
}

/// Port of `metadataForDesktopEntry`.
pub fn metadata_for_desktop_entry(entry: &DesktopEntry) -> Option<AppMetadata> {
    let app = definition_for_desktop_entry(entry)?;
    metadata_for_app_id(app.app_id)
}

/// Port of `applyMetadataToDesktopEntry`.
pub fn apply_metadata_to_desktop_entry(entry: &mut DesktopEntry) {
    let Some(meta) = metadata_for_desktop_entry(entry) else {
        return;
    };
    if entry.icon.is_empty() {
        entry.icon = meta.icon_path;
    }
    if entry.name == entry.id {
        entry.name = meta.display_name.clone();
        entry.name_lower = meta.display_name.to_ascii_lowercase();
    }
}
