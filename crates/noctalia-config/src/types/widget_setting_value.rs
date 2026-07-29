//! Port of `src/config/widget_setting_value.h` (pulled forward from task 2.2
//! into task 2.1.2, per that task's own note in MIGRATION_PLAN.md: `2.1.2`'s
//! `WidgetConfig::settings` map is keyed by this type and won't compile
//! without it).
//!
//! The C++ header is a single `std::variant` alias plus two function
//! templates (`widgetSettingValueAs<T>`/`widgetSettingValueFrom<T>`)
//! specialized per-type via `if constexpr`. Rust has no variadic template
//! specialization, so the two directions become traits
//! ([`WidgetSettingValueAs`]/[`IntoWidgetSettingValue`]) implemented for the
//! concrete types each direction is actually used with today: `bool`,
//! `i64`, `f64`, `String`, `Vec<String>`, [`WidgetSettingStringMap`], and
//! `ColorSpec`. The C++ template also covers arbitrary integral/floating
//! types via `std::in_range`/range-clamped `static_cast` (for e.g. a
//! hypothetical `int32_t` caller) — narrowed here to the types with real
//! call sites (`WidgetConfig`'s `getInt`/`getDouble` in `config_types.cpp`,
//! `widget_definition.h`'s template call sites); extend the trait impls if a
//! future port needs another concrete type.

use std::collections::HashMap;

use noctalia_core::color::{ColorSpec, color_spec_from_config_string};

/// Port of `WidgetSettingStringMap` (widget_setting_value.h:18).
pub type WidgetSettingStringMap = HashMap<String, String>;

/// Port of `WidgetSettingValue` (widget_setting_value.h:19-20), the
/// `std::variant<bool, int64_t, double, string, vector<string>,
/// WidgetSettingStringMap>`.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetSettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    StringList(Vec<String>),
    StringMap(WidgetSettingStringMap),
}

/// Port of `widgetSettingValueAs<T>` (widget_setting_value.h:27-84): decodes
/// a raw widget setting to a typed value, or `None` if the stored variant
/// doesn't convert to `Self`. `context` is only meaningful for the
/// `ColorSpec` impl (forwarded to `color_spec_from_config_string`'s error
/// message).
pub trait WidgetSettingValueAs: Sized {
    fn widget_setting_value_as(value: &WidgetSettingValue, context: &str) -> Option<Self>;
}

/// Convenience free function mirroring the C++'s call-site shape
/// (`noctalia::config::widgetSettingValueAs<T>(value, context)`).
pub fn widget_setting_value_as<T: WidgetSettingValueAs>(
    value: &WidgetSettingValue,
    context: &str,
) -> Option<T> {
    T::widget_setting_value_as(value, context)
}

