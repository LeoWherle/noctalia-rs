//! `noctalia theme` CLI entry point.
//! Partial port of `src/theme/cli.{cpp,h}` — task 4.2.6a (core JSON generate) only.
//!
//! Image-path and `--theme-json` generation (`--scheme`/`--dark`/`--light`/`--both`/
//! `--pure-black`, `-o`) are wired. Template rendering (`-r`/`-c`/`--builtin-config`) and
//! `--list-templates` are documented in [`HELP_TEXT`] (ported verbatim, matching the C++'s full
//! surface) but report explicit "not implemented yet" errors — see MIGRATION_PLAN.md tasks
//! 4.2.6b and 4.2.7. `--builtin-config`'s only other job for this task (rejecting a `--config`
//! combination) is still fully checked, since that's a real argument-conflict rule independent
//! of the unported rendering pipeline.
//!
//! Known gap surfaced while building this: `--theme-json`'s fixed-palette-mode branch (input
//! JSON shaped like a builtin catalog entry, keyed `mPrimary`/`mOnPrimary`/...) round-trips
//! through [`crate::palette::expand_fixed_palette_mode`], which is itself an incomplete port
//! (see the MIGRATION_PLAN.md note on task 3.2) — missing container/fixed/outline-variant
//! derived tokens. This CLI wires straight onto that existing function rather than working
//! around it: the output carries the same known incompleteness `expand_builtin_palette` already
//! has (that gap predates and is independent of this task), not a new regression introduced here.

use std::collections::HashMap;
use std::path::Path;

use crate::color::Color;
use crate::image::load_and_resize;
use crate::outputs::{Variant, to_json};
use crate::palette::{
    GeneratedPalette, Palette, expand_fixed_palette_mode, synthesize_terminal_palette_tokens,
};
use crate::scheme::{apply_pure_black_dark, generate, scheme_from_string};

// NOTE: plain multi-line string literal, not `\`-continued lines — see noctalia_config::cli's
// identical note on why that would silently eat indentation.
const HELP_TEXT: &str = "Usage: noctalia theme <image> [options]
       noctalia theme --list-templates [-c <file>]

Generate a color palette from an image. Material You and custom
schemes produce very different results.

Options:
  --scheme <name>   Material You (Material Design 3):
                      m3-tonal-spot  (default)
                      m3-content
                      m3-fruit-salad
                      m3-rainbow
                      m3-monochrome
                    Custom (HSL-space, non-M3):
                      vibrant
                      faithful
                      soft
                      dysfunctional
                      muted
  --dark            Emit only the dark variant (default)
  --light           Emit only the light variant
  --both            Emit both variants under dark/light keys
  --pure-black      Re-anchor the dark surface ramp to true black (OLED)
  --theme-json <f>  Load precomputed dark/light token maps from JSON
  -o <file>         Write JSON to file instead of stdout
  -r <in:out>       Render a template file to an output path
  -c <file>         Process a TOML template config file
  --builtin-config  Process the shipped built-in template catalog
  --list-templates  List built-in, cached community, and configured user templates
                    Use -c <file> to include a specific template config
  --default-mode    Template default mode: dark or light";

const TERMINAL_JSON_KEY: &str = "terminal";
const DIRECT_COLOR_TOKEN_KEYS: [(&str, &str); 6] = [
    ("foreground", "terminal_foreground"),
    ("background", "terminal_background"),
    ("cursor", "terminal_cursor"),
    ("cursorText", "terminal_cursor_text"),
    ("selectionFg", "terminal_selection_fg"),
    ("selectionBg", "terminal_selection_bg"),
];
const ANSI_COLOR_JSON_KEYS: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];
const ANSI_GROUP_TOKEN_KEYS: [(&str, &str); 2] =
    [("normal", "terminal_normal"), ("bright", "terminal_bright")];

