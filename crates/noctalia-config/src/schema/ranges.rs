//! Numeric range constraints for schema fields.
//! Port of `src/config/schema/ranges.h`.

/// Optional numeric constraint carried by a Field. Single source for parse-time
/// clamping, GUI slider bounds, and range validation.
///
/// Port of `Range<T>` (ranges.h / field.h:23-27).
#[derive(Debug, Clone, Copy)]
pub struct Range<T: PartialOrd + Copy> {
    pub min: Option<T>,
    pub max: Option<T>,
    /// GUI metadata only — the parser ignores this.
    pub step: Option<T>,
}

impl<T: PartialOrd + Copy> Range<T> {
    pub const fn new(min: Option<T>, max: Option<T>, step: Option<T>) -> Self {
        Self { min, max, step }
    }
}

/// Port of `applyRange` (field.h:29-37).
pub fn apply_range<T: PartialOrd + Copy>(value: T, range: &Range<T>) -> T {
    let mut v = value;
    if let Some(min) = range.min
        && v < min
    {
        v = min;
    }
    if let Some(max) = range.max
        && v > max
    {
        v = max;
    }
    v
}

// ── Shared ranges (ranges.h:13-46) ─────────────────────────────────────────

/// Opacities, intensities, 0..1 factors.
pub const UNIT_RANGE: Range<f64> = Range::new(Some(0.0), Some(1.0), Some(0.01));

/// ui_scale, notification/osd scale.
pub const SCALE_RANGE: Range<f64> = Range::new(Some(0.5), Some(2.5), Some(0.05));

/// Calendar/weather refresh interval.
pub const REFRESH_MINUTES_RANGE: Range<i64> = Range::new(Some(5), Some(240), Some(5));

// Shell.
pub const ANIMATION_SPEED_RANGE: Range<f64> = Range::new(Some(0.1), Some(4.0), Some(0.05));
pub const CORNER_RADIUS_SCALE_RANGE: Range<f64> = Range::new(Some(0.0), Some(2.0), Some(0.05));
pub const CONTROL_CENTER_WIDTH_RANGE: Range<i64> = Range::new(Some(600), Some(1200), Some(10));
pub const SCREEN_CORNERS_SIZE_RANGE: Range<i64> = Range::new(Some(1), Some(100), Some(1));
pub const HOT_CORNERS_DELAY_MS_RANGE: Range<i64> = Range::new(Some(0), Some(2000), Some(50));

/// Clipboard history count — config limits imported from types.
pub const CLIPBOARD_HISTORY_MAX_ENTRIES_RANGE: Range<i64> =
    Range::new(Some(10), Some(10000), Some(10));

pub const SESSION_GRID_COLUMNS_RANGE: Range<i64> = Range::new(Some(1), Some(5), Some(1));

// Battery / wallpaper.
pub const BATTERY_WARNING_THRESHOLD_RANGE: Range<i64> = Range::new(Some(0), Some(100), Some(1));
pub const WALLPAPER_TRANSITION_DURATION_RANGE: Range<f64> =
    Range::new(Some(100.0), Some(30000.0), Some(100.0));
pub const WALLPAPER_AUTOMATION_INTERVAL_RANGE: Range<i64> =
    Range::new(Some(1), Some(86400), Some(1));

// Dock.
pub const DOCK_ICON_SIZE_RANGE: Range<i64> = Range::new(Some(16), Some(128), Some(1));
pub const DOCK_PADDING_RANGE: Range<i64> = Range::new(Some(0), Some(100), Some(1));
pub const DOCK_ITEM_SPACING_RANGE: Range<i64> = Range::new(Some(0), Some(100), Some(1));
pub const DOCK_MARGIN_ENDS_RANGE: Range<i64> = Range::new(Some(0), Some(500), Some(1));
pub const DOCK_MARGIN_EDGE_RANGE: Range<i64> = Range::new(Some(0), Some(100), Some(1));
/// radius + each corner.
pub const DOCK_RADIUS_RANGE: Range<i64> = Range::new(Some(0), Some(80), Some(1));
pub const DOCK_BORDER_WIDTH_RANGE: Range<f64> = Range::new(Some(0.0), Some(20.0), Some(0.5));
pub const DOCK_ACTIVE_SCALE_RANGE: Range<f64> = Range::new(Some(0.1), Some(1.75), Some(0.05));
pub const DOCK_INACTIVE_SCALE_RANGE: Range<f64> = Range::new(Some(0.1), Some(1.0), Some(0.05));
pub const DOCK_MAGNIFICATION_SCALE_RANGE: Range<f64> = Range::new(Some(1.0), Some(2.0), Some(0.05));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_range_clamps_below_min() {
        let r = Range::new(Some(0.5_f64), Some(2.5), None);
        assert_eq!(apply_range(0.1, &r), 0.5);
    }

    #[test]
    fn apply_range_clamps_above_max() {
        let r = Range::new(Some(0.5_f64), Some(2.5), None);
        assert_eq!(apply_range(3.0, &r), 2.5);
    }

    #[test]
    fn apply_range_leaves_value_in_range() {
        let r = Range::new(Some(0.5_f64), Some(2.5), None);
        assert_eq!(apply_range(1.5, &r), 1.5);
    }

    #[test]
    fn apply_range_min_only() {
        let r: Range<i64> = Range::new(Some(0), None, None);
        assert_eq!(apply_range(-5, &r), 0);
        assert_eq!(apply_range(100, &r), 100);
    }
}