impl WidgetSettingValueAs for bool {
    fn widget_setting_value_as(value: &WidgetSettingValue, _context: &str) -> Option<Self> {
        match value {
            WidgetSettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl WidgetSettingValueAs for i64 {
    fn widget_setting_value_as(value: &WidgetSettingValue, _context: &str) -> Option<Self> {
        match value {
            WidgetSettingValue::Int(i) => Some(*i),
            // Port of the integral branch's double fallback (widget_setting_value.h:39-48):
            // round-to-nearest, reject non-finite or out-of-range.
            WidgetSettingValue::Float(f) => {
                if !f.is_finite() {
                    return None;
                }
                let rounded = f.round();
                if rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
                    Some(rounded as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl WidgetSettingValueAs for f64 {
    fn widget_setting_value_as(value: &WidgetSettingValue, _context: &str) -> Option<Self> {
        match value {
            // f64 <- f64 is always in range, so only the finiteness check applies
            // (widget_setting_value.h:50-57).
            WidgetSettingValue::Float(f) => {
                if f.is_finite() {
                    Some(*f)
                } else {
                    None
                }
            }
            // Unconditional cast, matching widget_setting_value.h:58-60.
            WidgetSettingValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl WidgetSettingValueAs for String {
    fn widget_setting_value_as(value: &WidgetSettingValue, _context: &str) -> Option<Self> {
        match value {
            WidgetSettingValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl WidgetSettingValueAs for Vec<String> {
    fn widget_setting_value_as(value: &WidgetSettingValue, _context: &str) -> Option<Self> {
        match value {
            WidgetSettingValue::StringList(v) => Some(v.clone()),
            // Port of widget_setting_value.h:69-71: a bare string is treated as a
            // single-element list.
            WidgetSettingValue::String(s) => Some(vec![s.clone()]),
            _ => None,
        }
    }
}

impl WidgetSettingValueAs for WidgetSettingStringMap {
    fn widget_setting_value_as(value: &WidgetSettingValue, _context: &str) -> Option<Self> {
        match value {
            WidgetSettingValue::StringMap(m) => Some(m.clone()),
            _ => None,
        }
    }
}

impl WidgetSettingValueAs for ColorSpec {
    /// Divergence from `widgetSettingValueAs<ColorSpec>` (widget_setting_value.h:76-79):
    /// the C++ calls `colorSpecFromConfigString` directly and lets its
    /// `std::runtime_error` on an unparseable string propagate uncaught out of
    /// `WidgetConfig::getColorSpec`/`getOptionalColorSpec` (config_types.cpp:282-302,
    /// neither of which catches it either). This trait's `Option<Self>` return has no
    /// channel for that error, so a malformed color string here decodes to `None` and
    /// silently falls back to the caller-supplied default instead of throwing. Accepted
    /// for now because every real caller today gets these strings through the
    /// schema-validated config path (task 2.3, not yet ported), where a malformed value
    /// shouldn't reach this deep in practice; revisit (e.g. widen this trait to return
    /// `Result`) if a caller ever needs to observe the parse failure itself.
    fn widget_setting_value_as(value: &WidgetSettingValue, context: &str) -> Option<Self> {
        match value {
            WidgetSettingValue::String(s) => color_spec_from_config_string(s, context).ok(),
            _ => None,
        }
    }
}

/// Port of `widgetSettingValueFrom<T>`'s overflow_error (widget_setting_value.h:92,98):
/// raised when converting an out-of-range/non-finite value into a `WidgetSettingValue`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WidgetSettingValueOverflowError {
    #[error("widget setting integer is outside the supported range")]
    IntegerOutOfRange,
    #[error("widget setting number is not finite or is outside the supported range")]
    NotFinite,
}

/// Port of `widgetSettingValueFrom<T>` (widget_setting_value.h:87-112): converts a
/// typed setting to the storage representation used by `WidgetConfig`.
pub trait IntoWidgetSettingValue {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError>;
}

impl IntoWidgetSettingValue for bool {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        Ok(WidgetSettingValue::Bool(self))
    }
}

impl IntoWidgetSettingValue for i64 {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        // T = i64 always satisfies in_range<int64_t>(value) trivially; the check is
        // real for the C++ template's other integral instantiations only.
        Ok(WidgetSettingValue::Int(self))
    }
}

impl IntoWidgetSettingValue for f64 {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        if self.is_finite() {
            Ok(WidgetSettingValue::Float(self))
        } else {
            Err(WidgetSettingValueOverflowError::NotFinite)
        }
    }
}

impl IntoWidgetSettingValue for String {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        Ok(WidgetSettingValue::String(self))
    }
}

impl IntoWidgetSettingValue for Vec<String> {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        Ok(WidgetSettingValue::StringList(self))
    }
}

impl IntoWidgetSettingValue for WidgetSettingStringMap {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        Ok(WidgetSettingValue::StringMap(self))
    }
}

impl IntoWidgetSettingValue for ColorSpec {
    fn into_widget_setting_value(
        self,
    ) -> Result<WidgetSettingValue, WidgetSettingValueOverflowError> {
        Ok(WidgetSettingValue::String(
            noctalia_core::color::color_spec_to_config_string(&self),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn bool_round_trips() {
        let v = true.into_widget_setting_value().unwrap();
        assert_eq!(v, WidgetSettingValue::Bool(true));
        assert_eq!(widget_setting_value_as::<bool>(&v, ""), Some(true));
    }

    #[test]
    fn int_reads_back_from_stored_int() {
        let v = WidgetSettingValue::Int(42);
        assert_eq!(widget_setting_value_as::<i64>(&v, ""), Some(42));
    }

    #[test]
    fn int_reads_back_from_finite_float_by_rounding() {
        let v = WidgetSettingValue::Float(2.6);
        assert_eq!(widget_setting_value_as::<i64>(&v, ""), Some(3));
    }

    #[test]
    fn int_rejects_non_finite_float() {
        let v = WidgetSettingValue::Float(f64::NAN);
        assert_eq!(widget_setting_value_as::<i64>(&v, ""), None);
        let v = WidgetSettingValue::Float(f64::INFINITY);
        assert_eq!(widget_setting_value_as::<i64>(&v, ""), None);
    }

    #[test]
    fn float_reads_back_from_stored_int_unconditionally() {
        let v = WidgetSettingValue::Int(7);
        assert_eq!(widget_setting_value_as::<f64>(&v, ""), Some(7.0));
    }

    #[test]
    fn float_rejects_non_finite_stored_float() {
        let v = WidgetSettingValue::Float(f64::NAN);
        assert_eq!(widget_setting_value_as::<f64>(&v, ""), None);
    }

    #[test]
    fn string_list_accepts_a_single_bare_string() {
        let v = WidgetSettingValue::String("solo".to_string());
        assert_eq!(
            widget_setting_value_as::<Vec<String>>(&v, ""),
            Some(vec!["solo".to_string()])
        );
    }

    #[test]
    fn string_map_round_trips() {
        let mut map = WidgetSettingStringMap::new();
        map.insert("eDP-1".to_string(), "laptop".to_string());
        let v = map.clone().into_widget_setting_value().unwrap();
        assert_eq!(
            widget_setting_value_as::<WidgetSettingStringMap>(&v, ""),
            Some(map)
        );
    }

    #[test]
    fn color_spec_parses_through_config_string() {
        let v = WidgetSettingValue::String("primary".to_string());
        let spec =
            widget_setting_value_as::<ColorSpec>(&v, "widget.color").expect("valid role token");
        assert_eq!(spec.role, Some(noctalia_core::color::ColorRole::Primary));
    }

    #[test]
    fn color_spec_rejects_invalid_string() {
        let v = WidgetSettingValue::String("not-a-color".to_string());
        assert_eq!(widget_setting_value_as::<ColorSpec>(&v, ""), None);
    }

    #[test]
    fn color_spec_rejects_non_string_variant() {
        let v = WidgetSettingValue::Bool(true);
        assert_eq!(widget_setting_value_as::<ColorSpec>(&v, ""), None);
    }

    #[test]
    fn cross_type_reads_return_none() {
        let v = WidgetSettingValue::Bool(true);
        assert_eq!(widget_setting_value_as::<String>(&v, ""), None);
        assert_eq!(widget_setting_value_as::<i64>(&v, ""), None);
    }

    #[test]
    fn non_finite_float_is_rejected_on_the_write_path() {
        let err = f64::NAN.into_widget_setting_value().unwrap_err();
        assert_eq!(err, WidgetSettingValueOverflowError::NotFinite);
    }
}
