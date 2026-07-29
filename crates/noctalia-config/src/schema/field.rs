//! Field descriptors and the Schema type.
//! Port of `src/config/schema/field.h` — the generic "one TOML key ↔ one struct member" bridge.
//!
//! In Rust we cannot use pointer-to-member like C++. Instead, `Field<S>` stores
//! boxed closures that read/write through `&mut S` / `&S`. The factory functions
//! (`bool_field`, `string_field`, `i32_field`, …) create these closures for the
//! common leaf types, mirroring the C++ `field(bool Struct::*member, …)` overloads.

use std::collections::HashMap;

use super::diagnostics::{Diagnostics, join_path};
use super::ranges::{Range, apply_range};

/// Type alias for the read closure of a schema field.
pub type ReadFn<S> = Box<dyn Fn(&toml::Table, &mut S, &str, &mut Diagnostics) + Send + Sync>;
/// Type alias for the write closure of a schema field.
pub type WriteFn<S> = Box<dyn Fn(&mut toml::Table, &S) + Send + Sync>;
/// Type alias for the unknown-key finder closure of a schema field.
pub type FindUnknownFn = Box<dyn Fn(&toml::Table, &str, &mut Vec<String>) + Send + Sync>;

/// One descriptor: binds a single TOML key in a parent table to part of a Struct.
///
/// Port of `Field<Struct>` (field.h:57-65).
pub struct Field<S> {
    pub key: &'static str,
    pub read: ReadFn<S>,
    pub write: WriteFn<S>,
    /// Set only for composite fields (sub-tables/named-maps/arrays): recurse into
    /// this key and append dotted paths of unrecognized child keys.
    pub find_unknown: Option<FindUnknownFn>,
}

/// Ordered set of fields for one struct.
/// Port of `Schema<Struct>` (field.h:69).
pub type Schema<S> = Vec<Field<S>>;

// ── Helper: extract a finite f64 from a TOML value ─────────────────────────

/// Mirror of `finiteDouble` (field.h:41-52).
fn finite_double(value: &toml::Value) -> Option<f64> {
    match value {
        toml::Value::Float(f) => {
            if f.is_finite() {
                Some(*f)
            } else {
                None
            }
        }
        toml::Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

// ── Leaf codec factories ───────────────────────────────────────────────────

/// `field(bool Struct::*member, key)` (field.h:73-83).
pub fn bool_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> bool,
    set: fn(&mut S, bool),
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Boolean(v)) = tbl.get(key) {
                set(out, *v);
            }
        }),
        write: Box::new(move |tbl, s| {
            tbl.insert(key.to_string(), toml::Value::Boolean(get(s)));
        }),
        find_unknown: None,
    }
}

/// `field(std::string Struct::*member, key)` (field.h:85-95).
pub fn string_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &str,
    set: fn(&mut S, String),
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                set(out, v.clone());
            }
        }),
        write: Box::new(move |tbl, s| {
            tbl.insert(key.to_string(), toml::Value::String(get(s).to_string()));
        }),
        find_unknown: None,
    }
}

/// `field(std::int32_t Struct::*member, key, range)` (field.h:97-115).
pub fn i32_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> i32,
    set: fn(&mut S, i32),
    range: Option<Range<i64>>,
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Integer(v)) = tbl.get(key) {
                let mut value = *v;
                if let Some(ref r) = range {
                    value = apply_range(value, r);
                }
                set(out, value as i32);
            }
        }),
        write: Box::new(move |tbl, s| {
            tbl.insert(key.to_string(), toml::Value::Integer(i64::from(get(s))));
        }),
        find_unknown: None,
    }
}

