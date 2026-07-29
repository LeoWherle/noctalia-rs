//! Schema engine — read, write, and unknown-key detection for schema-backed structs.
//! Port of `src/config/schema/engine.h`.

use std::collections::HashSet;

use super::diagnostics::{Diagnostics, join_path};
use super::field::{Field, Schema};

/// Populate `out` from `tbl` by running every field's reader.
/// Port of `readInto` (engine.h:17-23).
pub fn read_into<S>(
    tbl: &toml::Table,
    out: &mut S,
    schema: &Schema<S>,
    parent_path: &str,
    diag: &mut Diagnostics,
) {
    for f in schema {
        (f.read)(tbl, out, parent_path, diag);
    }
}

/// Serialize `in_` into a fresh table by running every field's writer, in schema order.
/// Port of `writeTable` (engine.h:27-33).
pub fn write_table<S>(in_: &S, schema: &Schema<S>) -> toml::Table {
    let mut tbl = toml::Table::new();
    for f in schema {
        (f.write)(&mut tbl, in_);
    }
    tbl
}

/// Append dotted paths of keys in `tbl` that no field in `schema` recognizes.
/// Port of `collectUnknownKeys` (engine.h:37-58).
pub fn collect_unknown_keys<S>(
    tbl: &toml::Table,
    schema: &Schema<S>,
    parent_path: &str,
    unknown: &mut Vec<String>,
) {
    let known: HashSet<&str> = schema.iter().map(|f| f.key).collect();
    for (key, _) in tbl {
        if !known.contains(key.as_str()) {
            unknown.push(join_path(parent_path, key));
        }
    }
    for f in schema {
        if let Some(ref find) = f.find_unknown {
            find(tbl, parent_path, unknown);
        }
    }
}

/// Nested struct under a fixed key (e.g. `shell.shadow`).
/// Port of `subTable` (engine.h:173-191).
pub fn sub_table<Parent: Send + Sync + 'static, Sub: Default + Send + Sync + 'static>(
    key: &'static str,
    get: fn(&Parent) -> &Sub,
    set: fn(&mut Parent, Sub),
    sub_schema: &'static Schema<Sub>,
) -> Field<Parent> {
    Field {
        key,
        read: Box::new(move |tbl, out, parent, diag| {
            if let Some(toml::Value::Table(sub)) = tbl.get(key) {
                let mut sub_out = Sub::default();
                read_into(sub, &mut sub_out, sub_schema, &join_path(parent, key), diag);
                set(out, sub_out);
            }
        }),
        write: Box::new(move |tbl, s| {
            tbl.insert(
                key.to_string(),
                toml::Value::Table(write_table(get(s), sub_schema)),
            );
        }),
        find_unknown: Some(Box::new(move |tbl, parent, unknown| {
            if let Some(toml::Value::Table(sub)) = tbl.get(key) {
                collect_unknown_keys(sub, sub_schema, &join_path(parent, key), unknown);
            }
        })),
    }
}

/// Dynamic `[parent.<name>]` sub-tables read into a `Vec<Elem>`.
/// Port of `namedMap` (engine.h:65-117).
pub fn named_map<Parent: Send + Sync + 'static, Elem: Default + Send + Sync + 'static>(
    key: &'static str,
    get: fn(&Parent) -> &[Elem],
    set: fn(&mut Parent, Vec<Elem>),
    sub_schema: &'static Schema<Elem>,
    set_name: fn(&mut Elem, &str),
    get_name: fn(&Elem) -> &str,
    read_skip_empty_name: bool,
) -> Field<Parent> {
    Field {
        key,
        read: Box::new(move |tbl, out, parent, diag| {
            if let Some(toml::Value::Table(map)) = tbl.get(key) {
                let mut vec = Vec::new();
                for (name, node) in map {
                    if let toml::Value::Table(sub) = node {
                        let mut elem = Elem::default();
                        set_name(&mut elem, name);
                        read_into(
                            sub,
                            &mut elem,
                            sub_schema,
                            &join_path(&join_path(parent, key), name),
                            diag,
                        );
                        if read_skip_empty_name && get_name(&elem).is_empty() {
                            continue;
                        }
                        vec.push(elem);
                    }
                }
                set(out, vec);
            }
        }),
        write: Box::new(move |tbl, s| {
            let elems = get(s);
            if elems.is_empty() {
                return;
            }
            let mut map = toml::Table::new();
            for elem in elems {
                let name = get_name(elem);
                if name.is_empty() {
                    continue;
                }
                map.insert(
                    name.to_string(),
                    toml::Value::Table(write_table(elem, sub_schema)),
                );
            }
            tbl.insert(key.to_string(), toml::Value::Table(map));
        }),
        find_unknown: Some(Box::new(move |tbl, parent, unknown| {
            if let Some(toml::Value::Table(map)) = tbl.get(key) {
                for (name, node) in map {
                    if let toml::Value::Table(sub) = node {
                        collect_unknown_keys(
                            sub,
                            sub_schema,
                            &join_path(&join_path(parent, key), name),
                            unknown,
                        );
                    }
                }
            }
        })),
    }
}

