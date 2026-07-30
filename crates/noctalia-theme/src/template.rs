//! Template engine & renderer.
//! Port of `src/theme/template_engine.{cpp,h}`'s `TemplateEngine::render`/`renderFile` and
//! their private `EngineImpl` (tokenizer, `<* for/if *>` block parser, `{{expr}}` expression
//! resolver, and full filter set). `TemplateEngine::applyCustomColors`/`processConfigTemplates`
//! (the `[templates]`/`[config.custom_colors]` orchestration on top of this) are task 4.2.6b.2,
//! not yet ported.
//!
//! Divergences from the C++ (all deliberate, see MIGRATION_PLAN.md task 4.2.6b.1):
//! - `std::regex` has no C ABI to bind against (same "C++-only dependency" exception as
//!   toml/serde_json/zbus in the dependency strategy) — uses the `regex` crate instead.
//! - `StringUtils::trim`/`toLower` aren't ported yet (`string_utils.h` has no owning task);
//!   the ASCII-whitespace `trim` this file needs is reimplemented locally (`c_trim`, same
//!   pattern as `noctalia-ipc::arg_parse`'s `c_trim`), `toLower` maps directly onto
//!   `str::to_ascii_lowercase` (equivalent for the "C" locale `std::tolower` the C++ uses).
//! - `findClosestColor`/Lab distance (used by `compare_to`/`colors_to_compare` template
//!   entries) is deferred to 4.2.6b.2, the only place that calls it.
//! - `resolveFromScope`'s color-format branch and `getPaletteEntries`/`processColorExpression`
//!   fall back to returning "unresolved" on a malformed hex string instead of the C++'s
//!   uncaught `Color::fromHex` throw (which would `std::terminate` the whole process) —
//!   unreachable in practice since theme data hex strings always round-trip through
//!   `Color::toHex`, but a strictly safer failure mode than a crash.
//! - `buildColorsMap`'s memoization (`m_colorsMap`) is dropped; it's recomputed on each
//!   `for k, v in colors` use instead of once per render — a pure performance difference,
//!   not a behavioral one.
//! - `parse_for`'s variable-list split on a trailing comma (`for x, y, in expr`, a template
//!   author typo) keeps a trailing empty-string variable name where the C++'s
//!   `std::getline(vars, item, ',')` silently drops it — unreachable in practice (none of the
//!   real templates under `assets/templates/` use `for`/`if` at all today, per `grep`).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use material_colors::color::Argb;
use material_colors::hct::Hct;
use material_colors::palette::TonalPalette;

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

    /// Port of `TemplateEngine::makeThemeData`: one hex string per token per mode, no derived
    /// suffix keys — `.hex`/`.rgb`/etc. are format *filters* applied at render time via
    /// `colors.<name>.<mode>.<format>`, not precomputed map entries.
    pub fn make_theme_data(palette: &GeneratedPalette) -> ThemeData {
        let mut data = ThemeData::new();

        let mut dark_map = ModeMap::new();
        for (k, &v) in &palette.dark {
            dark_map.insert(k.clone(), Color::from_argb(v).to_hex());
        }
        data.insert("dark".to_string(), dark_map);

        let mut light_map = ModeMap::new();
        for (k, &v) in &palette.light {
            light_map.insert(k.clone(), Color::from_argb(v).to_hex());
        }
        data.insert("light".to_string(), light_map);

        data
    }

    pub fn render(&self, template_text: &str) -> RenderResult {
        Evaluator::new(&self.theme_data, &self.options).render(template_text)
    }

    pub fn render_file(&self, input_path: &Path, output_path: &Path) -> RenderFileResult {
        Evaluator::new(&self.theme_data, &self.options).render_file(input_path, output_path)
    }
}

// ---------------------------------------------------------------------------------------------
// Internals — port of template_engine.cpp's anonymous namespace + private `EngineImpl`.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct RichColor {
    color: Color,
    alpha: f64,
}

type ScopeArray = Vec<ScopeValue>;
type ScopeMap = HashMap<String, ScopeValue>;

#[derive(Clone, Debug, Default)]
enum ScopeValue {
    #[default]
    None,
    Bool(bool),
    Number(f64),
    Str(String),
    Color(RichColor),
    // Mirrors the C++ variant's `ScopeArray` member (`resolveIterable`'s "scope value is
    // already an array" branch reads it) — no call site in this task constructs one either,
    // in the Rust port or the C++ original; kept for the sum type's exhaustiveness.
    #[allow(dead_code)]
    Array(ScopeArray),
    Map(ScopeMap),
}

#[derive(Clone, Debug, Default)]
struct ForNode {
    variables: Vec<String>,
    iterable: String,
    body: Vec<Node>,
}

#[derive(Clone, Debug)]
struct IfNode {
    condition_expr: String,
    negated: bool,
    then_body: Vec<Node>,
    else_body: Vec<Node>,
}

#[derive(Clone, Debug)]
enum Node {
    Text(String),
    For(ForNode),
    If(IfNode),
}

#[derive(Clone, Debug)]
enum Token {
    Text(String),
    Block(String),
}

#[derive(Default)]
struct VariableScope {
    scopes: Vec<ScopeMap>,
}

impl VariableScope {
    fn push(&mut self, bindings: ScopeMap) {
        self.scopes.push(bindings);
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn set(&mut self, name: String, value: ScopeValue) {
        if self.scopes.is_empty() {
            self.scopes.push(ScopeMap::new());
        }
        if let Some(last) = self.scopes.last_mut() {
            last.insert(name, value);
        }
    }

    fn get(&self, name: &str) -> Option<&ScopeValue> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }
}

const KNOWN_FORMATS: &[&str] = &[
    "hex",
    "hex_stripped",
    "rgb",
    "rgb_csv",
    "rgba",
    "hsl",
    "hsla",
    "red",
    "green",
    "blue",
    "alpha",
    "hue",
    "saturation",
    "lightness",
];

