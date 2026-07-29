//! Contrast calculations & WCAG contrast adjustments.
//! Port of `src/theme/contrast.{cpp,h}`.

use crate::color::Color;

fn linearize_channel(c: u8) -> f64 {
    let n = c as f64 / 255.0;
    if n <= 0.03928 {
        n / 12.92
    } else {
        ((n + 0.055) / 1.055).powf(2.4)
    }
}

pub fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * linearize_channel(r) + 0.7152 * linearize_channel(g) + 0.0722 * linearize_channel(b)
}

pub fn contrast_ratio(a: &Color, b: &Color) -> f64 {
    let l1 = relative_luminance(a.r, a.g, a.b);
    let l2 = relative_luminance(b.r, b.g, b.b);
    let lighter = l1.max(l2);
    let darker = l1.min(l2);
    (lighter + 0.05) / (darker + 0.05)
}

pub fn is_dark(c: &Color) -> bool {
    relative_luminance(c.r, c.g, c.b) < 0.179
}

/// Binary-searches the foreground's HSL lightness toward black or white until WCAG contrast meets `min_ratio`.
/// `prefer_light`: -1 = darken, +1 = lighten, 0 = auto (lighten if background is dark).
pub fn ensure_contrast(
    foreground: &Color,
    background: &Color,
    min_ratio: f64,
    prefer_light: i32,
) -> Color {
    if contrast_ratio(foreground, background) >= min_ratio {
        return *foreground;
    }

    let (h, s, l) = foreground.to_hsl();
    let lighten = if prefer_light > 0 {
        true
    } else if prefer_light < 0 {
        false
    } else {
        is_dark(background)
    };

    let mut low = if lighten { l } else { 0.0 };
    let mut high = if lighten { 1.0 } else { l };

    let mut best = *foreground;
    for _ in 0..20 {
        let mid = (low + high) / 2.0;
        let test = Color::from_hsl(h, s, mid);
        if contrast_ratio(&test, background) >= min_ratio {
            best = test;
            if lighten {
                high = mid;
            } else {
                low = mid;
            }
        } else if lighten {
            low = mid;
        } else {
            high = mid;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_luminance_black_and_white() {
        assert_eq!(relative_luminance(0, 0, 0), 0.0);
        assert!((relative_luminance(255, 255, 255) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn contrast_ratio_black_white_is_21() {
        let ratio = contrast_ratio(&Color::BLACK, &Color::WHITE);
        assert!((ratio - 21.0).abs() < 1e-2);
    }

    #[test]
    fn is_dark_threshold() {
        assert!(is_dark(&Color::BLACK));
        assert!(!is_dark(&Color::WHITE));
    }

    #[test]
    fn ensure_contrast_adjusts_lightness() {
        let dark_blue = Color::new(0, 0, 100);
        let dark_bg = Color::new(10, 10, 10);

        let adjusted = ensure_contrast(&dark_blue, &dark_bg, 4.5, 0);
        assert!(contrast_ratio(&adjusted, &dark_bg) >= 4.5);
    }
}
