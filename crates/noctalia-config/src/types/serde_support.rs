//! Shared `serde(with = ...)` bridges between `ColorSpec` and its
//! config-string representation (`colorSpecFromConfigString`/
//! `colorSpecToConfigString`, ported in task 2.1.1). Extracted from `bar.rs`
//! (where these first landed as private helpers) once a second struct
//! (`shell.rs`, task 2.1.3) needed the exact same bridge — no behavior
//! change, just a shared home for both.

/// `ColorSpec` <-> config-string bridge for required (non-`Option`) fields.
pub mod color_spec_serde {
    use noctalia_core::color::{
        ColorSpec, color_spec_from_config_string, color_spec_to_config_string,
    };
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(spec: &ColorSpec, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&color_spec_to_config_string(spec))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ColorSpec, D::Error> {
        let raw = String::deserialize(deserializer)?;
        color_spec_from_config_string(&raw, "").map_err(serde::de::Error::custom)
    }
}

/// `Option<ColorSpec>` <-> config-string bridge.
pub mod optional_color_spec_serde {
    use noctalia_core::color::{
        ColorSpec, color_spec_from_config_string, color_spec_to_config_string,
    };
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        spec: &Option<ColorSpec>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match spec {
            Some(s) => serializer.serialize_some(&color_spec_to_config_string(s)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<ColorSpec>, D::Error> {
        let raw = Option::<String>::deserialize(deserializer)?;
        raw.map(|s| color_spec_from_config_string(&s, ""))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
