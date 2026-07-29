//! Application theme outputs (KDE color scheme, Firefox theme, JSON export).
//! Port of `src/theme/kde_color_scheme.{cpp,h}`, `src/theme/json_output.{cpp,h}`,
//! and `src/theme/firefox_theme/*`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::color::Color;
use crate::palette::GeneratedPalette;
use crate::scheme::Scheme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Variant {
    Dark,
    Light,
    Both,
}

pub fn hex_string(argb: u32) -> String {
    let color = Color::from_argb(argb);
    color.to_hex()
}

pub fn to_json(palette: &GeneratedPalette, _scheme: Scheme, variant: Variant) -> String {
    match variant {
        Variant::Dark => serde_json::to_string_pretty(&palette.dark).unwrap_or_default(),
        Variant::Light => serde_json::to_string_pretty(&palette.light).unwrap_or_default(),
        Variant::Both => serde_json::to_string_pretty(&serde_json::json!({
            "dark": palette.dark,
            "light": palette.light,
        }))
        .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdeColorSchemeApplyResult {
    pub success: bool,
    pub error: String,
    pub notification_error: String,
}

pub fn merge_kde_color_scheme(scheme_path: &Path, kde_globals_path: &Path) -> Result<(), String> {
    let scheme_content = fs::read_to_string(scheme_path)
        .map_err(|e| format!("cannot read scheme file {}: {e}", scheme_path.display()))?;

    if let Some(parent) = kde_globals_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    fs::write(kde_globals_path, &scheme_content).map_err(|e| {
        format!(
            "cannot write kde globals {}: {e}",
            kde_globals_path.display()
        )
    })?;

    Ok(())
}

pub fn apply_kde_color_scheme(scheme_path: &Path) -> KdeColorSchemeApplyResult {
    let kde_globals = match std::env::var_os("HOME") {
        Some(home) => Path::new(&home).join(".config/kdeglobals"),
        None => Path::new("/tmp/kdeglobals").to_path_buf(),
    };

    match merge_kde_color_scheme(scheme_path, &kde_globals) {
        Ok(_) => KdeColorSchemeApplyResult {
            success: true,
            error: String::new(),
            notification_error: String::new(),
        },
        Err(err) => KdeColorSchemeApplyResult {
            success: false,
            error: err,
            notification_error: String::new(),
        },
    }
}

pub fn generate_firefox_theme_css(palette: &GeneratedPalette, is_dark: bool) -> String {
    let tokens = if is_dark {
        &palette.dark
    } else {
        &palette.light
    };

    let get_color = |key: &str| -> String {
        tokens
            .get(key)
            .copied()
            .map(hex_string)
            .unwrap_or_else(|| "#000000".to_string())
    };

    format!(
        ":root {{\n  --toolbar-field-background-color: {};\n  --toolbar-field-color: {};\n  --frame: {};\n  --tab-selected: {};\n}}\n",
        get_color("surface_variant"),
        get_color("on_surface_variant"),
        get_color("surface"),
        get_color("primary")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn to_json_serializes_palette_to_json() {
        let mut palette = GeneratedPalette::default();
        palette.dark.insert("primary".to_string(), 0xffe6b450);
        palette.light.insert("primary".to_string(), 0xffff8f40);

        let json_both = to_json(&palette, Scheme::TonalSpot, Variant::Both);
        assert!(json_both.contains("\"dark\""));
        assert!(json_both.contains("\"light\""));

        let json_dark = to_json(&palette, Scheme::TonalSpot, Variant::Dark);
        assert!(json_dark.contains("primary"));
    }

    #[test]
    fn merge_kde_color_scheme_writes_to_destination() {
        let temp_dir = std::env::temp_dir().join(format!("noctalia-kde-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let scheme_file = temp_dir.join("scheme.colors");
        let target_file = temp_dir.join("kdeglobals");

        fs::write(&scheme_file, "[Colors:Window]\nBackground=11,14,20\n").unwrap();
        assert!(merge_kde_color_scheme(&scheme_file, &target_file).is_ok());
        assert!(target_file.exists());

        let content = fs::read_to_string(&target_file).unwrap();
        assert!(content.contains("[Colors:Window]"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn firefox_theme_css_generation() {
        let mut palette = GeneratedPalette::default();
        palette.dark.insert("surface".to_string(), 0xff0b0e14);
        palette.dark.insert("primary".to_string(), 0xffe6b450);

        let css = generate_firefox_theme_css(&palette, true);
        assert!(css.contains("--frame: #0b0e14"));
        assert!(css.contains("--tab-selected: #e6b450"));
    }
}
