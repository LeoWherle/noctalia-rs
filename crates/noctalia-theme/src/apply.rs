//! Template application service & built-in/community template catalogs.
//! Port of `src/theme/template_apply_service.{cpp,h}`, `builtin_templates.{cpp,h}`,
//! and `community_templates.{cpp,h}`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::palette::GeneratedPalette;
use crate::template::{Options, TemplateEngine};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinTemplateInfo {
    pub id: String,
    pub name: String,
    pub category: String,
    pub output_paths: Vec<String>,
    pub output_dynamic: bool,
    pub output_path_dynamic_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableTemplate {
    pub id: String,
    pub display_name: String,
    pub category: String,
    pub output_paths: Vec<String>,
    pub output_dynamic: bool,
}

/// Port of `loadBuiltinTemplateInfo` (`builtin_templates.cpp:11-84`), reading the shipped
/// `assets/templates/builtin.toml` catalog. Entries sorted by `(category, id)`.
pub fn load_builtin_template_info() -> Result<Vec<BuiltinTemplateInfo>, String> {
    let config_path = noctalia_core::files::paths::asset_path("templates/builtin.toml");
    let content = fs::read_to_string(&config_path).map_err(|err| err.to_string())?;
    let root: toml::Table = content
        .parse()
        .map_err(|err: toml::de::Error| err.to_string())?;

    let mut out = Vec::new();
    let Some(catalog) = root.get("catalog").and_then(|v| v.as_table()) else {
        return Ok(out);
    };
    for (id, node) in catalog {
        let Some(info) = node.as_table() else {
            continue;
        };
        let name = info
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let category = info
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push(BuiltinTemplateInfo {
            id: id.clone(),
            name,
            category,
            output_paths: Vec::new(),
            output_dynamic: false,
            output_path_dynamic_command: String::new(),
        });
    }

    if let Some(templates) = root.get("templates").and_then(|v| v.as_table()) {
        for entry in &mut out {
            let Some(tpl) = templates.get(&entry.id).and_then(|v| v.as_table()) else {
                continue;
            };
            if let Some(opd) = tpl.get("output_path_dynamic").and_then(|v| v.as_str()) {
                entry.output_dynamic = true;
                entry.output_path_dynamic_command = opd.to_string();
            }
            append_output_path_node(&mut entry.output_paths, tpl.get("output_path"));
        }
    }

    out.sort_by(|a, b| (&a.category, &a.id).cmp(&(&b.category, &b.id)));
    Ok(out)
}

/// Reads a TOML `output_path` node (string or array of strings) into `output_paths`. Shared by
/// `load_builtin_template_info` and `append_template_output_paths`
/// (`builtin_templates.cpp:56-67`, `community_templates.cpp:595-606`).
fn append_output_path_node(output_paths: &mut Vec<String>, node: Option<&toml::Value>) {
    let Some(node) = node else {
        return;
    };
    if let Some(s) = node.as_str() {
        output_paths.push(s.to_string());
    } else if let Some(arr) = node.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                output_paths.push(s.to_string());
            }
        }
    }
}

/// Port of `availableTemplates` (`builtin_templates.cpp:86-112`) — every built-in template the
/// user can opt into. A catalog parse failure is silently swallowed to an empty list, matching
/// the C++'s `err = nullptr` default-argument call convention at this call site.
pub fn available_templates() -> Vec<AvailableTemplate> {
    let entries = load_builtin_template_info().unwrap_or_default();
    let mut out: Vec<AvailableTemplate> = entries
        .into_iter()
        .map(|entry| {
            let display_name = if entry.name.is_empty() {
                entry.id.clone()
            } else {
                entry.name
            };
            AvailableTemplate {
                id: entry.id,
                display_name,
                category: entry.category,
                output_paths: entry.output_paths,
                output_dynamic: entry.output_dynamic,
            }
        })
        .collect();

    out.sort_by(|a, b| (&a.display_name, &a.id).cmp(&(&b.display_name, &b.id)));
    // `Vec::dedup_by` only removes *consecutive* duplicates, matching `std::ranges::unique`'s
    // same limitation — a no-op in practice since TOML table keys (the `id` source) are
    // inherently unique, ported as-is rather than "improved" into a full dedup.
    out.dedup_by(|a, b| a.id == b.id);
    out
}

