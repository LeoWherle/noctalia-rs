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

pub fn available_templates() -> Vec<AvailableTemplate> {
    vec![
        AvailableTemplate {
            id: "alacritty".to_string(),
            display_name: "Alacritty".to_string(),
            category: "Terminal".to_string(),
            output_paths: vec!["~/.config/alacritty/colors.toml".to_string()],
            output_dynamic: false,
        },
        AvailableTemplate {
            id: "kitty".to_string(),
            display_name: "Kitty".to_string(),
            category: "Terminal".to_string(),
            output_paths: vec!["~/.config/kitty/colors.conf".to_string()],
            output_dynamic: false,
        },
        AvailableTemplate {
            id: "foot".to_string(),
            display_name: "Foot".to_string(),
            category: "Terminal".to_string(),
            output_paths: vec!["~/.config/foot/colors.ini".to_string()],
            output_dynamic: false,
        },
    ]
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

pub fn community_templates_cache_dir() -> PathBuf {
    if let Some(val) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(val).join("noctalia").join("templates")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("noctalia")
            .join("templates")
    } else {
        PathBuf::from("/tmp/noctalia-cache/templates")
    }
}

pub fn community_template_dir(id: &str) -> PathBuf {
    community_templates_cache_dir().join(id)
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
            "# {template_id} theme\nprimary = {{{{ primary }}}}\nsurface = {{{{ surface }}}}\n"
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
}