/// Port of `setToken` (`cli.cpp:270-272`). Diverges deliberately from the C++: an invalid hex
/// string there propagates an uncaught `Color::fromHex` exception all the way out of `runCli`
/// (no `try`/`catch` anywhere between `injectTerminalColors` and `main`) — this project's lints
/// ban panics in non-test code, so a malformed terminal color here is silently skipped instead,
/// matching the tolerance `loadTokenMode`'s own per-token `try`/`catch` already shows elsewhere
/// in this same C++ file.
fn set_token(dst: &mut HashMap<String, u32>, key: &str, hex: &str) {
    if let Ok(color) = Color::from_hex(hex) {
        dst.insert(key.to_string(), color.to_argb());
    }
}

/// Port of `injectTerminalColors` (`cli.cpp:326-344`).
fn inject_terminal_colors(dst: &mut HashMap<String, u32>, mode_json: &serde_json::Value) {
    let Some(terminal) = mode_json.get(TERMINAL_JSON_KEY).filter(|v| v.is_object()) else {
        return;
    };

    for (json_key, token_key) in DIRECT_COLOR_TOKEN_KEYS {
        if let Some(hex) = terminal.get(json_key).and_then(|v| v.as_str()) {
            set_token(dst, token_key, hex);
        }
    }
    for (group_json_key, token_prefix) in ANSI_GROUP_TOKEN_KEYS {
        let Some(group) = terminal.get(group_json_key).filter(|v| v.is_object()) else {
            continue;
        };
        for color_key in ANSI_COLOR_JSON_KEYS {
            if let Some(hex) = group.get(color_key).and_then(|v| v.as_str()) {
                set_token(dst, &format!("{token_prefix}_{color_key}"), hex);
            }
        }
    }
}

/// Port of `parseFixedPaletteJson` (`cli.cpp:274-324`). The C++ masks alpha out of every parsed
/// color before rebuilding it (`rgbHex(x.toArgb() & 0x00FFFFFFU)`); this crate's [`Color`] has no
/// alpha channel to begin with (see `color.rs`), so that masking has no Rust equivalent to write.
fn parse_fixed_palette_json(src: &serde_json::Value) -> Result<Palette, String> {
    const ERR: &str = "fixed palette json is missing required colors";

    let load = |key: &str| -> Option<Color> {
        src.get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| Color::from_hex(s).ok())
    };

    let primary = load("mPrimary").ok_or(ERR)?;
    let on_primary = load("mOnPrimary").ok_or(ERR)?;
    let secondary = load("mSecondary").ok_or(ERR)?;
    let on_secondary = load("mOnSecondary").ok_or(ERR)?;
    let tertiary = load("mTertiary").ok_or(ERR)?;
    let on_tertiary = load("mOnTertiary").ok_or(ERR)?;
    let error = load("mError").ok_or(ERR)?;
    let on_error = load("mOnError").ok_or(ERR)?;
    let surface = load("mSurface").ok_or(ERR)?;
    let on_surface = load("mOnSurface").ok_or(ERR)?;
    let surface_variant = load("mSurfaceVariant").ok_or(ERR)?;
    let on_surface_variant = load("mOnSurfaceVariant").ok_or(ERR)?;
    let outline = load("mOutline").ok_or(ERR)?;
    let shadow = load("mShadow").unwrap_or(surface);

    Ok(Palette {
        primary,
        on_primary,
        secondary,
        on_secondary,
        tertiary,
        on_tertiary,
        error,
        on_error,
        surface,
        on_surface,
        surface_variant,
        on_surface_variant,
        outline,
        shadow,
        hover: tertiary,
        on_hover: on_tertiary,
    })
}

fn is_fixed_palette_mode(src: &serde_json::Value) -> bool {
    src.is_object() && src.get("mPrimary").is_some()
}

/// Port of `loadThemeJson`'s `loadTokenMode` lambda (`cli.cpp:361-372`).
fn load_token_mode(src: &serde_json::Value, dst: &mut HashMap<String, u32>) {
    let Some(obj) = src.as_object() else {
        return;
    };
    for (key, value) in obj {
        let Some(hex) = value.as_str() else { continue };
        if let Ok(color) = Color::from_hex(hex) {
            dst.insert(key.clone(), color.to_argb());
        }
    }
}