/// `field(float Struct::*member, key, range)` (field.h:117-134).
/// In Rust we use f32 fields but TOML stores f64; read/write converts.
pub fn f32_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> f32,
    set: fn(&mut S, f32),
    range: Option<Range<f64>>,
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(val) = tbl.get(key).and_then(finite_double) {
                let mut v = val as f32;
                if let Some(ref r) = range {
                    // Apply range on f64, then cast
                    let clamped = apply_range(val, r);
                    v = clamped as f32;
                }
                set(out, v);
            }
        }),
        write: Box::new(move |tbl, s| {
            tbl.insert(key.to_string(), toml::Value::Float(f64::from(get(s))));
        }),
        find_unknown: None,
    }
}

/// `field(double Struct::*member, key, range)` (field.h:136-152).
pub fn f64_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> f64,
    set: fn(&mut S, f64),
    range: Option<Range<f64>>,
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(val) = tbl.get(key).and_then(finite_double) {
                let v = if let Some(ref r) = range {
                    apply_range(val, r)
                } else {
                    val
                };
                set(out, v);
            }
        }),
        write: Box::new(move |tbl, s| {
            tbl.insert(key.to_string(), toml::Value::Float(get(s)));
        }),
        find_unknown: None,
    }
}

/// `field(std::optional<double> Struct::*member, key)` (field.h:156-170).
pub fn optional_f64_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> Option<f64>,
    set: fn(&mut S, Option<f64>),
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(val) = tbl.get(key).and_then(finite_double) {
                set(out, Some(val));
            }
        }),
        write: Box::new(move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::Float(v));
            }
        }),
        find_unknown: None,
    }
}

/// `field(std::optional<std::int32_t> Struct::*member, key, range)` (field.h:172-186).
pub fn optional_i32_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> Option<i32>,
    set: fn(&mut S, Option<i32>),
    range: Option<Range<i64>>,
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Integer(v)) = tbl.get(key) {
                let value = if let Some(ref r) = range {
                    apply_range(*v, r)
                } else {
                    *v
                };
                set(out, Some(value as i32));
            }
        }),
        write: Box::new(move |tbl, s| {
            if let Some(v) = get(s) {
                tbl.insert(key.to_string(), toml::Value::Integer(i64::from(v)));
            }
        }),
        find_unknown: None,
    }
}

/// `field(std::vector<std::string> Struct::*member, key)` (field.h:188-210).
pub fn string_vec_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &[String],
    set: fn(&mut S, Vec<String>),
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Array(arr)) = tbl.get(key) {
                let values: Vec<String> = arr
                    .iter()
                    .filter_map(|item| {
                        if let toml::Value::String(s) = item {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                set(out, values);
            }
        }),
        write: Box::new(move |tbl, s| {
            let arr: Vec<toml::Value> = get(s)
                .iter()
                .map(|v| toml::Value::String(v.clone()))
                .collect();
            tbl.insert(key.to_string(), toml::Value::Array(arr));
        }),
        find_unknown: None,
    }
}

/// `field(std::unordered_map<std::string, std::string> Struct::*member, key)` (field.h:214-243).
pub fn string_map_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &HashMap<String, String>,
    set: fn(&mut S, HashMap<String, String>),
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Table(sub)) = tbl.get(key) {
                let mut values = HashMap::new();
                for (entry_key, node) in sub {
                    if let toml::Value::String(value) = node {
                        values.insert(entry_key.clone(), value.clone());
                    }
                }
                set(out, values);
            }
        }),
        write: Box::new(move |tbl, s| {
            let map = get(s);
            if map.is_empty() {
                return;
            }
            let mut sub = toml::Table::new();
            for (entry_key, value) in map {
                sub.insert(entry_key.clone(), toml::Value::String(value.clone()));
            }
            tbl.insert(key.to_string(), toml::Value::Table(sub));
        }),
        find_unknown: None,
    }
}