/// Array-of-tables read into a `Vec<Elem>`.
/// Port of `arrayOf` (engine.h:123-169).
pub fn array_of<Parent: Send + Sync + 'static, Elem: Default + Send + Sync + 'static>(
    key: &'static str,
    get: fn(&Parent) -> &[Elem],
    set: fn(&mut Parent, Vec<Elem>),
    sub_schema: &'static Schema<Elem>,
    keep: fn(&Elem) -> bool,
) -> Field<Parent> {
    Field {
        key,
        read: Box::new(move |tbl, out, parent, diag| {
            if let Some(toml::Value::Array(arr)) = tbl.get(key) {
                let mut vec = Vec::new();
                for node in arr {
                    if let toml::Value::Table(sub) = node {
                        let mut elem = Elem::default();
                        read_into(sub, &mut elem, sub_schema, &join_path(parent, key), diag);
                        if keep(&elem) {
                            vec.push(elem);
                        }
                    }
                }
                set(out, vec);
            }
        }),
        write: Box::new(move |tbl, s| {
            let arr: Vec<toml::Value> = get(s)
                .iter()
                .filter(|elem| keep(elem))
                .map(|elem| toml::Value::Table(write_table(elem, sub_schema)))
                .collect();
            tbl.insert(key.to_string(), toml::Value::Array(arr));
        }),
        find_unknown: Some(Box::new(move |tbl, parent, unknown| {
            if let Some(toml::Value::Array(arr)) = tbl.get(key) {
                for node in arr {
                    if let toml::Value::Table(sub) = node {
                        collect_unknown_keys(sub, sub_schema, &join_path(parent, key), unknown);
                    }
                }
            }
        })),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::field::*;
    use super::*;

    #[derive(Debug, Default, PartialEq)]
    struct Outer {
        flag: bool,
        name: String,
    }

    #[test]
    fn read_into_populates_struct_from_toml() {
        let schema: Schema<Outer> = vec![
            bool_field("flag", |s| s.flag, |s, v| s.flag = v),
            string_field("name", |s| &s.name, |s, v| s.name = v),
        ];
        let tbl: toml::Table = toml::from_str("flag = true\nname = \"hello\"").unwrap();
        let mut out = Outer::default();
        let mut diag = Diagnostics::default();
        read_into(&tbl, &mut out, &schema, "", &mut diag);
        assert!(out.flag);
        assert_eq!(out.name, "hello");
    }

    #[test]
    fn write_table_emits_all_fields() {
        let schema: Schema<Outer> = vec![
            bool_field("flag", |s| s.flag, |s, v| s.flag = v),
            string_field("name", |s| &s.name, |s, v| s.name = v),
        ];
        let s = Outer {
            flag: true,
            name: "test".to_string(),
        };
        let tbl = write_table(&s, &schema);
        assert_eq!(tbl.get("flag"), Some(&toml::Value::Boolean(true)));
        assert_eq!(
            tbl.get("name"),
            Some(&toml::Value::String("test".to_string()))
        );
    }

    #[test]
    fn collect_unknown_keys_finds_extra_keys() {
        let schema: Schema<Outer> = vec![bool_field("flag", |s| s.flag, |s, v| s.flag = v)];
        let tbl: toml::Table = toml::from_str("flag = true\nextra = 42").unwrap();
        let mut unknown = Vec::new();
        collect_unknown_keys(&tbl, &schema, "section", &mut unknown);
        assert_eq!(unknown, vec!["section.extra"]);
    }

    #[test]
    fn read_into_and_write_table_are_inverses() {
        let schema: Schema<Outer> = vec![
            bool_field("flag", |s| s.flag, |s, v| s.flag = v),
            string_field("name", |s| &s.name, |s, v| s.name = v),
        ];
        let original = Outer {
            flag: true,
            name: "roundtrip".to_string(),
        };
        let tbl = write_table(&original, &schema);
        let mut roundtrip = Outer::default();
        let mut diag = Diagnostics::default();
        read_into(&tbl, &mut roundtrip, &schema, "", &mut diag);
        assert_eq!(roundtrip.flag, original.flag);
        assert_eq!(roundtrip.name, original.name);
    }
}