fn catalog_cache_path() -> PathBuf {
    community_templates_cache_dir().join("catalog.json")
}

struct CommunityCatalogEntry {
    id: String,
    display_name: String,
    category: String,
}

/// Port of `stringField` (`community_templates.cpp:68-81`).
fn string_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> String {
    let read = |key: &str| -> String {
        if key.is_empty() {
            return String::new();
        }
        obj.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let value = read(snake);
    if value.is_empty() { read(camel) } else { value }
}

/// Port of `parseInfo` (`community_templates.cpp:242-256`). `files`/`entries` are intentionally
/// not parsed here — they only feed the sync/download pipeline (out of scope: `--list-templates`
/// is the only caller of this module's community-catalog reading, and it never reads them).
fn parse_catalog_info(obj: &serde_json::Value) -> Option<CommunityCatalogEntry> {
    let obj = obj.as_object()?;
    let id = string_field(obj, "name", "id");
    if id.is_empty() || !is_safe_community_template_id(&id) {
        return None;
    }
    let mut display_name = string_field(obj, "display_name", "displayName");
    if display_name.is_empty() {
        display_name = id.clone();
    }
    let category = string_field(obj, "category", "");
    Some(CommunityCatalogEntry {
        id,
        display_name,
        category,
    })
}

/// Port of `parseCatalogFile` (`community_templates.cpp:258-285`). Parse failures (missing file,
/// invalid JSON, non-array `templates`) are swallowed to an empty list, matching the C++ (which
/// only logs a warning, an omitted side effect that doesn't change any caller's control flow).
fn parse_catalog_file(path: &Path) -> Vec<CommunityCatalogEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };
    let entries = root
        .as_object()
        .and_then(|obj| obj.get("templates"))
        .cloned()
        .unwrap_or(root);
    let Some(array) = entries.as_array() else {
        return Vec::new();
    };
    array.iter().filter_map(parse_catalog_info).collect()
}

/// Port of `appendTemplateOutputPaths` (`community_templates.cpp:587-607`).
fn append_template_output_paths(output_paths: &mut Vec<String>, root: &toml::Table) {
    let Some(templates) = root.get("templates").and_then(|v| v.as_table()) else {
        return;
    };
    for node in templates.values() {
        let Some(tpl) = node.as_table() else { continue };
        append_output_path_node(output_paths, tpl.get("output_path"));
    }
}

/// Port of `appendOutputPathsFromCacheToml` (`community_templates.cpp:646-655`).
fn append_output_paths_from_cache_toml(t: &mut AvailableTemplate) {
    let toml_path = community_template_config_path(&t.id);
    let Ok(content) = fs::read_to_string(&toml_path) else {
        return;
    };
    let Ok(root) = content.parse::<toml::Table>() else {
        return;
    };
    append_template_output_paths(&mut t.output_paths, &root);
}