const SUPPORTED_FILTERS: &[&str] = &[
    "grayscale",
    "invert",
    "set_alpha",
    "set_lightness",
    "set_hue",
    "set_saturation",
    "set_red",
    "set_green",
    "set_blue",
    "lighten",
    "darken",
    "saturate",
    "desaturate",
    "auto_lightness",
    "rotate_hue",
];

const COLOR_ARG_FILTERS: &[&str] = &["blend", "harmonize"];

const PALETTE_TONES: [i32; 18] = [
    0, 5, 10, 15, 20, 25, 30, 35, 40, 50, 60, 70, 80, 90, 95, 98, 99, 100,
];

fn color_alias(name: &str) -> &str {
    match name {
        "hover" => "surface_container_high",
        "on_hover" => "on_surface",
        other => other,
    }
}

/// All hardcoded, compile-time-constant patterns below; a bad pattern is caught by any test
/// run (never by runtime template input), so `expect` here can't turn into a user-facing panic.
#[allow(clippy::expect_used)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("hardcoded regex pattern must compile")
}

static BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?s)<\*(.*?)\*>"));
static EXPR_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"\{\{([^}\n]+?)\}\}"));
static FOR_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"^for\s+(.+?)\s+in\s+(.+)$"));
static IF_EXPR_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"^\{\{(.+?)\}\}$"));
static COLOR_EXPR_RE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"^colors\.([a-z_0-9]+)\.([a-z_0-9]+)\.([a-z_0-9]+)$"));
static RANGE_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"^(-?\d+)\.\.(-?\d+)$"));
static FILTER_NAME_ARG_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"^([a-z_]+)\s*:\s*(.+)$"));
static REPLACE_DQ_RE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r#"^"([^"]*?)"\s*,\s*"([^"]*?)"$"#));
static REPLACE_SQ_RE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"^'([^']*?)'\s*,\s*'([^']*?)'$"));
static CAMEL_BOUNDARY_RE: LazyLock<Regex> = LazyLock::new(|| compiled(r"([a-z])([A-Z])"));
// Unanchored + search (not full-match) on purpose — matches the C++'s `regex_search`, unlike
// every other regex above (all matched via `regex_match`, hence the `^...$` anchors).
static COLOR_ARG_RE: LazyLock<Regex> =
    LazyLock::new(|| compiled(r#"["']?(#[0-9a-fA-F]{6})["']?\s*(?:,\s*(.+))?"#));

/// Matches `std::isspace` in the (always-active, un-localized) "C" locale: space, tab, LF,
/// vertical tab, form feed, CR — not the same set as Rust's Unicode-aware `str::trim`.
/// Same pattern as `noctalia-ipc::arg_parse::is_c_isspace`.
fn is_c_isspace(b: u8) -> bool {
    matches!(b, b' ' | 0x09..=0x0d)
}

fn c_trim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_c_isspace(bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && is_c_isspace(bytes[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

fn parse_dot_decimal(text: &str) -> Option<f64> {
    let trimmed = c_trim(text);
    if trimmed.is_empty() || trimmed.starts_with('+') {
        return None;
    }
    let value: f64 = trimmed.parse().ok()?;
    value.is_finite().then_some(value)
}

fn format_dot_decimal(value: f64) -> String {
    format!("{value}")
}

fn scope_value_to_string(value: &ScopeValue) -> String {
    match value {
        ScopeValue::Str(s) => s.clone(),
        ScopeValue::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        ScopeValue::Number(n) => {
            let rounded = n.round();
            if (n - rounded).abs() < 1.0e-9 {
                format!("{}", rounded as i64)
            } else {
                format_dot_decimal(*n)
            }
        }
        ScopeValue::Color(c) => c.color.to_hex(),
        ScopeValue::None | ScopeValue::Array(_) | ScopeValue::Map(_) => String::new(),
    }
}

fn is_truthy(value: &ScopeValue) -> bool {
    match value {
        ScopeValue::None => false,
        ScopeValue::Bool(b) => *b,
        ScopeValue::Number(n) => *n != 0.0,
        ScopeValue::Str(s) => {
            let lowered = c_trim(s).to_ascii_lowercase();
            !(lowered.is_empty() || lowered == "false" || lowered == "0" || lowered == "none")
        }
        ScopeValue::Array(a) => !a.is_empty(),
        ScopeValue::Map(m) => !m.is_empty(),
        ScopeValue::Color(_) => true,
    }
}

fn as_rich_color(value: &ScopeValue) -> Result<RichColor, ()> {
    match value {
        ScopeValue::Color(c) => Ok(c.clone()),
        ScopeValue::Str(s) => Color::from_hex(s)
            .map(|color| RichColor { color, alpha: 1.0 })
            .map_err(|_| ()),
        _ => Err(()),
    }
}

fn format_color(color: &RichColor, format_type: &str) -> String {
    match format_type {
        "hex" => color.color.to_hex(),
        "hex_stripped" => {
            let hex = color.color.to_hex();
            hex.strip_prefix('#').unwrap_or(&hex).to_string()
        }
        "rgb" => format!(
            "rgb({}, {}, {})",
            color.color.r, color.color.g, color.color.b
        ),
        "rgb_csv" => format!("{},{},{}", color.color.r, color.color.g, color.color.b),
        "rgba" => format!(
            "rgba({}, {}, {}, {})",
            color.color.r,
            color.color.g,
            color.color.b,
            format_dot_decimal(color.alpha)
        ),
        "hsl" | "hsla" => {
            let (h, s, l) = color.color.to_hsl();
            if format_type == "hsl" {
                format!(
                    "hsl({}, {}%, {}%)",
                    h as i64,
                    (s * 100.0) as i64,
                    (l * 100.0) as i64
                )
            } else {
                format!(
                    "hsla({}, {}%, {}%, {})",
                    h as i64,
                    (s * 100.0) as i64,
                    (l * 100.0) as i64,
                    format_dot_decimal(color.alpha)
                )
            }
        }
        "hue" => {
            let (h, _, _) = color.color.to_hsl();
            format!("{}", h as i64)
        }
        "saturation" => {
            let (_, s, _) = color.color.to_hsl();
            format!("{}", (s * 100.0) as i64)
        }
        "lightness" => {
            let (_, _, l) = color.color.to_hsl();
            format!("{}", (l * 100.0) as i64)
        }
        "red" => color.color.r.to_string(),
        "green" => color.color.g.to_string(),
        "blue" => color.color.b.to_string(),
        "alpha" => format_dot_decimal(color.alpha),
        _ => color.color.to_hex(),
    }
}

fn split_pipes(expr: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '\0';
    for ch in expr.chars() {
        if (ch == '"' || ch == '\'') && !in_quotes {
            in_quotes = true;
            quote_char = ch;
            current.push(ch);
        } else if ch == quote_char && in_quotes {
            in_quotes = false;
            quote_char = '\0';
            current.push(ch);
        } else if ch == '|' && !in_quotes {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn parse_filter(filter_str: &str) -> (String, Option<String>) {
    let filter_str = c_trim(filter_str);
    if let Some(caps) = FILTER_NAME_ARG_RE.captures(filter_str) {
        return (caps[1].to_string(), Some(c_trim(&caps[2]).to_string()));
    }
    if let Some(space_idx) = filter_str.find([' ', '\t']) {
        return (
            c_trim(&filter_str[..space_idx]).to_string(),
            Some(c_trim(&filter_str[space_idx + 1..]).to_string()),
        );
    }
    (filter_str.to_string(), None)
}

fn parse_number(arg: Option<&str>) -> Result<f64, ()> {
    match arg {
        None => Ok(0.0),
        Some(a) => parse_dot_decimal(a).ok_or(()),
    }
}

fn split_words(s: &str) -> Vec<String> {
    let replaced = CAMEL_BOUNDARY_RE.replace_all(s, "${1}_${2}");
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in replaced.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn apply_replace(value: &str, arg: Option<&str>) -> String {
    let Some(arg) = arg else {
        return value.to_string();
    };
    if let Some(caps) = REPLACE_DQ_RE.captures(arg) {
        return value.replace(&caps[1], &caps[2]);
    }
    if let Some(caps) = REPLACE_SQ_RE.captures(arg) {
        return value.replace(&caps[1], &caps[2]);
    }
    value.to_string()
}

fn capitalize_ascii(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn to_camel_case(value: &str) -> String {
    let words = split_words(value);
    let Some(first) = words.first() else {
        return value.to_string();
    };
    let mut out = first.to_ascii_lowercase();
    for word in &words[1..] {
        out.push_str(&capitalize_ascii(&word.to_ascii_lowercase()));
    }
    out
}

fn to_pascal_case(value: &str) -> String {
    let words = split_words(value);
    if words.is_empty() {
        return value.to_string();
    }
    words
        .iter()
        .map(|w| capitalize_ascii(&w.to_ascii_lowercase()))
        .collect()
}

fn join_lower(value: &str, sep: &str) -> String {
    let words = split_words(value);
    words
        .iter()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(sep)
}

fn apply_color_arg_filter(
    mut value: RichColor,
    name: &str,
    arg: Option<&str>,
) -> Result<RichColor, ()> {
    let Some(arg) = arg else {
        return Ok(value);
    };
    let Some(caps) = COLOR_ARG_RE.captures(arg) else {
        return Ok(value);
    };
    let target = Color::from_hex(&caps[1]).map_err(|_| ())?;
    let (src_hue, src_sat, src_light) = value.color.to_hsl();
    let (target_hue, _target_sat, _target_light) = target.to_hsl();
    let mut diff = target_hue - src_hue;
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    let new_hue = if name == "blend" {
        let amount_arg = caps.get(2).map(|m| m.as_str());
        let amount = parse_number(amount_arg)?.clamp(0.0, 1.0);
        (src_hue + diff * amount + 360.0) % 360.0
    } else {
        let mut rotation = (diff.abs() * 0.5).min(15.0);
        if diff < 0.0 {
            rotation = -rotation;
        }
        (src_hue + rotation + 360.0) % 360.0
    };
    value.color = Color::from_hsl(new_hue, src_sat, src_light);
    Ok(value)
}

fn apply_color_filter(
    mut color: RichColor,
    name: &str,
    arg: Option<&str>,
) -> Result<RichColor, ()> {
    let (h, s, l) = color.color.to_hsl();
    match name {
        "grayscale" => {
            let gray = (0.299 * color.color.r as f64
                + 0.587 * color.color.g as f64
                + 0.114 * color.color.b as f64)
                .round() as u8;
            color.color = Color::new(gray, gray, gray);
            return Ok(color);
        }
        "invert" => {
            color.color = Color::new(
                255 - color.color.r,
                255 - color.color.g,
                255 - color.color.b,
            );
            return Ok(color);
        }
        _ => {}
    }
    let num_arg = parse_number(arg)?;
    match name {
        "set_alpha" => color.alpha = num_arg.clamp(0.0, 1.0),
        "set_lightness" => color.color = Color::from_hsl(h, s, (num_arg / 100.0).clamp(0.0, 1.0)),
        "set_hue" => color.color = Color::from_hsl((num_arg + 360.0) % 360.0, s, l),
        "rotate_hue" => color.color = Color::from_hsl((h + num_arg + 360.0) % 360.0, s, l),
        "set_saturation" => color.color = Color::from_hsl(h, (num_arg / 100.0).clamp(0.0, 1.0), l),
        "lighten" => color.color = Color::from_hsl(h, s, (l + num_arg / 100.0).clamp(0.0, 1.0)),
        "darken" => color.color = Color::from_hsl(h, s, (l - num_arg / 100.0).clamp(0.0, 1.0)),
        "saturate" => color.color = Color::from_hsl(h, (s + num_arg / 100.0).clamp(0.0, 1.0), l),
        "desaturate" => color.color = Color::from_hsl(h, (s - num_arg / 100.0).clamp(0.0, 1.0), l),
        "auto_lightness" => {
            let target = if l < 0.5 {
                l + num_arg / 100.0
            } else {
                l - num_arg / 100.0
            };
            color.color = Color::from_hsl(h, s, target.clamp(0.0, 1.0));
        }
        "set_red" => color.color.r = (num_arg.round() as i64).clamp(0, 255) as u8,
        "set_green" => color.color.g = (num_arg.round() as i64).clamp(0, 255) as u8,
        "set_blue" => color.color.b = (num_arg.round() as i64).clamp(0, 255) as u8,
        _ => {}
    }
    Ok(color)
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut last_end = 0usize;
    for caps in BLOCK_RE.captures_iter(text) {
        let Some(whole) = caps.get(0) else { continue };
        let (start, end) = (whole.start(), whole.end());
        let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let mut adjusted_start = start;
        let mut adjusted_end = end;
        if c_trim(&text[line_start..start]).is_empty() {
            let bytes = text.as_bytes();
            let mut after_end = end;
            while after_end < text.len() && (bytes[after_end] == b' ' || bytes[after_end] == b'\t')
            {
                after_end += 1;
            }
            if after_end == text.len() || bytes[after_end] == b'\n' {
                adjusted_start = line_start;
                adjusted_end = if after_end == text.len() {
                    after_end
                } else {
                    after_end + 1
                };
            }
        }
        if adjusted_start > last_end {
            tokens.push(Token::Text(text[last_end..adjusted_start].to_string()));
        }
        let group1 = caps.get(1).map(|g| g.as_str()).unwrap_or("");
        tokens.push(Token::Block(c_trim(group1).to_string()));
        last_end = adjusted_end;
    }
    if last_end < text.len() {
        tokens.push(Token::Text(text[last_end..].to_string()));
    }
    tokens
}

fn resolve_from_scope(base: &str, scope: &VariableScope) -> ScopeValue {
    let parts: Vec<&str> = base.split('.').collect();
    if parts.is_empty() {
        return ScopeValue::None;
    }
    let Some(current) = scope.get(parts[0]) else {
        return ScopeValue::None;
    };
    let mut value = current.clone();
    for &part in &parts[1..] {
        match &value {
            ScopeValue::Map(map) => {
                let Some(v) = map.get(part) else {
                    return ScopeValue::None;
                };
                value = v.clone();
            }
            ScopeValue::Str(s) => {
                if s.len() >= 7 && s.starts_with('#') && KNOWN_FORMATS.contains(&part) {
                    let rich = RichColor {
                        color: Color::from_hex(s).unwrap_or(Color::BLACK),
                        alpha: 1.0,
                    };
                    return ScopeValue::Str(format_color(&rich, part));
                }
                return ScopeValue::None;
            }
            _ => return ScopeValue::None,
        }
    }
    value
}

fn resolve_iterable_range(expr: &str) -> Option<ScopeArray> {
    let caps = RANGE_RE.captures(expr)?;
    let start: i64 = caps[1].parse().ok()?;
    let end: i64 = caps[2].parse().ok()?;
    Some((start..end).map(|i| ScopeValue::Number(i as f64)).collect())
}

struct Evaluator<'a> {
    theme_data: &'a ThemeData,
    options: &'a Options,
    error_count: usize,
}

impl<'a> Evaluator<'a> {
    fn new(theme_data: &'a ThemeData, options: &'a Options) -> Self {
        Self {
            theme_data,
            options,
            error_count: 0,
        }
    }

    fn log_error(&mut self) {
        self.error_count += 1;
    }

    fn render(&mut self, template_text: &str) -> RenderResult {
        self.error_count = 0;
        let tokens = tokenize(template_text);
        let mut pos = 0usize;
        let nodes = self.parse_nodes(&tokens, &mut pos, &[]);
        let mut scope = VariableScope::default();
        let text = self.evaluate_nodes(&nodes, &mut scope);
        RenderResult {
            text,
            error_count: self.error_count,
        }
    }

    fn render_file(&mut self, input_path: &Path, output_path: &Path) -> RenderFileResult {
        let Ok(content) = fs::read_to_string(input_path) else {
            return RenderFileResult::default();
        };
        let rendered = self.render(&content);
        if rendered.error_count > 0 {
            return RenderFileResult {
                success: false,
                wrote: false,
                error_count: rendered.error_count,
            };
        }
        if let Some(parent) = output_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let previous = fs::read_to_string(output_path).unwrap_or_default();
        if previous == rendered.text {
            return RenderFileResult {
                success: true,
                wrote: false,
                error_count: 0,
            };
        }
        let wrote_ok = fs::write(output_path, &rendered.text).is_ok();
        RenderFileResult {
            success: wrote_ok,
            wrote: wrote_ok,
            error_count: 0,
        }
    }

    fn parse_nodes(
        &mut self,
        tokens: &[Token],
        pos: &mut usize,
        stop_keywords: &[&str],
    ) -> Vec<Node> {
        let mut nodes = Vec::new();
        while *pos < tokens.len() {
            match &tokens[*pos] {
                Token::Text(t) => {
                    if !t.is_empty() {
                        nodes.push(Node::Text(t.clone()));
                    }
                    *pos += 1;
                }
                Token::Block(cmd) => {
                    if stop_keywords.iter().any(|kw| cmd.starts_with(kw)) {
                        return nodes;
                    }
                    if cmd.starts_with("for ") {
                        let node = self.parse_for(tokens, pos);
                        nodes.push(Node::For(node));
                    } else if cmd.starts_with("if ") {
                        let node = self.parse_if(tokens, pos);
                        nodes.push(Node::If(node));
                    } else {
                        *pos += 1;
                    }
                }
            }
        }
        nodes
    }

    fn current_block_text(tokens: &[Token], pos: usize) -> String {
        match tokens.get(pos) {
            Some(Token::Block(cmd)) => cmd.clone(),
            _ => String::new(),
        }
    }

    fn parse_for(&mut self, tokens: &[Token], pos: &mut usize) -> ForNode {
        let cmd = Self::current_block_text(tokens, *pos);
        *pos += 1;
        let Some(caps) = FOR_RE.captures(&cmd) else {
            self.log_error();
            return ForNode::default();
        };
        let variables: Vec<String> = caps[1].split(',').map(|s| c_trim(s).to_string()).collect();
        let iterable = c_trim(&caps[2]).to_string();
        let body = self.parse_nodes(tokens, pos, &["endfor"]);
        if *pos < tokens.len() {
            *pos += 1;
        }
        ForNode {
            variables,
            iterable,
            body,
        }
    }

    fn parse_if(&mut self, tokens: &[Token], pos: &mut usize) -> IfNode {
        let cmd = Self::current_block_text(tokens, *pos);
        *pos += 1;
        let mut negated = false;
        // Called only when `cmd.starts_with("if ")`, so byte index 3 is always a valid boundary.
        let mut condition_part = c_trim(cmd.get(3..).unwrap_or("")).to_string();
        if let Some(rest) = condition_part.strip_prefix("not ") {
            negated = true;
            condition_part = c_trim(rest).to_string();
        }
        let mut condition_expr = condition_part.clone();
        if let Some(caps) = IF_EXPR_RE.captures(&condition_part) {
            condition_expr = c_trim(&caps[1]).to_string();
        }
        let then_body = self.parse_nodes(tokens, pos, &["else", "endif"]);
        let mut else_body = Vec::new();
        if let Some(Token::Block(stop_cmd)) = tokens.get(*pos)
            && c_trim(stop_cmd) == "else"
        {
            *pos += 1;
            else_body = self.parse_nodes(tokens, pos, &["endif"]);
        }
        if *pos < tokens.len() {
            *pos += 1;
        }
        IfNode {
            condition_expr,
            negated,
            then_body,
            else_body,
        }
    }

    fn build_colors_map(&self) -> ScopeMap {
        let mut names: HashSet<&str> = HashSet::new();
        for mode_data in self.theme_data.values() {
            for (name, value) in mode_data {
                if !value.is_empty() {
                    names.insert(name.as_str());
                }
            }
        }
        let mut sorted: Vec<&str> = names.into_iter().collect();
        sorted.sort_unstable();

        let mut colors = ScopeMap::new();
        for name in sorted {
            let mut mode_map = ScopeMap::new();
            for (mode, mode_data) in self.theme_data {
                if let Some(v) = mode_data.get(name) {
                    mode_map.insert(mode.clone(), ScopeValue::Str(v.clone()));
                }
            }
            if let Some(def) = mode_map.get(&self.options.default_mode).cloned() {
                mode_map.insert("default".to_string(), def);
            }
            colors.insert(name.to_string(), ScopeValue::Map(mode_map));
        }
        colors
    }

    fn get_palette_entries(&self, palette_name: &str) -> ScopeArray {
        let mapped = match palette_name {
            "primary" => "primary",
            "secondary" => "secondary",
            "tertiary" => "tertiary",
            "error" => "error",
            "neutral" => "surface",
            "neutral_variant" => "surface_variant",
            _ => return ScopeArray::new(),
        };
        let Some(mode_data) = self.theme_data.get(&self.options.default_mode) else {
            return ScopeArray::new();
        };
        let Some(hex) = mode_data.get(mapped) else {
            return ScopeArray::new();
        };
        let Ok(color) = Color::from_hex(hex) else {
            return ScopeArray::new();
        };
        let argb = Argb {
            alpha: 255,
            red: color.r,
            green: color.g,
            blue: color.b,
        };
        let palette = TonalPalette::from_hct(Hct::new(argb));

        PALETTE_TONES
            .iter()
            .map(|&tone| {
                let toned = palette.tone(tone);
                let hex = Color::new(toned.red, toned.green, toned.blue).to_hex();
                let mut entry = ScopeMap::new();
                entry.insert("default".to_string(), ScopeValue::Str(hex.clone()));
                entry.insert("dark".to_string(), ScopeValue::Str(hex.clone()));
                entry.insert("light".to_string(), ScopeValue::Str(hex));
                ScopeValue::Map(entry)
            })
            .collect()
    }

    fn process_color_expression(&mut self, base: &str, filters: &[String]) -> String {
        let Some(caps) = COLOR_EXPR_RE.captures(base) else {
            self.log_error();
            return format!("{{{{{base}}}}}");
        };
        let color_name = color_alias(&caps[1]).to_string();
        let mode = caps[2].to_string();
        let format_type = caps[3].to_string();

        let mode_key = if mode == "default" {
            self.options.default_mode.clone()
        } else {
            mode.clone()
        };
        let Some(hex) = self
            .theme_data
            .get(&mode_key)
            .and_then(|mode_data| mode_data.get(&color_name))
        else {
            self.log_error();
            return format!("{{{{UNKNOWN:{color_name}.{mode}}}}}");
        };
        let mut color = RichColor {
            color: Color::from_hex(hex).unwrap_or(Color::BLACK),
            alpha: 1.0,
        };

        for filter_str in filters {
            let (name, arg) = parse_filter(filter_str);
            let arg_ref = arg.as_deref();
            match name.as_str() {
                "replace" => return apply_replace(&format_color(&color, &format_type), arg_ref),
                "lower_case" => return format_color(&color, &format_type).to_ascii_lowercase(),
                "camel_case" => return to_camel_case(&format_color(&color, &format_type)),
                "pascal_case" => return to_pascal_case(&format_color(&color, &format_type)),
                "snake_case" => return join_lower(&format_color(&color, &format_type), "_"),
                "kebab_case" => return join_lower(&format_color(&color, &format_type), "-"),
                "to_color" => continue,
                _ if COLOR_ARG_FILTERS.contains(&name.as_str()) => {
                    match apply_color_arg_filter(color.clone(), &name, arg_ref) {
                        Ok(c) => color = c,
                        Err(()) => {
                            self.log_error();
                            return format!("{{{{{base}}}}}");
                        }
                    }
                }
                _ if SUPPORTED_FILTERS.contains(&name.as_str()) => {
                    match apply_color_filter(color.clone(), &name, arg_ref) {
                        Ok(c) => color = c,
                        Err(()) => {
                            self.log_error();
                            return format!("{{{{{base}}}}}");
                        }
                    }
                }
                _ => {}
            }
        }
        format_color(&color, &format_type)
    }

    fn resolve_expression_value(&mut self, expr: &str, scope: &VariableScope) -> ScopeValue {
        let parts = split_pipes(expr);
        if parts.is_empty() {
            return ScopeValue::None;
        }
        let base = c_trim(&parts[0]).to_string();
        let filters: Vec<String> = parts[1..].iter().map(|f| c_trim(f).to_string()).collect();

        let mut resolved: ScopeValue;
        if base == "mode" {
            resolved = ScopeValue::Str(self.options.default_mode.clone());
        } else if base == "closest_color" {
            resolved = ScopeValue::Str(self.options.closest_color.clone());
        } else if base == "image" {
            resolved = ScopeValue::Str(self.options.image_path.clone());
        } else if base == "config_dir" {
            resolved = ScopeValue::Str(self.options.config_dir.clone());
        } else if base == "config_file" {
            resolved = ScopeValue::Str(self.options.config_file.clone());
        } else {
            let from_scope = resolve_from_scope(&base, scope);
            if !matches!(from_scope, ScopeValue::None) {
                resolved = from_scope;
            } else if base.starts_with("colors.") {
                return ScopeValue::Str(self.process_color_expression(&base, &filters));
            } else {
                return ScopeValue::Str(format!("{{{{{expr}}}}}"));
            }
        }

        for filter in &filters {
            let (name, arg) = parse_filter(filter);
            let arg_ref = arg.as_deref();
            let new_value = match name.as_str() {
                "replace" => Some(ScopeValue::Str(apply_replace(
                    &scope_value_to_string(&resolved),
                    arg_ref,
                ))),
                "lower_case" => Some(ScopeValue::Str(
                    scope_value_to_string(&resolved).to_ascii_lowercase(),
                )),
                "camel_case" => Some(ScopeValue::Str(to_camel_case(&scope_value_to_string(
                    &resolved,
                )))),
                "pascal_case" => Some(ScopeValue::Str(to_pascal_case(&scope_value_to_string(
                    &resolved,
                )))),
                "snake_case" => Some(ScopeValue::Str(join_lower(
                    &scope_value_to_string(&resolved),
                    "_",
                ))),
                "kebab_case" => Some(ScopeValue::Str(join_lower(
                    &scope_value_to_string(&resolved),
                    "-",
                ))),
                "to_color" => Some(ScopeValue::Str(scope_value_to_string(&resolved))),
                _ if COLOR_ARG_FILTERS.contains(&name.as_str()) => {
                    match as_rich_color(&resolved)
                        .and_then(|c| apply_color_arg_filter(c, &name, arg_ref))
                    {
                        Ok(c) => Some(ScopeValue::Color(c)),
                        Err(()) => {
                            self.log_error();
                            None
                        }
                    }
                }
                _ if SUPPORTED_FILTERS.contains(&name.as_str()) => {
                    match as_rich_color(&resolved)
                        .and_then(|c| apply_color_filter(c, &name, arg_ref))
                    {
                        Ok(c) => Some(ScopeValue::Color(c)),
                        Err(()) => {
                            self.log_error();
                            None
                        }
                    }
                }
                _ => None,
            };
            if let Some(v) = new_value {
                resolved = v;
            }
        }
        resolved
    }

    fn resolve_text(&mut self, text: &str, scope: &VariableScope) -> String {
        let mut output = String::new();
        let mut last = 0usize;
        for caps in EXPR_RE.captures_iter(text) {
            let Some(whole) = caps.get(0) else { continue };
            output.push_str(&text[last..whole.start()]);
            let expr = caps.get(1).map(|g| c_trim(g.as_str())).unwrap_or("");
            let value = self.resolve_expression_value(expr, scope);
            output.push_str(&scope_value_to_string(&value));
            last = whole.end();
        }
        output.push_str(&text[last..]);
        output
    }

    fn resolve_iterable(&self, expr: &str, scope: &VariableScope) -> ScopeArray {
        if let Some(range) = resolve_iterable_range(expr) {
            return range;
        }
        if expr == "colors" {
            let colors = self.build_colors_map();
            let mut keys: Vec<&String> = colors.keys().collect();
            keys.sort();
            return keys
                .into_iter()
                .map(|k| {
                    let mut pair = ScopeMap::new();
                    pair.insert("key".to_string(), ScopeValue::Str(k.clone()));
                    pair.insert(
                        "value".to_string(),
                        colors.get(k).cloned().unwrap_or(ScopeValue::None),
                    );
                    ScopeValue::Map(pair)
                })
                .collect();
        }
        if let Some(name) = expr.strip_prefix("palettes.") {
            return self.get_palette_entries(name);
        }
        if let Some(value) = scope.get(expr) {
            match value {
                ScopeValue::Array(arr) => return arr.clone(),
                ScopeValue::Map(map) => {
                    return map
                        .iter()
                        .map(|(k, v)| {
                            let mut pair = ScopeMap::new();
                            pair.insert("key".to_string(), ScopeValue::Str(k.clone()));
                            pair.insert("value".to_string(), v.clone());
                            ScopeValue::Map(pair)
                        })
                        .collect();
                }
                _ => {}
            }
        }
        ScopeArray::new()
    }

    fn evaluate_nodes(&mut self, nodes: &[Node], scope: &mut VariableScope) -> String {
        let mut out = String::new();
        for node in nodes {
            match node {
                Node::Text(text) => out.push_str(&self.resolve_text(text, scope)),
                Node::For(f) => out.push_str(&self.evaluate_for(f, scope)),
                Node::If(i) => out.push_str(&self.evaluate_if(i, scope)),
            }
        }
        out
    }

    fn evaluate_for(&mut self, node: &ForNode, scope: &mut VariableScope) -> String {
        let iterable = self.resolve_iterable(&node.iterable, scope);
        if iterable.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        let total = iterable.len();
        for (index, item) in iterable.iter().enumerate() {
            let mut loop_meta = ScopeMap::new();
            loop_meta.insert("index".to_string(), ScopeValue::Number(index as f64));
            loop_meta.insert("first".to_string(), ScopeValue::Bool(index == 0));
            loop_meta.insert("last".to_string(), ScopeValue::Bool(index == total - 1));
            let mut bindings = ScopeMap::new();
            bindings.insert("loop".to_string(), ScopeValue::Map(loop_meta));
            scope.push(bindings);

            let is_pair = matches!(item, ScopeValue::Map(m) if m.contains_key("key") && m.contains_key("value"));
            if is_pair {
                if let ScopeValue::Map(pair) = item {
                    if node.variables.len() >= 2 {
                        scope.set(node.variables[0].clone(), pair["key"].clone());
                        scope.set(node.variables[1].clone(), pair["value"].clone());
                    } else if let Some(v0) = node.variables.first() {
                        scope.set(v0.clone(), pair["key"].clone());
                    }
                }
            } else if let Some(v0) = node.variables.first() {
                scope.set(v0.clone(), item.clone());
            }

            out.push_str(&self.evaluate_nodes(&node.body, scope));
            scope.pop();
        }
        out
    }

    fn evaluate_if(&mut self, node: &IfNode, scope: &mut VariableScope) -> String {
        let value = self.resolve_expression_value(&node.condition_expr, scope);
        let mut truthy = is_truthy(&value);
        if node.negated {
            truthy = !truthy;
        }
        if truthy {
            self.evaluate_nodes(&node.then_body, scope)
        } else {
            self.evaluate_nodes(&node.else_body, scope)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn theme_data_from(dark: &[(&str, &str)], light: &[(&str, &str)]) -> ThemeData {
        let mut data = ThemeData::new();
        data.insert(
            "dark".to_string(),
            dark.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        data.insert(
            "light".to_string(),
            light
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        data
    }

    #[test]
    fn make_theme_data_has_no_derived_suffix_keys() {
        let mut palette = GeneratedPalette::default();
        palette.dark.insert("primary".to_string(), 0xffe6b450);

        let data = TemplateEngine::make_theme_data(&palette);
        let dark = &data["dark"];
        assert_eq!(dark.get("primary"), Some(&"#e6b450".to_string()));
        assert!(dark.get("primary_hex").is_none());
        assert!(dark.get("primary_rgb").is_none());
    }

    #[test]
    fn colors_expression_resolves_across_modes_and_default() {
        let data = theme_data_from(&[("primary", "#e6b450")], &[("primary", "#112233")]);
        let engine = TemplateEngine::with_options(
            data,
            Options {
                default_mode: "dark".to_string(),
                ..Options::default()
            },
        );
        let res = engine.render(
            "{{colors.primary.dark.hex}} {{colors.primary.light.hex}} {{colors.primary.default.hex}}",
        );
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "#e6b450 #112233 #e6b450");
    }

    #[test]
    fn unknown_color_reports_unknown_and_counts_error() {
        let data = theme_data_from(&[("primary", "#e6b450")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render("{{colors.missing.dark.hex}}");
        assert_eq!(res.error_count, 1);
        assert_eq!(res.text, "{{UNKNOWN:missing.dark}}");
    }

    #[test]
    fn unknown_bare_expression_passes_through_without_error() {
        let engine = TemplateEngine::new(ThemeData::new());
        let res = engine.render("{{ nonexistent }}");
        assert_eq!(res.error_count, 0);
        // resolve_text trims the captured expression before falling back to raw passthrough,
        // matching `StringUtils::trim((*it)[1].str())` in the C++'s `resolveText`.
        assert_eq!(res.text, "{{nonexistent}}");
    }

    #[test]
    fn hover_and_on_hover_alias_to_surface_container_high_and_on_surface() {
        let data = theme_data_from(
            &[
                ("surface_container_high", "#101010"),
                ("on_surface", "#f0f0f0"),
            ],
            &[],
        );
        let engine = TemplateEngine::new(data);
        let res = engine.render("{{colors.hover.dark.hex}} {{colors.on_hover.dark.hex}}");
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "#101010 #f0f0f0");
    }

    #[test]
    fn for_loop_over_int_range_binds_loop_metadata() {
        let engine = TemplateEngine::new(ThemeData::new());
        let res =
            engine.render("<* for i in 0..3 *>{{i}}:{{loop.first}}:{{loop.last}} <* endfor *>");
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "0:true:false 1:false:false 2:false:true ");
    }

    #[test]
    fn block_tag_alone_on_its_own_line_consumes_the_newline() {
        let engine = TemplateEngine::new(ThemeData::new());
        let res = engine.render("a\n<* for i in 0..2 *>\nX\n<* endfor *>\nb");
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "a\nX\nX\nb");
    }

    #[test]
    fn if_else_branches_on_truthiness_and_negation() {
        let engine = TemplateEngine::new(ThemeData::new());
        let truthy = engine.render("<* if {{mode}} *>Y<* else *>N<* endif *>");
        assert_eq!(truthy.text, "Y");
        let negated = engine.render("<* if not {{mode}} *>Y<* else *>N<* endif *>");
        assert_eq!(negated.text, "N");
    }

    #[test]
    fn nested_for_and_if() {
        let engine = TemplateEngine::new(ThemeData::new());
        let res = engine.render("<* for i in 0..3 *><* if {{i}} *>{{i}}<* endif *><* endfor *>");
        assert_eq!(res.error_count, 0);
        // i=0 is falsy ("0" string), so only 1 and 2 render.
        assert_eq!(res.text, "12");
    }

    #[test]
    fn for_loop_over_colors_iterates_sorted_names() {
        let data = theme_data_from(&[("beta", "#000000"), ("alpha", "#ffffff")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render("<* for k, v in colors *>{{k}} <* endfor *>");
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "alpha beta ");
    }

    #[test]
    fn palettes_iterable_yields_eighteen_tones() {
        let data = theme_data_from(&[("primary", "#4285f4")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render("<* for t in palettes.primary *>{{t.default}} <* endfor *>");
        assert_eq!(res.error_count, 0);
        let tones: Vec<&str> = res.text.trim().split(' ').collect();
        assert_eq!(tones.len(), 18);
        for tone in tones {
            assert!(tone.starts_with('#'), "expected a hex color, got {tone}");
        }
    }

    fn render_color_filter(hex: &str, filter: &str) -> (String, usize) {
        let data = theme_data_from(&[("c", hex)], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render(&format!("{{{{colors.c.dark.hex | {filter}}}}}"));
        (res.text, res.error_count)
    }

    #[test]
    fn grayscale_filter_averages_channels() {
        let (text, errors) = render_color_filter("#ff0000", "grayscale");
        assert_eq!(errors, 0);
        let expected = Color::new(76, 76, 76).to_hex(); // round(0.299*255)
        assert_eq!(text, expected);
    }

    #[test]
    fn invert_filter_flips_channels() {
        let (text, errors) = render_color_filter("#112233", "invert");
        assert_eq!(errors, 0);
        assert_eq!(text, Color::new(0xee, 0xdd, 0xcc).to_hex());
    }

    #[test]
    fn set_alpha_filter_changes_rgba_output() {
        let data = theme_data_from(&[("c", "#112233")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render("{{colors.c.dark.rgba | set_alpha:0.25}}");
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "rgba(17, 34, 51, 0.25)");
    }

    #[test]
    fn lighten_and_darken_filters_move_lightness() {
        let (lighter, e1) = render_color_filter("#404040", "lighten:20");
        let (darker, e2) = render_color_filter("#404040", "darken:20");
        assert_eq!((e1, e2), (0, 0));
        let base = Color::from_hex("#404040").unwrap();
        let (h, s, l) = base.to_hsl();
        assert_eq!(
            lighter,
            Color::from_hsl(h, s, (l + 0.2).clamp(0.0, 1.0)).to_hex()
        );
        assert_eq!(
            darker,
            Color::from_hsl(h, s, (l - 0.2).clamp(0.0, 1.0)).to_hex()
        );
    }

    #[test]
    fn set_hue_rotate_hue_saturate_desaturate_auto_lightness_apply() {
        for filter in [
            "set_hue:200",
            "rotate_hue:90",
            "set_saturation:50",
            "saturate:10",
            "desaturate:10",
            "auto_lightness:15",
            "set_lightness:60",
        ] {
            let (_text, errors) = render_color_filter("#3366cc", filter);
            assert_eq!(errors, 0, "filter {filter} should not error");
        }
    }

    #[test]
    fn set_red_green_blue_filters_clamp_channels() {
        let (text, errors) = render_color_filter("#000000", "set_red:999");
        assert_eq!(errors, 0);
        assert_eq!(text, Color::new(255, 0, 0).to_hex());
    }

    #[test]
    fn blend_filter_moves_hue_toward_target_by_amount() {
        let data = theme_data_from(&[("c", "#ff0000")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render(r##"{{colors.c.dark.hex | blend:"#0000ff", 1.0}}"##);
        assert_eq!(res.error_count, 0);
        // Fully blending red toward blue's hue (240) rotates hue all the way there.
        let (h, _, _) = Color::from_hex(&res.text).unwrap().to_hsl();
        assert!((h - 240.0).abs() < 1.0);
    }

    #[test]
    fn harmonize_filter_rotates_hue_toward_target_capped_at_15_degrees() {
        let data = theme_data_from(&[("c", "#ff0000")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render(r##"{{colors.c.dark.hex | harmonize:"#00ff00"}}"##);
        assert_eq!(res.error_count, 0);
        let (h, _, _) = Color::from_hex(&res.text).unwrap().to_hsl();
        assert!((h - 15.0).abs() < 1.0);
    }

    #[test]
    fn case_conversion_filters_apply_to_formatted_value() {
        let data = theme_data_from(&[("primary_color", "#aabbcc")], &[]);
        let engine = TemplateEngine::new(data);
        assert_eq!(
            engine
                .render("{{colors.primary_color.dark.hex | lower_case}}")
                .text,
            "#aabbcc"
        );
        assert_eq!(engine.render("{{mode | camel_case}}").text, "dark");
        assert_eq!(engine.render("{{mode | pascal_case}}").text, "Dark");
    }

    #[test]
    fn replace_filter_substitutes_literal_substrings() {
        let engine = TemplateEngine::new(ThemeData::new());
        let res = engine.render(r#"{{mode | replace:"dark", "night"}}"#);
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "night");
    }

    #[test]
    fn to_color_filter_is_a_passthrough() {
        let data = theme_data_from(&[("c", "#123456")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render("{{colors.c.dark.hex | to_color}}");
        assert_eq!(res.error_count, 0);
        assert_eq!(res.text, "#123456");
    }

    #[test]
    fn invalid_color_filter_argument_logs_error_and_leaves_value_unchanged() {
        let data = theme_data_from(&[("c", "#123456")], &[]);
        let engine = TemplateEngine::new(data);
        let res = engine.render("{{colors.c.dark.hex | set_alpha:not_a_number}}");
        assert_eq!(res.error_count, 1);
        assert_eq!(res.text, "{{colors.c.dark.hex}}");
    }

    #[test]
    fn real_gtk_template_fixture_renders_with_zero_errors() {
        let fixture = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/templates/gtk/gtk3.css"
        ))
        .expect("fixture template must exist");

        let mut palette = GeneratedPalette::default();
        for (k, v) in [
            ("primary", 0xff4285f4u32),
            ("on_primary", 0xffffffffu32),
            ("error", 0xffba1a1au32),
            ("on_error", 0xffffffffu32),
            ("surface", 0xff1a1c1eu32),
            ("on_surface", 0xffe2e2e6u32),
            ("surface_container", 0xff26282au32),
        ] {
            palette.dark.insert(k.to_string(), v);
            palette.light.insert(k.to_string(), v);
        }

        let theme_data = TemplateEngine::make_theme_data(&palette);
        let engine = TemplateEngine::new(theme_data);
        let res = engine.render(&fixture);
        assert_eq!(res.error_count, 0, "rendered output: {}", res.text);
        assert!(res.text.contains("@define-color accent_color #4285f4;"));
    }
}
