//! Widget setting schema types.
//! Port of `src/config/schema/widget_schema.h`.

use crate::types::widget_setting_value::WidgetSettingValue;

/// The data/validation type of a setting value.
/// Port of `WidgetSettingType` (widget_schema.h:21-30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetSettingType {
    Bool,
    Int,
    Double,
    OptionalDouble,
    String,
    StringList,
    StringMap,
    Enum,
    Color,
}

/// One widget setting's schema.
/// Port of `WidgetSettingField` (widget_schema.h:36-43).
#[derive(Debug, Clone)]
pub struct WidgetSettingField {
    pub key: String,
    pub setting_type: WidgetSettingType,
    pub default_value: WidgetSettingValue,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub step: Option<f64>,
    /// Allowed values when `setting_type == Enum`.
    pub enum_values: Vec<String>,
}

impl Default for WidgetSettingField {
    fn default() -> Self {
        Self {
            key: String::new(),
            setting_type: WidgetSettingType::String,
            default_value: WidgetSettingValue::String(String::new()),
            min_value: None,
            max_value: None,
            step: None,
            enum_values: Vec::new(),
        }
    }
}

/// Port of `WidgetSettingSchema` (widget_schema.h:46).
pub type WidgetSettingSchema = Vec<WidgetSettingField>;