/// Port of `readTemplateTomlInfo` (`community_templates.cpp:609-644`).
fn read_template_toml_info(path: &Path, cache_id: &str) -> Option<AvailableTemplate> {
    if !is_safe_community_template_id(cache_id) {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let root: toml::Table = content.parse().ok()?;
    let catalog = root.get("catalog").and_then(|v| v.as_table())?;
    if catalog.is_empty() {
        return None;
    }

    let mut out = AvailableTemplate {
        id: cache_id.to_string(),
        display_name: cache_id.to_string(),
        category: String::new(),
        output_paths: Vec::new(),
        output_dynamic: false,
    };

    let Some(info) = catalog.get(cache_id).and_then(|v| v.as_table()) else {
        // Matches the C++: cache dir name has no matching `[catalog.<id>]` entry in its own
        // `template.toml` — falls back to the cache directory name (already `out`'s default)
        // rather than failing, after logging a warning this port omits (informational only, no
        // control-flow effect).
        append_template_output_paths(&mut out.output_paths, &root);
        return Some(out);
    };

    if let Some(name) = info.get("name").and_then(|v| v.as_str()) {
        out.display_name = name.to_string();
    }
    if let Some(category) = info.get("category").and_then(|v| v.as_str()) {
        out.category = category.to_string();
    }
    append_template_output_paths(&mut out.output_paths, &root);
    Some(out)
}

/// Port of `CommunityTemplateService::availableTemplates` (`community_templates.cpp:785-826`) —
/// catalog-listed templates plus any cached-but-not-in-catalog template directories. Sync/fetch
/// (`CommunityTemplateService::sync` and everything it calls) is out of scope: `--list-templates`
/// (this function's only caller) only reads what's already cached.
pub fn community_available_templates() -> Vec<AvailableTemplate> {
    let catalog = parse_catalog_file(&catalog_cache_path());
    let mut out: Vec<AvailableTemplate> = catalog
        .into_iter()
        .map(|info| {
            let display_name = if info.display_name.is_empty() {
                info.id.clone()
            } else {
                info.display_name
            };
            let mut t = AvailableTemplate {
                id: info.id,
                display_name,
                category: info.category,
                output_paths: Vec::new(),
                output_dynamic: false,
            };
            append_output_paths_from_cache_toml(&mut t);
            t
        })
        .collect();

    let cache_dir = community_templates_cache_dir();
    let Ok(read_dir) = fs::read_dir(&cache_dir) else {
        return out;
    };
    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let cache_id = entry.file_name().to_string_lossy().to_string();
        if out.iter().any(|t| t.id == cache_id) {
            continue;
        }
        let toml_path = entry.path().join("template.toml");
        if !toml_path.exists() {
            continue;
        }
        if let Some(info) = read_template_toml_info(&toml_path, &cache_id) {
            out.push(info);
        }
    }

    out.sort_by(|a, b| {
        (&a.category, &a.display_name, &a.id).cmp(&(&b.category, &b.display_name, &b.id))
    });
    out
}

pub fn format_template_tooltip(template: &AvailableTemplate) -> String {
    if template.output_paths.is_empty() && !template.output_dynamic {
        return String::new();
    }
    let mut tip = String::new();
    for path in &template.output_paths {
        if !tip.is_empty() {
            tip.push('\n');
        }
        tip.push_str(path);
    }
    if template.output_dynamic {
        if !tip.is_empty() {
            tip.push('\n');
        }
        tip.push_str("Dynamic Output");
    }
    tip
}

pub fn is_safe_community_template_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Port of `communityTemplatesCacheDir` (`community_templates.cpp:830-835`).
///
/// Fixed prior to task 4.2.7: this previously used `$XDG_CACHE_HOME`/`~/.cache` — the C++ stores
/// this under the **state** dir instead ("app-managed, re-fetchable data that should persist
/// between sessions and not be auto-reclaimed by cache cleaners", per the C++'s own comment), not
/// XDG cache. No existing test pinned the old (wrong) path, so this is a correctness fix, found
/// while porting `--list-templates`, which reads real cached community templates from here.
pub fn community_templates_cache_dir() -> PathBuf {
    let state = noctalia_core::files::paths::state_dir();
    if !state.is_empty() {
        PathBuf::from(state).join("community-templates")
    } else {
        PathBuf::from("/tmp/noctalia/community-templates")
    }
}

pub fn community_template_dir(id: &str) -> PathBuf {
    community_templates_cache_dir().join(id)
}

pub fn community_template_config_path(id: &str) -> PathBuf {
    community_template_dir(id).join("template.toml")
}