/// `field(std::optional<std::unordered_map<…>> Struct::*member, key)` (field.h:247-276).
pub fn optional_string_map_field<S: Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> &Option<HashMap<String, String>>,
    set: fn(&mut S, Option<HashMap<String, String>>),
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, _parent, _diag| {
            if let Some(toml::Value::Table(sub)) = tbl.get(key) {
                let mut values = HashMap::new();
                for (entry_key, node) in sub {
                    if let toml::Value::String(value) = node {
                        values.insert(entry_key.clone(), value.clone());
                    }
                }
                set(out, Some(values));
            }
        }),
        write: Box::new(move |tbl, s| {
            if let Some(map) = get(s) {
                let mut sub = toml::Table::new();
                for (entry_key, value) in map {
                    sub.insert(entry_key.clone(), toml::Value::String(value.clone()));
                }
                tbl.insert(key.to_string(), toml::Value::Table(sub));
            }
        }),
        find_unknown: None,
    }
}

/// Escape hatch for a single key whose read/write don't fit a stock codec.
/// Port of `custom` (field.h:280-287).
pub fn custom_field<S: Send + Sync + 'static>(
    key: &'static str,
    read: impl Fn(&toml::Table, &mut S, &str, &mut Diagnostics) + Send + Sync + 'static,
    write: impl Fn(&mut toml::Table, &S) + Send + Sync + 'static,
) -> Field<S> {
    Field {
        key,
        read: Box::new(read),
        write: Box::new(write),
        find_unknown: None,
    }
}

/// A keyless field that runs cross-field logic after all leaf reads.
/// Port of `finalize` (field.h:322-331).
pub fn finalize<S: Send + Sync + 'static>(
    func: impl Fn(&mut S, &str, &mut Diagnostics) + Send + Sync + 'static,
) -> Field<S> {
    Field {
        key: "",
        read: Box::new(move |_tbl, out, parent, diag| {
            func(out, parent, diag);
        }),
        write: Box::new(|_tbl, _s| {}),
        find_unknown: None,
    }
}

/// Enum field backed by a key-value lookup table.
/// Port of `enumField` (field.h:382-401).
pub fn enum_field<S: Send + Sync + 'static, E: Copy + PartialEq + Send + Sync + 'static>(
    key: &'static str,
    get: fn(&S) -> E,
    set: fn(&mut S, E),
    options: &'static [(&'static str, E)],
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, parent, diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                let trimmed = v.trim();
                if let Some((_, val)) = options.iter().find(|(k, _)| *k == trimmed) {
                    set(out, *val);
                } else {
                    diag.warn(join_path(parent, key), format!("unknown value \"{v}\""));
                }
            }
        }),
        write: Box::new(move |tbl, s| {
            let val = get(s);
            if let Some((k, _)) = options.iter().find(|(_, v)| *v == val) {
                tbl.insert(key.to_string(), toml::Value::String((*k).to_string()));
            }
        }),
        find_unknown: None,
    }
}

/// Optional enum field.
/// Port of `optionalEnumField` (field.h:356-378).
pub fn optional_enum_field<
    S: Send + Sync + 'static,
    E: Copy + PartialEq + Send + Sync + 'static,
