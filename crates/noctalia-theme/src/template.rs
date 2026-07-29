//! Template engine & renderer.
//! Port of `src/theme/template_engine.{cpp,h}`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::color::Color;
use crate::palette::GeneratedPalette;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderResult {
    pub text: String,
    pub error_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderFileResult {
    pub success: bool,
    pub wrote: bool,
    pub error_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub default_mode: String,
    pub image_path: String,
    pub closest_color: String,
    pub config_dir: String,
    pub config_file: String,
    pub enabled_templates: HashSet<String>,
    pub scheme_type: String,
    pub verbose: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            default_mode: "dark".to_string(),
            image_path: String::new(),
            closest_color: String::new(),
            config_dir: String::new(),
            config_file: String::new(),
            enabled_templates: HashSet::new(),
            scheme_type: "content".to_string(),
            verbose: true,
        }
    }
}

pub type ModeMap = HashMap<String, String>;
pub type ThemeData = HashMap<String, ModeMap>;

pub struct TemplateEngine {
    theme_data: ThemeData,
    options: Options,
}

impl TemplateEngine {
    pub fn new(theme_data: ThemeData) -> Self {
        Self {
            theme_data,
            options: Options::default(),
        }
    }

    pub fn with_options(theme_data: ThemeData, options: Options) -> Self {
        Self {
            theme_data,
            options,
        }
    }

    pub fn make_theme_data(palette: &GeneratedPalette) -> ThemeData {
        let mut data = ThemeData::new();

        let mut dark_map = ModeMap::new();
        for (k, &v) in &palette.dark {
            let color = Color::from_argb(v);
            dark_map.insert(k.clone(), color.to_hex());
            dark_map.insert(format!("{k}_hex"), color.to_hex());
            dark_map.insert(format!("{k}_raw"), format!("{:#08x}", v));
            let (r, g, b) = (color.r, color.g, color.b);
            dark_map.insert(format!("{k}_rgb"), format!("{r},{g},{b}"));
        }
        data.insert("dark".to_string(), dark_map);

        let mut light_map = ModeMap::new();
        for (k, &v) in &palette.light {
            let color = Color::from_argb(v);
            light_map.insert(k.clone(), color.to_hex());
            light_map.insert(format!("{k}_hex"), color.to_hex());
            light_map.insert(format!("{k}_raw"), format!("{:#08x}", v));
            let (r, g, b) = (color.r, color.g, color.b);
            light_map.insert(format!("{k}_rgb"), format!("{r},{g},{b}"));
        }
        data.insert("light".to_string(), light_map);

        data
    }

    pub fn render(&self, template_text: &str) -> RenderResult {
        let mut output = String::new();
        let mut error_count = 0;

        let mode = self
            .theme_data
            .get(&self.options.default_mode)
            .or_else(|| self.theme_data.get("dark"))
            .or_else(|| self.theme_data.get("light"));

        let empty_map = ModeMap::new();
        let bindings = mode.unwrap_or(&empty_map);

        let mut rest = template_text;
        while let Some(start) = rest.find("{{") {
            output.push_str(&rest[..start]);
            let after_open = &rest[start + 2..];

            if let Some(end) = after_open.find("}}") {
                let expr = after_open[..end].trim();
                rest = &after_open[end + 2..];

                if let Some(val) = evaluate_expression(expr, bindings, &self.options) {
                    output.push_str(&val);
                } else {
                    error_count += 1;
                    output.push_str(&format!("{{{{ {expr} }}}}"));
                }
            } else {
                output.push_str("{{");
                rest = after_open;
            }
        }
        output.push_str(rest);

        RenderResult {
            text: output,
            error_count,
        }
    }

    pub fn render_file(&self, input_path: &Path, output_path: &Path) -> RenderFileResult {
        let content = match fs::read_to_string(input_path) {
            Ok(c) => c,
            Err(_) => {
                return RenderFileResult {
                    success: false,
                    wrote: false,
                    error_count: 1,
                };
            }
        };

        let result = self.render(&content);
        if result.error_count > 0 {
            return RenderFileResult {
                success: false,
                wrote: false,
                error_count: result.error_count,
            };
        }

        if let Some(parent) = output_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let wrote = fs::write(output_path, &result.text).is_ok();

        RenderFileResult {
            success: wrote,
            wrote,
            error_count: result.error_count,
        }
    }
}

fn evaluate_expression(expr: &str, bindings: &ModeMap, options: &Options) -> Option<String> {
    let parts: Vec<&str> = expr.split('|').map(str::trim).collect();
    let var_name = parts[0];

    let mut value = match var_name {
        "image" | "wallpaper" => options.image_path.clone(),
        "scheme" => options.scheme_type.clone(),
        "closest_color" => options.closest_color.clone(),
        _ => bindings.get(var_name).cloned()?,
    };

    for &filter in &parts[1..] {
        value = apply_filter(&value, filter)?;
    }

    Some(value)
}

fn apply_filter(val: &str, filter: &str) -> Option<String> {
    if filter == "upper" || filter == "uppercase" {
        Some(val.to_uppercase())
    } else if filter == "lower" || filter == "lowercase" {
        Some(val.to_lowercase())
    } else if filter == "trim" || filter == "strip" {
        Some(val.trim().to_string())
    } else if filter.starts_with("default(") && filter.ends_with(')') {
        let fallback = filter[8..filter.len() - 1]
            .trim_matches('\'')
            .trim_matches('"');
        if val.is_empty() {
            Some(fallback.to_string())
        } else {
            Some(val.to_string())
        }
    } else if filter == "hex" {
        if let Ok(color) = Color::from_hex(val) {
            Some(color.to_hex())
        } else {
            Some(val.to_string())
        }
    } else {
        Some(val.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_render_replaces_variables() {
        let mut palette = GeneratedPalette::default();
        palette.dark.insert("primary".to_string(), 0xffe6b450);
        palette.dark.insert("surface".to_string(), 0xff0b0e14);

        let data = TemplateEngine::make_theme_data(&palette);
        let engine = TemplateEngine::new(data);

        let res = engine.render("color: {{ primary }}; background: {{ surface }};");
        assert_eq!(res.error_count, 0);
        assert!(res.text.contains("#e6b450"));
        assert!(res.text.contains("#0b0e14"));
    }

    #[test]
    fn template_render_applies_filters() {
        let mut palette = GeneratedPalette::default();
        palette.dark.insert("primary".to_string(), 0xffe6b450);

        let data = TemplateEngine::make_theme_data(&palette);
        let engine = TemplateEngine::new(data);

        let res = engine.render("color: {{ primary | lower }};");
        assert_eq!(res.error_count, 0);
        assert!(res.text.contains("#e6b450"));
    }
}
