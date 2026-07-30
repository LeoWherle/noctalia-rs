//! Theme palettes & token expansion.
//! Port of `src/theme/palette.h`, `fixed_palette.{cpp,h}`, `builtin_palettes.{cpp,h}`,
//! `custom_palettes.{cpp,h}`, and `community_palettes.{cpp,h}`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub primary: Color,
    pub on_primary: Color,
    pub secondary: Color,
    pub on_secondary: Color,
    pub tertiary: Color,
    pub on_tertiary: Color,
    pub error: Color,
    pub on_error: Color,
    pub surface: Color,
    pub on_surface: Color,
    pub surface_variant: Color,
    pub on_surface_variant: Color,
    pub outline: Color,
    pub shadow: Color,
    pub hover: Color,
    pub on_hover: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalAnsiColors {
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPalette {
    pub normal: TerminalAnsiColors,
    pub bright: TerminalAnsiColors,
    pub foreground: Color,
    pub background: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub cursor_text: Color,
    pub cursor: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedPaletteMode {
    pub palette: Palette,
    pub terminal: TerminalPalette,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinPalette {
    pub name: &'static str,
    pub dark: FixedPaletteMode,
    pub light: FixedPaletteMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedPalette {
    pub dark: HashMap<String, u32>,
    pub light: HashMap<String, u32>,
}

pub fn expand_fixed_palette_mode(palette: &Palette, _is_dark: bool) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    map.insert("primary".to_string(), palette.primary.to_argb());
    map.insert("on_primary".to_string(), palette.on_primary.to_argb());
    map.insert("secondary".to_string(), palette.secondary.to_argb());
    map.insert("on_secondary".to_string(), palette.on_secondary.to_argb());
    map.insert("tertiary".to_string(), palette.tertiary.to_argb());
    map.insert("on_tertiary".to_string(), palette.on_tertiary.to_argb());
    map.insert("error".to_string(), palette.error.to_argb());
    map.insert("on_error".to_string(), palette.on_error.to_argb());
    map.insert("surface".to_string(), palette.surface.to_argb());
    map.insert("on_surface".to_string(), palette.on_surface.to_argb());
    map.insert(
        "surface_variant".to_string(),
        palette.surface_variant.to_argb(),
    );
    map.insert(
        "on_surface_variant".to_string(),
        palette.on_surface_variant.to_argb(),
    );
    map.insert("outline".to_string(), palette.outline.to_argb());
    map.insert("shadow".to_string(), palette.shadow.to_argb());
    map.insert("hover".to_string(), palette.hover.to_argb());
    map.insert("on_hover".to_string(), palette.on_hover.to_argb());
    map
}

pub fn apply_terminal_palette(tokens: &mut HashMap<String, u32>, terminal: &TerminalPalette) {
    tokens.insert(
        "terminal_foreground".to_string(),
        terminal.foreground.to_argb(),
    );
    tokens.insert(
        "terminal_background".to_string(),
        terminal.background.to_argb(),
    );
    tokens.insert("terminal_cursor".to_string(), terminal.cursor.to_argb());
    tokens.insert(
        "terminal_cursor_text".to_string(),
        terminal.cursor_text.to_argb(),
    );
    tokens.insert(
        "terminal_selection_fg".to_string(),
        terminal.selection_fg.to_argb(),
    );
    tokens.insert(
        "terminal_selection_bg".to_string(),
        terminal.selection_bg.to_argb(),
    );

    tokens.insert(
        "terminal_normal_black".to_string(),
        terminal.normal.black.to_argb(),
    );
    tokens.insert(
        "terminal_normal_red".to_string(),
        terminal.normal.red.to_argb(),
    );
    tokens.insert(
        "terminal_normal_green".to_string(),
        terminal.normal.green.to_argb(),
    );
    tokens.insert(
        "terminal_normal_yellow".to_string(),
        terminal.normal.yellow.to_argb(),
    );
    tokens.insert(
        "terminal_normal_blue".to_string(),
        terminal.normal.blue.to_argb(),
    );
    tokens.insert(
        "terminal_normal_magenta".to_string(),
        terminal.normal.magenta.to_argb(),
    );
    tokens.insert(
        "terminal_normal_cyan".to_string(),
        terminal.normal.cyan.to_argb(),
    );
    tokens.insert(
        "terminal_normal_white".to_string(),
        terminal.normal.white.to_argb(),
    );

    tokens.insert(
        "terminal_bright_black".to_string(),
        terminal.bright.black.to_argb(),
    );
    tokens.insert(
        "terminal_bright_red".to_string(),
        terminal.bright.red.to_argb(),
    );
    tokens.insert(
        "terminal_bright_green".to_string(),
        terminal.bright.green.to_argb(),
    );
    tokens.insert(
        "terminal_bright_yellow".to_string(),
        terminal.bright.yellow.to_argb(),
    );
    tokens.insert(
        "terminal_bright_blue".to_string(),
        terminal.bright.blue.to_argb(),
    );
    tokens.insert(
        "terminal_bright_magenta".to_string(),
        terminal.bright.magenta.to_argb(),
    );
    tokens.insert(
        "terminal_bright_cyan".to_string(),
        terminal.bright.cyan.to_argb(),
    );
    tokens.insert(
        "terminal_bright_white".to_string(),
        terminal.bright.white.to_argb(),
    );
}

/// Port of `synthesizeTerminalPaletteTokens(TokenMap&)` (`fixed_palette.cpp:155-193`).
///
/// Fixed prior to task 4.2.6: this previously used a different (undocumented, incorrect)
/// fallback chain — e.g. `terminal_normal_green` fell back to `tertiary` instead of the C++'s
/// `primary`, and the top-level fallbacks didn't chain through `background`/`surface` the way
/// `tokenOr(tokens, "background", tokenOr(tokens, "surface", kOpaqueBlack))` does. Found while
/// building on this function for the `--theme-json` CLI path; no golden test pinned the old
/// (wrong) values, so this is a correctness fix, not a recorded deliberate divergence.
pub fn synthesize_terminal_palette_tokens(tokens: &mut HashMap<String, u32>) {
    const OPAQUE_BLACK: u32 = 0xff000000;
    const OPAQUE_WHITE: u32 = 0xffffffff;

    let get = |t: &HashMap<String, u32>, key: &str, fallback: u32| -> u32 {
        t.get(key).copied().unwrap_or(fallback)
    };

    let background = get(tokens, "background", get(tokens, "surface", OPAQUE_BLACK));
    let foreground = get(tokens, "on_surface", OPAQUE_WHITE);
    let surface = get(tokens, "surface", background);
    let surface_variant = get(tokens, "surface_variant", surface);
    let on_surface_variant = get(tokens, "on_surface_variant", foreground);
    let outline = get(tokens, "outline", on_surface_variant);
    let error = get(tokens, "error", foreground);
    let primary = get(tokens, "primary", foreground);
    let secondary = get(tokens, "secondary", primary);
    let tertiary = get(tokens, "tertiary", secondary);
    let primary_fixed_dim = get(tokens, "primary_fixed_dim", primary);
    let secondary_fixed_dim = get(tokens, "secondary_fixed_dim", secondary);

    let entries: [(&str, u32); 22] = [
        ("terminal_foreground", foreground),
        ("terminal_background", background),
        ("terminal_cursor", foreground),
        ("terminal_cursor_text", background),
        ("terminal_selection_fg", on_surface_variant),
        ("terminal_selection_bg", surface_variant),
        ("terminal_normal_black", surface_variant),
        ("terminal_normal_red", error),
        ("terminal_normal_green", primary),
        ("terminal_normal_yellow", secondary),
        ("terminal_normal_blue", tertiary),
        ("terminal_normal_magenta", primary_fixed_dim),
        ("terminal_normal_cyan", secondary_fixed_dim),
        ("terminal_normal_white", foreground),
        ("terminal_bright_black", outline),
        ("terminal_bright_red", error),
        ("terminal_bright_green", primary),
        ("terminal_bright_yellow", secondary),
        ("terminal_bright_blue", tertiary),
        ("terminal_bright_magenta", primary_fixed_dim),
        ("terminal_bright_cyan", secondary_fixed_dim),
        ("terminal_bright_white", foreground),
    ];
    for (key, value) in entries {
        tokens.entry(key.to_string()).or_insert(value);
    }
}

pub fn expand_fixed_palettes(dark: &Palette, light: &Palette) -> GeneratedPalette {
    GeneratedPalette {
        dark: expand_fixed_palette_mode(dark, true),
        light: expand_fixed_palette_mode(light, false),
    }
}

pub fn expand_builtin_palette(palette: &BuiltinPalette) -> GeneratedPalette {
    let mut generated = expand_fixed_palettes(&palette.dark.palette, &palette.light.palette);
    apply_terminal_palette(&mut generated.dark, &palette.dark.terminal);
    apply_terminal_palette(&mut generated.light, &palette.light.terminal);
    generated
}

const fn hex(s: &str) -> Color {
    let bytes = s.as_bytes();
    let offset = if bytes[0] == b'#' { 1 } else { 0 };
    let r = (hex_digit(bytes[offset]) << 4) | hex_digit(bytes[offset + 1]);
    let g = (hex_digit(bytes[offset + 2]) << 4) | hex_digit(bytes[offset + 3]);
    let b = (hex_digit(bytes[offset + 4]) << 4) | hex_digit(bytes[offset + 5]);
    Color::new(r, g, b)
}

const fn hex_digit(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

pub static BUILTIN_PALETTES: &[BuiltinPalette] = &[
    BuiltinPalette {
        name: "Ayu",
        dark: FixedPaletteMode {
            palette: Palette {
                primary: hex("#E6B450"),
                on_primary: hex("#0B0E14"),
                secondary: hex("#AAD94C"),
                on_secondary: hex("#0B0E14"),
                tertiary: hex("#39BAE6"),
                on_tertiary: hex("#0B0E14"),
                error: hex("#D95757"),
                on_error: hex("#0B0E14"),
                surface: hex("#0B0E14"),
                on_surface: hex("#D1D1C7"),
                surface_variant: hex("#1e222a"),
                on_surface_variant: hex("#8E959E"),
                outline: hex("#565B66"),
                shadow: hex("#000000"),
                hover: hex("#39BAE6"),
                on_hover: hex("#0B0E14"),
            },
            terminal: TerminalPalette {
                normal: TerminalAnsiColors {
                    black: hex("#171b24"),
                    red: hex("#ed8274"),
                    green: hex("#87d96c"),
                    yellow: hex("#facc6e"),
                    blue: hex("#6dcbfa"),
                    magenta: hex("#dabafa"),
                    cyan: hex("#90e1c6"),
                    white: hex("#c7c7c7"),
                },
                bright: TerminalAnsiColors {
                    black: hex("#686868"),
                    red: hex("#f28779"),
                    green: hex("#d5ff80"),
                    yellow: hex("#ffd173"),
                    blue: hex("#73d0ff"),
                    magenta: hex("#dfbfff"),
                    cyan: hex("#95e6cb"),
                    white: hex("#ffffff"),
                },
                foreground: hex("#D1D1C7"),
                background: hex("#1f2430"),
                selection_fg: hex("#1f2430"),
                selection_bg: hex("#409fff"),
                cursor_text: hex("#1f2430"),
                cursor: hex("#ffcc66"),
            },
        },
        light: FixedPaletteMode {
            palette: Palette {
                primary: hex("#FF8F40"),
                on_primary: hex("#F8F9FA"),
                secondary: hex("#86B300"),
                on_secondary: hex("#F8F9FA"),
                tertiary: hex("#55B4D4"),
                on_tertiary: hex("#F8F9FA"),
                error: hex("#E65050"),
                on_error: hex("#F8F9FA"),
                surface: hex("#F8F9FA"),
                on_surface: hex("#42474C"),
                surface_variant: hex("#E4E6E9"),
                on_surface_variant: hex("#6E757C"),
                outline: hex("#8A9199"),
                shadow: hex("#F8F9FA"),
                hover: hex("#55B4D4"),
                on_hover: hex("#F8F9FA"),
            },
            terminal: TerminalPalette {
                normal: TerminalAnsiColors {
                    black: hex("#000000"),
                    red: hex("#ea6c6d"),
                    green: hex("#6cbf43"),
                    yellow: hex("#eca944"),
                    blue: hex("#3199e1"),
                    magenta: hex("#9e75c7"),
                    cyan: hex("#46ba94"),
                    white: hex("#bababa"),
                },
                bright: TerminalAnsiColors {
                    black: hex("#686868"),
                    red: hex("#f07171"),
                    green: hex("#86b300"),
                    yellow: hex("#f2ae49"),
                    blue: hex("#399ee6"),
                    magenta: hex("#a37acc"),
                    cyan: hex("#4cbf99"),
                    white: hex("#d1d1d1"),
                },
                foreground: hex("#42474C"),
                background: hex("#f8f9fa"),
                selection_fg: hex("#f8f9fa"),
                selection_bg: hex("#035bd6"),
                cursor_text: hex("#f8f9fa"),
                cursor: hex("#ffaa33"),
            },
        },
    },
    BuiltinPalette {
        name: "Catppuccin",
        dark: FixedPaletteMode {
            palette: Palette {
                primary: hex("#cba6f7"),
                on_primary: hex("#11111b"),
                secondary: hex("#fab387"),
                on_secondary: hex("#11111b"),
                tertiary: hex("#94e2d5"),
                on_tertiary: hex("#11111b"),
                error: hex("#f38ba8"),
                on_error: hex("#11111b"),
                surface: hex("#1e1e2e"),
                on_surface: hex("#cdd6f4"),
                surface_variant: hex("#313244"),
                on_surface_variant: hex("#a3b4eb"),
                outline: hex("#4c4f69"),
                shadow: hex("#11111b"),
                hover: hex("#94e2d5"),
                on_hover: hex("#11111b"),
            },
            terminal: TerminalPalette {
                normal: TerminalAnsiColors {
                    black: hex("#45475a"),
                    red: hex("#f38ba8"),
                    green: hex("#a6e3a1"),
                    yellow: hex("#f9e2af"),
                    blue: hex("#89b4fa"),
                    magenta: hex("#f5c2e7"),
                    cyan: hex("#94e2d5"),
                    white: hex("#a6adc8"),
                },
                bright: TerminalAnsiColors {
                    black: hex("#585b70"),
                    red: hex("#f37799"),
                    green: hex("#89d88b"),
                    yellow: hex("#ebd391"),
                    blue: hex("#74a8fc"),
                    magenta: hex("#f2aede"),
                    cyan: hex("#6bd7ca"),
                    white: hex("#bac2de"),
                },
                foreground: hex("#cdd6f4"),
                background: hex("#1e1e2e"),
                selection_fg: hex("#cdd6f4"),
                selection_bg: hex("#585b70"),
                cursor_text: hex("#1e1e2e"),
                cursor: hex("#f5e0dc"),
            },
        },
        light: FixedPaletteMode {
            palette: Palette {
                primary: hex("#8839ef"),
                on_primary: hex("#eff1f5"),
                secondary: hex("#fe640b"),
                on_secondary: hex("#eff1f5"),
                tertiary: hex("#40a02b"),
                on_tertiary: hex("#eff1f5"),
                error: hex("#d20f39"),
                on_error: hex("#dce0e8"),
                surface: hex("#eff1f5"),
                on_surface: hex("#4c4f69"),
                surface_variant: hex("#ccd0da"),
                on_surface_variant: hex("#6c6f85"),
                outline: hex("#a5adcb"),
                shadow: hex("#dce0e8"),
                hover: hex("#40a02b"),
                on_hover: hex("#eff1f5"),
            },
            terminal: TerminalPalette {
                normal: TerminalAnsiColors {
                    black: hex("#bcc0cc"),
                    red: hex("#d20f39"),
                    green: hex("#40a02b"),
                    yellow: hex("#df8e1d"),
                    blue: hex("#1e66f5"),
                    magenta: hex("#ea76cb"),
                    cyan: hex("#179299"),
                    white: hex("#5c5f77"),
                },
                bright: TerminalAnsiColors {
                    black: hex("#acb0be"),
                    red: hex("#d20f39"),
                    green: hex("#40a02b"),
                    yellow: hex("#df8e1d"),
                    blue: hex("#1e66f5"),
                    magenta: hex("#ea76cb"),
                    cyan: hex("#179299"),
                    white: hex("#6c6f85"),
                },
                foreground: hex("#4c4f69"),
                background: hex("#eff1f5"),
                selection_fg: hex("#eff1f5"),
                selection_bg: hex("#dc8a78"),
                cursor_text: hex("#eff1f5"),
                cursor: hex("#dc8a78"),
            },
        },
    },
    BuiltinPalette {
        name: "Dracula",
        dark: FixedPaletteMode {
            palette: Palette {
                primary: hex("#bd93f9"),
                on_primary: hex("#282A36"),
                secondary: hex("#ff79c6"),
                on_secondary: hex("#4e1d32"),
                tertiary: hex("#8be9fd"),
                on_tertiary: hex("#003543"),
                error: hex("#FF5555"),
                on_error: hex("#282A36"),
                surface: hex("#282A36"),
                on_surface: hex("#F8F8F2"),
                surface_variant: hex("#44475A"),
                on_surface_variant: hex("#d6d8e0"),
                outline: hex("#5a5e77"),
                shadow: hex("#282A36"),
                hover: hex("#8be9fd"),
                on_hover: hex("#003543"),
            },
            terminal: TerminalPalette {
                normal: TerminalAnsiColors {
                    black: hex("#21222c"),
                    red: hex("#ff5555"),
                    green: hex("#50fa7b"),
                    yellow: hex("#f1fa8c"),
                    blue: hex("#bd93f9"),
                    magenta: hex("#ff79c6"),
                    cyan: hex("#8be9fd"),
                    white: hex("#f8f8f2"),
                },
                bright: TerminalAnsiColors {
                    black: hex("#6272a4"),
                    red: hex("#ff6e6e"),
                    green: hex("#69ff94"),
                    yellow: hex("#ffffa5"),
                    blue: hex("#d6acff"),
                    magenta: hex("#ff92df"),
                    cyan: hex("#a4ffff"),
                    white: hex("#ffffff"),
                },
                foreground: hex("#f8f8f2"),
                background: hex("#282a36"),
                selection_fg: hex("#ffffff"),
                selection_bg: hex("#44475a"),
                cursor_text: hex("#282a36"),
                cursor: hex("#f8f8f2"),
            },
        },
        light: FixedPaletteMode {
            palette: Palette {
                primary: hex("#8332f4"),
                on_primary: hex("#ffffff"),
                secondary: hex("#ff1399"),
                on_secondary: hex("#ffffff"),
                tertiary: hex("#0398b9"),
                on_tertiary: hex("#ffffff"),
                error: hex("#FF5555"),
                on_error: hex("#282A36"),
                surface: hex("#f8f8f2"),
                on_surface: hex("#282a36"),
                surface_variant: hex("#e6e6ea"),
                on_surface_variant: hex("#44475a"),
                outline: hex("#cacad3"),
                shadow: hex("#d6d8e0"),
                hover: hex("#0398b9"),
                on_hover: hex("#ffffff"),
            },
            terminal: TerminalPalette {
                normal: TerminalAnsiColors {
                    black: hex("#f8f8f2"),
                    red: hex("#ff5555"),
                    green: hex("#50fa7b"),
                    yellow: hex("#f1fa8c"),
                    blue: hex("#bd93f9"),
                    magenta: hex("#ff79c6"),
                    cyan: hex("#8be9fd"),
                    white: hex("#282a36"),
                },
                bright: TerminalAnsiColors {
                    black: hex("#6272a4"),
                    red: hex("#ff6e6e"),
                    green: hex("#69ff94"),
                    yellow: hex("#ffffa5"),
                    blue: hex("#d6acff"),
                    magenta: hex("#ff92df"),
                    cyan: hex("#a4ffff"),
                    white: hex("#000000"),
                },
                foreground: hex("#282a36"),
                background: hex("#ffffff"),
                selection_fg: hex("#ffffff"),
                selection_bg: hex("#6272a4"),
                cursor_text: hex("#ffffff"),
                cursor: hex("#282a36"),
            },
        },
    },
];

pub fn builtin_palettes() -> &'static [BuiltinPalette] {
    BUILTIN_PALETTES
}

pub fn find_builtin_palette(name: &str) -> Option<&'static BuiltinPalette> {
    BUILTIN_PALETTES.iter().find(|p| p.name == name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn find_builtin_palettes() {
        assert!(find_builtin_palette("Ayu").is_some());
        assert!(find_builtin_palette("Catppuccin").is_some());
        assert!(find_builtin_palette("Dracula").is_some());
        assert!(find_builtin_palette("NonExistent").is_none());
    }

    #[test]
    fn expand_builtin_palette_contains_tokens() {
        let ayu = find_builtin_palette("Ayu").unwrap();
        let generated = expand_builtin_palette(ayu);
        assert!(generated.dark.contains_key("primary"));
        assert!(generated.dark.contains_key("terminal_foreground"));
        assert!(generated.light.contains_key("primary"));
        assert!(generated.light.contains_key("terminal_foreground"));
    }

    #[test]
    fn synthesize_terminal_palette_tokens_follows_cpp_fallback_chain() {
        // Matches `fixed_palette.cpp:155-193`'s exact fallback chain, not the token's own name.
        let mut tokens = HashMap::new();
        tokens.insert("surface".to_string(), 0x11);
        tokens.insert("on_surface".to_string(), 0x22);
        tokens.insert("primary".to_string(), 0x33);
        // secondary/tertiary/outline/error/*_fixed_dim/background left absent to exercise the
        // chained fallbacks.
        synthesize_terminal_palette_tokens(&mut tokens);

        assert_eq!(tokens["terminal_background"], 0x11); // background -> surface
        assert_eq!(tokens["terminal_foreground"], 0x22); // foreground -> on_surface
        assert_eq!(tokens["terminal_cursor"], 0x22); // foreground
        assert_eq!(tokens["terminal_cursor_text"], 0x11); // background
        assert_eq!(tokens["terminal_normal_green"], 0x33); // primary, NOT tertiary
        assert_eq!(tokens["terminal_normal_blue"], 0x33); // tertiary -> secondary -> primary
        assert_eq!(tokens["terminal_normal_magenta"], 0x33); // primary_fixed_dim -> primary
        assert_eq!(tokens["terminal_bright_black"], 0x22); // outline -> on_surface_variant -> foreground
    }

    #[test]
    fn synthesize_terminal_palette_tokens_never_overwrites_present_tokens() {
        let mut tokens = HashMap::new();
        tokens.insert("terminal_normal_green".to_string(), 0xdeadbeef);
        synthesize_terminal_palette_tokens(&mut tokens);
        assert_eq!(tokens["terminal_normal_green"], 0xdeadbeef);
    }
}