>(
    key: &'static str,
    get: fn(&S) -> Option<E>,
    set: fn(&mut S, Option<E>),
    options: &'static [(&'static str, E)],
) -> Field<S> {
    Field {
        key,
        read: Box::new(move |tbl, out, parent, diag| {
            if let Some(toml::Value::String(v)) = tbl.get(key) {
                let trimmed = v.trim();
                if let Some((_, val)) = options.iter().find(|(k, _)| *k == trimmed) {
                    set(out, Some(*val));
                } else {
                    diag.warn(join_path(parent, key), format!("unknown value \"{v}\""));
                }
            }
        }),
        write: Box::new(move |tbl, s| {
            if let Some(val) = get(s)
                && let Some((k, _)) = options.iter().find(|(_, v)| *v == val)
            {
                tbl.insert(key.to_string(), toml::Value::String((*k).to_string()));
            }
        }),
        find_unknown: None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq)]
    struct TestStruct {
        flag: bool,
        name: String,
        count: i32,
        scale: f32,
        ratio: f64,
        tags: Vec<String>,
    }

    #[test]
    fn bool_field_reads_and_writes() {
        let field = bool_field::<TestStruct>("flag", |s| s.flag, |s, v| s.flag = v);
        let tbl: toml::Table = toml::from_str("flag = true").unwrap();
        let mut out = TestStruct::default();
        let mut diag = Diagnostics::default();
        (field.read)(&tbl, &mut out, "", &mut diag);
        assert!(out.flag);

        let mut write_tbl = toml::Table::new();
        (field.write)(&mut write_tbl, &out);
        assert_eq!(write_tbl.get("flag"), Some(&toml::Value::Boolean(true)));
    }

    #[test]
    fn string_field_reads_and_writes() {
        let field = string_field::<TestStruct>("name", |s| &s.name, |s, v| s.name = v);
        let tbl: toml::Table = toml::from_str("name = \"hello\"").unwrap();
        let mut out = TestStruct::default();
        let mut diag = Diagnostics::default();
        (field.read)(&tbl, &mut out, "", &mut diag);
        assert_eq!(out.name, "hello");

        let mut write_tbl = toml::Table::new();
        (field.write)(&mut write_tbl, &out);
        assert_eq!(
            write_tbl.get("name"),
            Some(&toml::Value::String("hello".to_string()))
        );
    }

    #[test]
    fn i32_field_clamps_via_range() {
        let range = Some(Range::new(Some(0), Some(100), None));
        let field = i32_field::<TestStruct>("count", |s| s.count, |s, v| s.count = v, range);
        let tbl: toml::Table = toml::from_str("count = 200").unwrap();
        let mut out = TestStruct::default();
        let mut diag = Diagnostics::default();
        (field.read)(&tbl, &mut out, "", &mut diag);
        assert_eq!(out.count, 100);
    }

    #[test]
    fn f32_field_reads_int_as_float() {
        let field = f32_field::<TestStruct>("scale", |s| s.scale, |s, v| s.scale = v, None);
        let tbl: toml::Table = toml::from_str("scale = 2").unwrap();
        let mut out = TestStruct::default();
        let mut diag = Diagnostics::default();
        (field.read)(&tbl, &mut out, "", &mut diag);
        assert_eq!(out.scale, 2.0);
    }

    #[test]
    fn string_vec_field_round_trips() {
        let field = string_vec_field::<TestStruct>("tags", |s| &s.tags, |s, v| s.tags = v);
        let tbl: toml::Table = toml::from_str("tags = [\"a\", \"b\"]").unwrap();
        let mut out = TestStruct::default();
        let mut diag = Diagnostics::default();
        (field.read)(&tbl, &mut out, "", &mut diag);
        assert_eq!(out.tags, vec!["a", "b"]);

        let mut write_tbl = toml::Table::new();
        (field.write)(&mut write_tbl, &out);
        let arr = write_tbl.get("tags").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Side {
        Left,
        Right,
    }

    const SIDE_OPTIONS: &[(&str, Side)] = &[("left", Side::Left), ("right", Side::Right)];

    #[derive(Debug, Default)]
    struct EnumTestStruct {
        side: Option<Side>,
    }

    #[test]
    fn enum_field_warns_on_unknown_value() {
        let field = optional_enum_field::<EnumTestStruct, Side>(
            "side",
            |s| s.side,
            |s, v| s.side = v,
            SIDE_OPTIONS,
        );
        let tbl: toml::Table = toml::from_str("side = \"middle\"").unwrap();
        let mut out = EnumTestStruct::default();
        let mut diag = Diagnostics::default();
        (field.read)(&tbl, &mut out, "test", &mut diag);
        assert!(out.side.is_none());
        assert_eq!(diag.entries.len(), 1);
        assert!(diag.entries[0].message.contains("middle"));
    }
}
