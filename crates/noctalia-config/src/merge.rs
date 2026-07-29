//! Config table merging. Task 2.5.1: `ConfigService::deepMerge` only (despite
//! the name, this function lives in `src/config/config_service.cpp:1353`, not
//! `config_merge.{cpp,h}` — that file's `mergeConfigWithIncludes` is task
//! 2.5.3, which builds on this). See MIGRATION_PLAN.md's 2.5 split for why
//! the rest of `config_overrides.cpp` isn't here.

/// Recursively merges `overlay` into `base` in place: a key present as a
/// table in both merges recursively; anything else (including arrays — they
/// are never element-wise merged) has `overlay`'s value replace `base`'s
/// wholesale.
///
/// Port of `ConfigService::deepMerge` (`config_service.cpp:1353-1366`).
pub fn deep_merge(base: &mut toml::Table, overlay: &toml::Table) {
    for (k, v) in overlay {
        if let toml::Value::Table(overlay_tbl) = v
            && let Some(toml::Value::Table(base_tbl)) = base.get_mut(k)
        {
            deep_merge(base_tbl, overlay_tbl);
            continue;
        }
        base.insert(k.clone(), v.clone());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn parse(s: &str) -> toml::Table {
        toml::from_str(s).expect("test fixture must be valid TOML")
    }

    #[test]
    fn nested_tables_recurse() {
        let mut base = parse("[a]\nx = 1\ny = 2\n");
        let overlay = parse("[a]\ny = 20\nz = 30\n");
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base.get("a")
                .and_then(|v| v.get("x"))
                .and_then(|v| v.as_integer()),
            Some(1)
        );
        assert_eq!(
            base.get("a")
                .and_then(|v| v.get("y"))
                .and_then(|v| v.as_integer()),
            Some(20)
        );
        assert_eq!(
            base.get("a")
                .and_then(|v| v.get("z"))
                .and_then(|v| v.as_integer()),
            Some(30)
        );
    }

    #[test]
    fn deeply_nested_tables_recurse() {
        let mut base = parse("[a.b]\nx = 1\n[a.c]\nx = 1\n");
        let overlay = parse("[a.b]\nx = 2\n");
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base.get("a")
                .and_then(|v| v.get("b"))
                .and_then(|v| v.get("x"))
                .and_then(|v| v.as_integer()),
            Some(2)
        );
        assert_eq!(
            base.get("a")
                .and_then(|v| v.get("c"))
                .and_then(|v| v.get("x"))
                .and_then(|v| v.as_integer()),
            Some(1)
        );
    }

    #[test]
    fn arrays_replace_wholesale_not_element_wise() {
        let mut base = parse("list = [1, 2, 3]\n");
        let overlay = parse("list = [9]\n");
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base.get("list").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(1)
        );
    }

    #[test]
    fn table_over_non_table_replaces_wholesale() {
        let mut base = parse("a = 1\n");
        let overlay = parse("[a]\nx = 1\n");
        deep_merge(&mut base, &overlay);
        assert!(base.get("a").expect("a present").is_table());
    }

    #[test]
    fn non_table_over_table_replaces_wholesale() {
        let mut base = parse("[a]\nx = 1\n");
        let overlay = parse("a = 1\n");
        deep_merge(&mut base, &overlay);
        assert_eq!(base.get("a").and_then(|v| v.as_integer()), Some(1));
    }

    #[test]
    fn new_top_level_key_is_added() {
        let mut base = parse("a = 1\n");
        let overlay = parse("b = 2\n");
        deep_merge(&mut base, &overlay);
        assert_eq!(base.get("a").and_then(|v| v.as_integer()), Some(1));
        assert_eq!(base.get("b").and_then(|v| v.as_integer()), Some(2));
    }

    #[test]
    fn new_nested_table_key_is_added_wholesale() {
        let mut base = parse("a = 1\n");
        let overlay = parse("[b.c]\nx = 1\n");
        deep_merge(&mut base, &overlay);
        assert_eq!(base.get("a").and_then(|v| v.as_integer()), Some(1));
        assert_eq!(
            base.get("b")
                .and_then(|v| v.get("c"))
                .and_then(|v| v.get("x"))
                .and_then(|v| v.as_integer()),
            Some(1)
        );
    }

    #[test]
    fn empty_overlay_leaves_base_unchanged() {
        let mut base = parse("[a]\nx = 1\n");
        let overlay = toml::Table::new();
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base.get("a")
                .and_then(|v| v.get("x"))
                .and_then(|v| v.as_integer()),
            Some(1)
        );
    }
}
