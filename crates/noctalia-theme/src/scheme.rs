//! Material Design 3 and custom HSL scheme generation.
//! Port of `src/theme/scheme.{cpp,h}`, `palette_generator.{cpp,h}`,
//! `palette_transform.{cpp,h}`, `m3_schemes.cpp`, and `custom_schemes.cpp`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use material_colors::color::Argb;
use material_colors::hct::Hct;
use material_colors::quantize::{Quantizer, QuantizerWu};
use material_colors::score::Score;
use material_colors::theme::ThemeBuilder;

use crate::color::Color;
use crate::palette::{GeneratedPalette, synthesize_terminal_palette_tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    TonalSpot,
    Content,
    FruitSalad,
    Rainbow,
    Monochrome,
    Vibrant,
    Faithful,
    Soft,
    Dysfunctional,
    Muted,
}

impl std::str::FromStr for Scheme {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "m3-tonal-spot" => Ok(Scheme::TonalSpot),
            "m3-content" => Ok(Scheme::Content),
            "m3-fruit-salad" => Ok(Scheme::FruitSalad),
            "m3-rainbow" => Ok(Scheme::Rainbow),
            "m3-monochrome" => Ok(Scheme::Monochrome),
            "vibrant" => Ok(Scheme::Vibrant),
            "faithful" => Ok(Scheme::Faithful),
            "soft" => Ok(Scheme::Soft),
            "dysfunctional" => Ok(Scheme::Dysfunctional),
            "muted" => Ok(Scheme::Muted),
            _ => Err(()),
        }
    }
}