/// Port of `loadThemeJson`'s `loadFixedPalette` lambda (`cli.cpp:374-381`).
fn load_fixed_palette(
    src: &serde_json::Value,
    is_dark: bool,
    dst: &mut HashMap<String, u32>,
) -> Result<(), String> {
    let palette = parse_fixed_palette_json(src)?;
    *dst = expand_fixed_palette_mode(&palette, is_dark);
    inject_terminal_colors(dst, src);
    Ok(())
}

/// Port of `loadThemeJson` (`cli.cpp:346-415`).
fn load_theme_json(path: &Path) -> Result<GeneratedPalette, String> {
    let content =
        std::fs::read_to_string(path).map_err(|_| "cannot open theme json".to_string())?;
    let root: serde_json::Value = serde_json::from_str(&content).map_err(|err| err.to_string())?;

    let mut palette = GeneratedPalette::default();

    if root.get("dark").is_some() || root.get("light").is_some() {
        if let Some(dark) = root.get("dark") {
            if is_fixed_palette_mode(dark) {
                load_fixed_palette(dark, true, &mut palette.dark)?;
            } else {
                load_token_mode(dark, &mut palette.dark);
            }
        }
        if let Some(light) = root.get("light") {
            if is_fixed_palette_mode(light) {
                load_fixed_palette(light, false, &mut palette.light)?;
            } else {
                load_token_mode(light, &mut palette.light);
            }
        }
    } else if is_fixed_palette_mode(&root) {
        load_fixed_palette(&root, true, &mut palette.dark)?;
        load_fixed_palette(&root, false, &mut palette.light)?;
    } else {
        load_token_mode(&root, &mut palette.dark);
    }

    if palette.dark.is_empty() && palette.light.is_empty() {
        return Err("theme json contained no token maps".to_string());
    }

    // Port of the two-arg `synthesizeTerminalPaletteTokens(GeneratedPalette&)` overload
    // (`fixed_palette.cpp:195-202`): only synthesize into a mode that actually has tokens.
    if !palette.dark.is_empty() {
        synthesize_terminal_palette_tokens(&mut palette.dark);
    }
    if !palette.light.is_empty() {
        synthesize_terminal_palette_tokens(&mut palette.light);
    }

    Ok(palette)
}