pub fn apply_templates_dry_run(
    palette: &GeneratedPalette,
    default_mode: &str,
    target_dir: &Path,
    enabled_templates: &[String],
) -> Vec<PathBuf> {
    let mut generated_files = Vec::new();
    let theme_data = TemplateEngine::make_theme_data(palette);

    let options = Options {
        default_mode: default_mode.to_string(),
        enabled_templates: enabled_templates.iter().cloned().collect(),
        ..Default::default()
    };

    let engine = TemplateEngine::with_options(theme_data, options);

    let _ = fs::create_dir_all(target_dir);

    for template_id in enabled_templates {
        let out_file = target_dir.join(format!("{template_id}.conf"));
        let sample_template = format!(
            "# {template_id} theme\nprimary = {{{{colors.primary.default.hex}}}}\nsurface = {{{{colors.surface.default.hex}}}}\n"
        );
        let result = engine.render(&sample_template);
        if result.error_count == 0 && fs::write(&out_file, &result.text).is_ok() {
            generated_files.push(out_file);
        }
    }

    generated_files
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn safe_community_template_id_validation() {
        assert!(is_safe_community_template_id("my-template"));
        assert!(is_safe_community_template_id("template_123"));
        assert!(!is_safe_community_template_id("../evil"));
        assert!(!is_safe_community_template_id("foo/bar"));
    }

    #[test]
    fn dry_run_apply_generates_files_in_tempdir() {
        let temp_dir = std::env::temp_dir().join(format!("noctalia-apply-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        let mut palette = GeneratedPalette::default();
        palette.dark.insert("primary".to_string(), 0xffe6b450);
        palette.dark.insert("surface".to_string(), 0xff0b0e14);

        let enabled = vec!["alacritty".to_string(), "kitty".to_string()];
        let files = apply_templates_dry_run(&palette, "dark", &temp_dir, &enabled);

        assert_eq!(files.len(), 2);
        assert!(files[0].exists());
        assert!(files[1].exists());

        let content = fs::read_to_string(&files[0]).unwrap();
        assert!(content.contains("primary = #e6b450"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_builtin_template_info_reads_real_asset_catalog() {
        let entries = load_builtin_template_info().expect("builtin.toml should parse");
        assert!(!entries.is_empty());
        let alacritty = entries
            .iter()
            .find(|e| e.id == "alacritty")
            .expect("alacritty entry");
        assert_eq!(alacritty.name, "Alacritty");
        assert_eq!(alacritty.category, "terminal");
        assert!(!alacritty.output_paths.is_empty());
        // Sorted by (category, id): verify no adjacent pair violates that ordering.
        for pair in entries.windows(2) {
            let a = (&pair[0].category, &pair[0].id);
            let b = (&pair[1].category, &pair[1].id);
            assert!(a <= b, "not sorted: {a:?} > {b:?}");
        }
    }

    #[test]
    fn available_templates_uses_real_catalog_not_hardcoded_stub() {
        let templates = available_templates();
        assert!(templates.iter().any(|t| t.id == "kitty"));
        // The old hardcoded stub had exactly 3 entries; the real catalog has many more.
        assert!(templates.len() > 3);
    }

    #[test]
    fn parse_catalog_file_handles_top_level_array_shape() {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-community-catalog-array-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalog.json");
        fs::write(
            &path,
            r#"[{"name": "my-template", "category": "terminal"}]"#,
        )
        .unwrap();

        let entries = parse_catalog_file(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "my-template");
        assert_eq!(entries[0].display_name, "my-template");
        assert_eq!(entries[0].category, "terminal");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_catalog_file_handles_templates_wrapped_shape() {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-community-catalog-wrapped-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalog.json");
        fs::write(
            &path,
            r#"{"templates": [{"name": "abc", "display_name": "ABC"}]}"#,
        )
        .unwrap();

        let entries = parse_catalog_file(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "abc");
        assert_eq!(entries[0].display_name, "ABC");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_catalog_file_rejects_unsafe_ids() {
        let dir = std::env::temp_dir().join(format!(
            "noctalia-community-catalog-unsafe-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalog.json");
        fs::write(&path, r#"[{"name": "../evil"}]"#).unwrap();

        assert!(parse_catalog_file(&path).is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_catalog_file_missing_file_returns_empty() {
        assert!(parse_catalog_file(Path::new("/does/not/exist.json")).is_empty());
    }

    #[test]
    fn community_available_templates_discovers_uncataloged_cached_dirs() {
        let dir =
            std::env::temp_dir().join(format!("noctalia-community-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // SAFETY: test-only env mutation; noctalia-core's own tests already establish this
        // pattern for XDG-style env vars (see `noctalia_core::files::paths` tests).
        unsafe {
            std::env::set_var("NOCTALIA_STATE_HOME", &dir);
        }

        let cached = community_templates_cache_dir().join("mytemplate");
        fs::create_dir_all(&cached).unwrap();
        fs::write(
            cached.join("template.toml"),
            "[catalog.mytemplate]\nname = \"My Template\"\ncategory = \"editor\"\n",
        )
        .unwrap();

        let templates = community_available_templates();
        assert!(templates.iter().any(|t| t.id == "mytemplate"
            && t.display_name == "My Template"
            && t.category == "editor"));

        unsafe {
            std::env::remove_var("NOCTALIA_STATE_HOME");
        }
        let _ = fs::remove_dir_all(dir);
    }
}