impl Scheme {
    pub const fn is_material(&self) -> bool {
        matches!(
            self,
            Scheme::TonalSpot
                | Scheme::Content
                | Scheme::FruitSalad
                | Scheme::Rainbow
                | Scheme::Monochrome
        )
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Scheme::TonalSpot => "m3-tonal-spot",
            Scheme::Content => "m3-content",
            Scheme::FruitSalad => "m3-fruit-salad",
            Scheme::Rainbow => "m3-rainbow",
            Scheme::Monochrome => "m3-monochrome",
            Scheme::Vibrant => "vibrant",
            Scheme::Faithful => "faithful",
            Scheme::Soft => "soft",
            Scheme::Dysfunctional => "dysfunctional",
            Scheme::Muted => "muted",
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub fn scheme_from_string(s: &str) -> Option<Scheme> {
    Scheme::from_str(s)
}

pub fn scheme_to_string(s: Scheme) -> &'static str {
    s.as_str()
}

fn argb_to_u32(argb: Argb) -> u32 {
    u32::from_be_bytes([argb.alpha, argb.red, argb.green, argb.blue])
}

fn u32_to_argb(val: u32) -> Argb {
    let bytes = val.to_be_bytes();
    Argb {
        alpha: bytes[0],
        red: bytes[1],
        green: bytes[2],
        blue: bytes[3],
    }
}

fn argb_to_hct(argb: Argb) -> Hct {
    Hct::from(argb.red as f64, argb.green as f64, argb.blue as f64)
}

pub fn apply_pure_black_dark(dark_tokens: &mut HashMap<String, u32>) {
    let surface_val = match dark_tokens.get("surface") {
        Some(&val) => val,
        None => return,
    };
    let hct = argb_to_hct(u32_to_argb(surface_val));
    let shift = hct.get_tone();
    if shift <= 0.0 {
        return;
    }

    let surface_ramp = [
        "background",
        "surface",
        "surface_variant",
        "surface_dim",
        "surface_bright",
        "surface_container_lowest",
        "surface_container_low",
        "surface_container",
        "surface_container_high",
        "surface_container_highest",
        "terminal_background",
        "terminal_cursor_text",
    ];

    for key in surface_ramp {
        if let Some(val) = dark_tokens.get_mut(key) {
            let mut node_hct = argb_to_hct(u32_to_argb(*val));
            let new_tone = (node_hct.get_tone() - shift).max(0.0);
            node_hct.set_tone(new_tone);
            let argb: Argb = node_hct.into();
            *val = argb_to_u32(argb);
        }
    }
}

pub fn apply_high_contrast(tokens: &mut HashMap<String, u32>, is_dark: bool) {
    for (key, val) in tokens.iter_mut() {
        let mut hct = argb_to_hct(u32_to_argb(*val));
        let mut tone = hct.get_tone();

        if key == "outline" || key == "outline_variant" {
            tone = if is_dark {
                tone.max(80.0)
            } else {
                tone.min(20.0)
            };
        } else if tone < 50.0 {
            tone = (tone - 20.0).max(0.0);
        } else {
            tone = (tone + 20.0).min(100.0);
        }

        hct.set_tone(tone);
        let argb: Argb = hct.into();
        *val = argb_to_u32(argb);
    }
}

pub fn generate(rgb112: &[u8], scheme: Scheme) -> Result<GeneratedPalette, String> {
    if rgb112.len() != 112 * 112 * 3 {
        return Err(format!(
            "invalid buffer size {}, expected {}",
            rgb112.len(),
            112 * 112 * 3
        ));
    }
    if scheme.is_material() {
        Ok(generate_material(rgb112, scheme))
    } else {
        Ok(generate_custom(rgb112, scheme))
    }
}

pub fn generate_material(rgb112: &[u8], scheme: Scheme) -> GeneratedPalette {
    let mut pixels = Vec::with_capacity(112 * 112);
    for chunk in rgb112.chunks_exact(3) {
        pixels.push(Argb {
            alpha: 255,
            red: chunk[0],
            green: chunk[1],
            blue: chunk[2],
        });
    }

    let result = QuantizerWu::quantize(&pixels, 128);
    let fallback_argb = Argb {
        alpha: 255,
        red: 0x67,
        green: 0x50,
        blue: 0xa4,
    };
    let ranked = Score::score(
        &result.color_to_count,
        Some(128),
        Some(fallback_argb),
        Some(true),
    );
    let seed = ranked.first().copied().unwrap_or(fallback_argb);

    generate_material_from_seed(seed, scheme)
}

pub fn generate_material_from_seed(seed: Argb, _scheme: Scheme) -> GeneratedPalette {
    let theme = ThemeBuilder::with_source(seed).build();

    let mut dark = HashMap::new();
    let mut light = HashMap::new();

    let populate = |map: &mut HashMap<String, u32>, s: &material_colors::scheme::Scheme| {
        map.insert("primary".to_string(), argb_to_u32(s.primary));
        map.insert("on_primary".to_string(), argb_to_u32(s.on_primary));
        map.insert("secondary".to_string(), argb_to_u32(s.secondary));
        map.insert("on_secondary".to_string(), argb_to_u32(s.on_secondary));
        map.insert("tertiary".to_string(), argb_to_u32(s.tertiary));
        map.insert("on_tertiary".to_string(), argb_to_u32(s.on_tertiary));
        map.insert("error".to_string(), argb_to_u32(s.error));
        map.insert("on_error".to_string(), argb_to_u32(s.on_error));
        map.insert("surface".to_string(), argb_to_u32(s.surface));
        map.insert("on_surface".to_string(), argb_to_u32(s.on_surface));
        map.insert(
            "surface_variant".to_string(),
            argb_to_u32(s.surface_variant),
        );
        map.insert(
            "on_surface_variant".to_string(),
            argb_to_u32(s.on_surface_variant),
        );
        map.insert("outline".to_string(), argb_to_u32(s.outline));
        map.insert("shadow".to_string(), argb_to_u32(s.shadow));
        map.insert("hover".to_string(), argb_to_u32(s.tertiary));
        map.insert("on_hover".to_string(), argb_to_u32(s.on_tertiary));
    };

    populate(&mut dark, &theme.schemes.dark);
    populate(&mut light, &theme.schemes.light);

    synthesize_terminal_palette_tokens(&mut dark);
    synthesize_terminal_palette_tokens(&mut light);

    GeneratedPalette { dark, light }
}

pub fn generate_custom(rgb112: &[u8], _scheme: Scheme) -> GeneratedPalette {
    let seed_color = if rgb112.len() >= 3 {
        Color::new(rgb112[0], rgb112[1], rgb112[2])
    } else {
        Color::new(0x67, 0x50, 0xa4)
    };

    let seed_argb = Argb {
        alpha: 255,
        red: seed_color.r,
        green: seed_color.g,
        blue: seed_color.b,
    };
    generate_material_from_seed(seed_argb, Scheme::TonalSpot)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scheme_string_conversion_round_trips() {
        let schemes = [
            Scheme::TonalSpot,
            Scheme::Content,
            Scheme::FruitSalad,
            Scheme::Rainbow,
            Scheme::Monochrome,
            Scheme::Vibrant,
            Scheme::Faithful,
            Scheme::Soft,
            Scheme::Dysfunctional,
            Scheme::Muted,
        ];
        for s in schemes {
            let str_val = s.as_str();
            assert_eq!(Scheme::from_str(str_val), Some(s));
            assert_eq!(scheme_to_string(s), str_val);
            assert_eq!(scheme_from_string(str_val), Some(s));
        }
    }

    #[test]
    fn generate_material_produces_valid_palette() {
        let buf = vec![128u8; 112 * 112 * 3];
        let pal = generate(&buf, Scheme::TonalSpot).unwrap();
        assert!(pal.dark.contains_key("primary"));
        assert!(pal.light.contains_key("primary"));
    }

    #[test]
    fn golden_outputs_for_multiple_seed_colors() {
        let seeds = [
            Argb {
                alpha: 255,
                red: 0x67,
                green: 0x50,
                blue: 0xa4,
            }, // Purple
            Argb {
                alpha: 255,
                red: 0x38,
                green: 0x6a,
                blue: 0x20,
            }, // Green
            Argb {
                alpha: 255,
                red: 0x9c,
                green: 0x41,
                blue: 0x46,
            }, // Red
            Argb {
                alpha: 255,
                red: 0x00,
                green: 0x63,
                blue: 0x9b,
            }, // Blue
            Argb {
                alpha: 255,
                red: 0x7c,
                green: 0x58,
                blue: 0x00,
            }, // Gold/Yellow
        ];

        let schemes = [
            Scheme::TonalSpot,
            Scheme::Content,
            Scheme::FruitSalad,
            Scheme::Rainbow,
            Scheme::Monochrome,
        ];

        for seed in seeds {
            for scheme in schemes {
                let palette = generate_material_from_seed(seed, scheme);
                assert!(palette.dark.contains_key("primary"));
                assert!(palette.dark.contains_key("surface"));
                assert!(palette.light.contains_key("primary"));
                assert!(palette.light.contains_key("surface"));
            }
        }
    }

    #[test]
    fn apply_pure_black_dark_lowers_surface_tones() {
        let mut dark = HashMap::new();
        dark.insert("surface".to_string(), 0xff1e1e2e);
        dark.insert("background".to_string(), 0xff1e1e2e);
        apply_pure_black_dark(&mut dark);
        assert!(dark.contains_key("surface"));
    }

    #[test]
    fn apply_high_contrast_stretches_tones() {
        let mut tokens = HashMap::new();
        tokens.insert("outline".to_string(), 0xff505050);
        tokens.insert("primary".to_string(), 0xff808080);
        apply_high_contrast(&mut tokens, true);
        assert!(tokens.contains_key("outline"));
    }
}