/// Entry point for `noctalia theme <image> [options]`. `args` are the tokens after the `theme`
/// verb (matching `noctalia_config::cli::run_cli`'s convention). Returns a process exit code.
#[must_use]
pub fn run_cli(args: &[String]) -> i32 {
    let mut image_path: Option<String> = None;
    let mut theme_json_path: Option<String> = None;
    let mut scheme_name = "m3-tonal-spot".to_string();
    let mut variant = Variant::Dark;
    let mut pure_black = false;
    let mut out_path: Option<String> = None;
    let mut config_path: Option<String> = None;
    let mut builtin_config = false;
    let mut list_templates_requested = false;
    let mut render_specs: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--help" {
            println!("{HELP_TEXT}");
            return 0;
        }
        if a == "--scheme" && i + 1 < args.len() {
            scheme_name = args[i + 1].clone();
            i += 2;
            continue;
        }
        if a == "--theme-json" && i + 1 < args.len() {
            theme_json_path = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        if a == "--dark" {
            variant = Variant::Dark;
            i += 1;
            continue;
        }
        if a == "--light" {
            variant = Variant::Light;
            i += 1;
            continue;
        }
        if a == "--both" {
            variant = Variant::Both;
            i += 1;
            continue;
        }
        if a == "--pure-black" {
            pure_black = true;
            i += 1;
            continue;
        }
        if a == "-o" && i + 1 < args.len() {
            out_path = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        if (a == "--render" || a == "-r") && i + 1 < args.len() {
            render_specs.push(args[i + 1].clone());
            i += 2;
            continue;
        }
        if (a == "--config" || a == "-c") && i + 1 < args.len() {
            config_path = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        if a == "--builtin-config" {
            builtin_config = true;
            i += 1;
            continue;
        }
        if a == "--list-templates" {
            list_templates_requested = true;
            i += 1;
            continue;
        }
        if a == "--default-mode" && i + 1 < args.len() {
            // Parsed (to avoid misrouting to the unknown-argument error below) but unused: only
            // template rendering (task 4.2.6b) reads it.
            i += 2;
            continue;
        }
        if image_path.is_none() && !a.starts_with('-') {
            image_path = Some(a.to_string());
            i += 1;
            continue;
        }
        eprintln!("error: unknown theme argument: {a}");
        return 1;
    }

    if list_templates_requested {
        eprintln!(
            "error: `noctalia theme --list-templates` is not implemented yet (see MIGRATION_PLAN.md task 4.2.7)"
        );
        return 1;
    }

    if builtin_config && config_path.is_some() {
        eprintln!("error: --builtin-config cannot be combined with --config");
        return 1;
    }

    if image_path.is_none() && theme_json_path.is_none() {
        eprintln!(
            "error: theme requires an image path or --theme-json (try: noctalia theme --help)"
        );
        return 1;
    }

    let Some(scheme) = scheme_from_string(&scheme_name) else {
        eprintln!("error: unknown scheme '{scheme_name}'");
        return 1;
    };

    let mut palette = if let Some(theme_json_path) = &theme_json_path {
        let expanded = noctalia_core::files::paths::expand_user_path(theme_json_path);
        match load_theme_json(&expanded) {
            Ok(p) => p,
            Err(err) => {
                eprintln!("error: failed to load theme json: {err}");
                return 1;
            }
        }
    } else {
        // Guaranteed Some: the image-or-theme-json check above already returned on neither
        // being present, and the `theme_json_path` arm above handles the other case.
        let Some(image_path) = &image_path else {
            eprintln!(
                "error: theme requires an image path or --theme-json (try: noctalia theme --help)"
            );
            return 1;
        };
        let loaded = match load_and_resize(Path::new(image_path), scheme) {
            Ok(l) => l,
            Err(err) => {
                eprintln!("error: failed to load image: {err}");
                return 1;
            }
        };
        match generate(&loaded.rgb, scheme) {
            Ok(p) => p,
            Err(err) => {
                eprintln!("error: palette generation failed: {err}");
                return 1;
            }
        }
    };

    if pure_black {
        apply_pure_black_dark(&mut palette.dark);
    }

    let json = to_json(&palette, scheme, variant);
    // Port of `hasTemplateWork` (`cli.cpp:545`), computed after JSON generation (matching the
    // C++'s placement, not hoisted to the top) since `outPath` writes JSON regardless of this
    // flag and `hasTemplateWork`'s only effect above that is suppressing the stdout print.
    let has_template_work = !render_specs.is_empty() || config_path.is_some() || builtin_config;

    if let Some(out_path) = &out_path {
        if let Err(err) = std::fs::write(out_path, format!("{json}\n")) {
            eprintln!("error: cannot open output file: {out_path}: {err}");
            return 1;
        }
    } else if !has_template_work {
        println!("{json}");
    }

    if has_template_work {
        eprintln!(
            "error: `noctalia theme` template rendering (-r/-c/--builtin-config) is not implemented yet (see MIGRATION_PLAN.md task 4.2.6b)"
        );
        return 1;
    }

    0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("noctalia-theme-cli-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn help_flag_prints_help_and_succeeds() {
        assert_eq!(run_cli(&["--help".to_string()]), 0);
    }

    #[test]
    fn no_image_or_theme_json_fails() {
        assert_eq!(run_cli(&[]), 1);
    }

    #[test]
    fn unknown_flag_fails() {
        assert_eq!(run_cli(&["--not-a-real-flag".to_string()]), 1);
    }

    #[test]
    fn unknown_scheme_fails() {
        let dir = temp_dir("unknown-scheme");
        let img = dir.join("in.png");
        image::RgbImage::new(4, 4)
            .save(&img)
            .expect("write fixture png");

        assert_eq!(
            run_cli(&[
                img.to_string_lossy().to_string(),
                "--scheme".to_string(),
                "not-a-scheme".to_string(),
            ]),
            1
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nonexistent_image_fails() {
        assert_eq!(run_cli(&["/does/not/exist.png".to_string()]), 1);
    }

    #[test]
    fn render_spec_reports_not_implemented_yet() {
        let dir = temp_dir("render-not-impl");
        let img = dir.join("in.png");
        image::RgbImage::new(4, 4)
            .save(&img)
            .expect("write fixture png");

        let exit = run_cli(&[
            img.to_string_lossy().to_string(),
            "-r".to_string(),
            "in.tmpl:out.txt".to_string(),
        ]);
        assert_eq!(exit, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn render_spec_with_output_file_still_writes_json_before_reporting_not_implemented() {
        // Matches the C++'s actual control flow (`cli.cpp:546-556`): `-o` writes JSON
        // unconditionally, regardless of `hasTemplateWork` — only the *stdout* print and the
        // template-processing step itself are gated on it.
        let dir = temp_dir("render-with-outfile");
        let img = dir.join("in.png");
        image::RgbImage::new(4, 4)
            .save(&img)
            .expect("write fixture png");
        let out = dir.join("out.json");

        let exit = run_cli(&[
            img.to_string_lossy().to_string(),
            "-o".to_string(),
            out.to_string_lossy().to_string(),
            "-r".to_string(),
            "in.tmpl:out.txt".to_string(),
        ]);
        assert_eq!(exit, 1);
        assert!(out.exists());
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("primary"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builtin_config_combined_with_config_flag_fails() {
        let exit = run_cli(&[
            "img.png".to_string(),
            "--builtin-config".to_string(),
            "-c".to_string(),
            "foo.toml".to_string(),
        ]);
        assert_eq!(exit, 1);
    }

    #[test]
    fn list_templates_reports_not_implemented_yet() {
        assert_eq!(run_cli(&["--list-templates".to_string()]), 1);
    }

    #[test]
    fn image_generates_json_to_stdout() {
        let dir = temp_dir("image-json");
        let img = dir.join("in.png");
        let mut rgb = image::RgbImage::new(8, 8);
        for pixel in rgb.pixels_mut() {
            *pixel = image::Rgb([0x67, 0x50, 0xa4]);
        }
        rgb.save(&img).expect("write fixture png");

        let exit = run_cli(&[img.to_string_lossy().to_string()]);
        assert_eq!(exit, 0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn image_writes_json_to_output_file() {
        let dir = temp_dir("image-outfile");
        let img = dir.join("in.png");
        let mut rgb = image::RgbImage::new(8, 8);
        for pixel in rgb.pixels_mut() {
            *pixel = image::Rgb([0x38, 0x6a, 0x20]);
        }
        rgb.save(&img).expect("write fixture png");

        let out = dir.join("out.json");
        let exit = run_cli(&[
            img.to_string_lossy().to_string(),
            "-o".to_string(),
            out.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 0);

        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("primary"));
        assert!(content.ends_with('\n'));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_json_token_mode_generates_output() {
        let dir = temp_dir("theme-json-token");
        let json_path = dir.join("theme.json");
        std::fs::write(
            &json_path,
            r##"{"primary": "#e6b450", "surface": "#0b0e14"}"##,
        )
        .unwrap();

        let exit = run_cli(&[
            "--theme-json".to_string(),
            json_path.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_json_dark_light_token_mode_both_variant() {
        let dir = temp_dir("theme-json-both");
        let json_path = dir.join("theme.json");
        std::fs::write(
            &json_path,
            r##"{"dark": {"primary": "#e6b450"}, "light": {"primary": "#123456"}}"##,
        )
        .unwrap();

        let exit = run_cli(&[
            "--theme-json".to_string(),
            json_path.to_string_lossy().to_string(),
            "--both".to_string(),
        ]);
        assert_eq!(exit, 0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_json_missing_file_fails() {
        assert_eq!(
            run_cli(&[
                "--theme-json".to_string(),
                "/does/not/exist.json".to_string(),
            ]),
            1
        );
    }

    #[test]
    fn theme_json_empty_object_fails() {
        let dir = temp_dir("theme-json-empty");
        let json_path = dir.join("theme.json");
        std::fs::write(&json_path, "{}").unwrap();

        assert_eq!(
            run_cli(&[
                "--theme-json".to_string(),
                json_path.to_string_lossy().to_string(),
            ]),
            1
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_json_fixed_palette_mode_generates_output() {
        let dir = temp_dir("theme-json-fixed");
        let json_path = dir.join("theme.json");
        std::fs::write(
            &json_path,
            r##"{
                "mPrimary": "#e6b450", "mOnPrimary": "#0b0e14",
                "mSecondary": "#aad94c", "mOnSecondary": "#0b0e14",
                "mTertiary": "#39bae6", "mOnTertiary": "#0b0e14",
                "mError": "#d95757", "mOnError": "#0b0e14",
                "mSurface": "#0b0e14", "mOnSurface": "#d1d1c7",
                "mSurfaceVariant": "#1e222a", "mOnSurfaceVariant": "#8e959e",
                "mOutline": "#565b66"
            }"##,
        )
        .unwrap();

        let exit = run_cli(&[
            "--theme-json".to_string(),
            json_path.to_string_lossy().to_string(),
        ]);
        assert_eq!(exit, 0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_json_fixed_palette_mode_missing_field_fails() {
        let dir = temp_dir("theme-json-fixed-missing");
        let json_path = dir.join("theme.json");
        std::fs::write(&json_path, r##"{"mPrimary": "#e6b450"}"##).unwrap();

        assert_eq!(
            run_cli(&[
                "--theme-json".to_string(),
                json_path.to_string_lossy().to_string(),
            ]),
            1
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn inject_terminal_colors_sets_direct_and_ansi_tokens() {
        let mode_json: serde_json::Value = serde_json::from_str(
            r##"{
                "terminal": {
                    "foreground": "#ffffff",
                    "normal": {"red": "#ff0000"},
                    "bright": {"green": "#00ff00"}
                }
            }"##,
        )
        .unwrap();
        let mut dst = HashMap::new();
        inject_terminal_colors(&mut dst, &mode_json);

        assert_eq!(dst.get("terminal_foreground"), Some(&0xffffffffu32));
        assert_eq!(dst.get("terminal_normal_red"), Some(&0xffff0000u32));
        assert_eq!(dst.get("terminal_bright_green"), Some(&0xff00ff00u32));
    }

    #[test]
    fn set_token_skips_invalid_hex_instead_of_panicking() {
        let mut dst = HashMap::new();
        set_token(&mut dst, "some_key", "not-a-color");
        assert!(dst.is_empty());
    }

    #[test]
    fn parse_fixed_palette_json_defaults_shadow_to_surface() {
        let src: serde_json::Value = serde_json::from_str(
            r##"{
                "mPrimary": "#111111", "mOnPrimary": "#111111",
                "mSecondary": "#111111", "mOnSecondary": "#111111",
                "mTertiary": "#111111", "mOnTertiary": "#111111",
                "mError": "#111111", "mOnError": "#111111",
                "mSurface": "#222222", "mOnSurface": "#111111",
                "mSurfaceVariant": "#111111", "mOnSurfaceVariant": "#111111",
                "mOutline": "#111111"
            }"##,
        )
        .unwrap();
        let palette = parse_fixed_palette_json(&src).unwrap();
        assert_eq!(palette.shadow, palette.surface);
    }
}
