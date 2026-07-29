use std::collections::HashSet;
use std::path::{Path, PathBuf};

use noctalia_core::files::paths::{expand_env_vars, lexically_normal, resolve_path};
use noctalia_core::log::Logger;

const LOG: Logger = Logger::new("config");

/// Result of merging all config-dir files, including any pulled in via `[include]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergeResult {
    /// The merged TOML with every file's `[include]` table stripped out.
    pub merged: toml::Table,
    /// Every file actually loaded (root + included), canonicalized, in load order.
    pub loaded_files: Vec<PathBuf>,
    /// Directories named directly in an `[include].files` list (canonicalized).
    pub include_dirs: Vec<PathBuf>,
    /// First parse / missing-include error encountered, empty if none.
    pub first_error: String,
}

fn sorted_toml_in_dir(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("toml") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn canonical_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| lexically_normal(path))
}

struct IncludeDirective {
    files: Vec<String>,
    autoload: bool,
    has_autoload: bool,
}

fn read_include(tbl: &toml::Table) -> IncludeDirective {
    let mut directive = IncludeDirective {
        files: Vec::new(),
        autoload: true,
        has_autoload: false,
    };
    let Some(inc) = tbl.get("include").and_then(|v| v.as_table()) else {
        return directive;
    };
    if let Some(v) = inc.get("autoload").and_then(|v| v.as_bool()) {
        directive.autoload = v;
        directive.has_autoload = true;
    }
    if let Some(arr) = inc.get("files").and_then(|v| v.as_array()) {
        for node in arr {
            if let Some(s) = node.as_str() {
                directive.files.push(s.to_string());
            }
        }
    }
    directive
}

fn format_parse_error(path: &Path, content: &str, err: &toml::de::Error) -> String {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if let Some(span) = err.span() {
        let prefix = &content[..span.start.min(content.len())];
        let line = prefix.lines().count().max(1);
        let col = prefix.lines().last().map_or(1, |l| l.chars().count() + 1);
        format!("{filename} line {line}, column {col}: {err}")
    } else {
        format!("{filename}: {err}")
    }
}

fn expand_file(
    path: &Path,
    parsed: &toml::Table,
    visited: &mut HashSet<PathBuf>,
    out: &mut MergeResult,
) -> toml::Table {
    let key = canonical_key(path);
    if visited.contains(&key) {
        LOG.warn(format_args!(
            "config include cycle or duplicate skipped: {}",
            key.display()
        ));
        return toml::Table::new();
    }
    visited.insert(key.clone());
    out.loaded_files.push(key);

    let including_dir = path.parent().unwrap_or_else(|| Path::new(""));
    let directive = read_include(parsed);

    let mut base = toml::Table::new();
    for entry in &directive.files {
        let expanded = expand_env_vars(entry);
        let target = resolve_path(&expanded, Some(including_dir));

        if target.is_dir() {
            out.include_dirs.push(canonical_key(&target));
            for child in sorted_toml_in_dir(&target) {
                deep_merge(&mut base, &load_and_expand(&child, visited, out));
            }
        } else if target.is_file() {
            deep_merge(&mut base, &load_and_expand(&target, visited, out));
        } else {
            if out.first_error.is_empty() {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                out.first_error = format!("include not found: {entry} (from {filename})");
            }
            LOG.warn(format_args!(
                "config include not found: {} (from {})",
                target.display(),
                path.display()
            ));
        }
    }

    let mut body = parsed.clone();
    body.remove("include");
    deep_merge(&mut base, &body);
    base
}

fn load_and_expand(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    out: &mut MergeResult,
) -> toml::Table {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            if out.first_error.is_empty() {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                out.first_error = format!("failed to read {filename}: {e}");
            }
            LOG.warn(format_args!("failed to read {}: {e}", path.display()));
            return toml::Table::new();
        }
    };

    match content.parse::<toml::Table>() {
        Ok(parsed) => expand_file(path, &parsed, visited, out),
        Err(err) => {
            if out.first_error.is_empty() {
                out.first_error = format_parse_error(path, &content, &err);
            }
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            LOG.warn(format_args!("parse error in {filename}: {err}"));
            toml::Table::new()
        }
    }
}

/// Scans `config_dir` for `*.toml` (alphabetical) and merges them, honoring each
/// file's optional `[include]` table.
pub fn merge_config_with_includes(config_dir: &Path) -> MergeResult {
    let mut out = MergeResult::default();
    if config_dir.as_os_str().is_empty() {
        return out;
    }

    struct Root {
        path: PathBuf,
        table: toml::Table,
        opt_out: bool,
    }

    let mut roots = Vec::new();
    let mut any_opt_out = false;

    for path in sorted_toml_in_dir(config_dir) {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                if out.first_error.is_empty() {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    out.first_error = format!("failed to read {filename}: {e}");
                }
                LOG.warn(format_args!("failed to read {}: {e}", path.display()));
                continue;
            }
        };

        let tbl = match content.parse::<toml::Table>() {
            Ok(t) => t,
            Err(err) => {
                if out.first_error.is_empty() {
                    out.first_error = format_parse_error(&path, &content, &err);
                }
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                LOG.warn(format_args!("parse error in {filename}: {err}"));
                continue;
            }
        };

        let directive = read_include(&tbl);
        let opt_out = directive.has_autoload && !directive.autoload;
        any_opt_out = any_opt_out || opt_out;
        roots.push(Root {
            path,
            table: tbl,
            opt_out,
        });
    }

    let mut visited = HashSet::new();
    for root in roots {
        if any_opt_out && !root.opt_out {
            continue;
        }
        let expanded = expand_file(&root.path, &root.table, &mut visited, &mut out);
        deep_merge(&mut out.merged, &expanded);
    }

    out
}

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

    fn make_temp_dir(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("noctalia-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    #[test]
    fn multi_file_sorted_merge() {
        let dir = make_temp_dir("sorted-merge");
        std::fs::write(dir.join("01-base.toml"), "[shell]\nval = 1\nother = 'a'\n")
            .expect("write 01");
        std::fs::write(dir.join("02-override.toml"), "[shell]\nval = 2\n").expect("write 02");

        let res = merge_config_with_includes(&dir);
        assert!(res.first_error.is_empty());
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("val"))
                .and_then(|v| v.as_integer()),
            Some(2)
        );
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("other"))
                .and_then(|v| v.as_str()),
            Some("a")
        );
        assert_eq!(res.loaded_files.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn include_files_file_and_directory_forms() {
        let dir = make_temp_dir("include-forms");
        let extra_dir = dir.join("extra");
        std::fs::create_dir_all(&extra_dir).expect("mkdir extra");

        std::fs::write(dir.join("base.toml"), "[shell]\nbase_val = 10\n").expect("write base");
        std::fs::write(extra_dir.join("sub.toml"), "[shell]\nextra_val = 20\n").expect("write sub");
        std::fs::write(
            dir.join("config.toml"),
            "[include]\nfiles = ['base.toml', 'extra/']\n[shell]\nmain_val = 30\n",
        )
        .expect("write config");

        let res = merge_config_with_includes(&dir);
        assert!(res.first_error.is_empty());
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("base_val"))
                .and_then(|v| v.as_integer()),
            Some(10)
        );
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("extra_val"))
                .and_then(|v| v.as_integer()),
            Some(20)
        );
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("main_val"))
                .and_then(|v| v.as_integer()),
            Some(30)
        );
        assert_eq!(res.include_dirs.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycle_detection_skips_duplicates() {
        let dir = make_temp_dir("cycle");
        std::fs::write(
            dir.join("a.toml"),
            "[include]\nfiles = ['b.toml']\n[a]\nval = 1\n",
        )
        .expect("write a");
        std::fs::write(
            dir.join("b.toml"),
            "[include]\nfiles = ['a.toml']\n[b]\nval = 2\n",
        )
        .expect("write b");

        let res = merge_config_with_includes(&dir);
        assert_eq!(
            res.merged
                .get("a")
                .and_then(|v| v.get("val"))
                .and_then(|v| v.as_integer()),
            Some(1)
        );
        assert_eq!(
            res.merged
                .get("b")
                .and_then(|v| v.get("val"))
                .and_then(|v| v.as_integer()),
            Some(2)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autoload_false_opt_out_skips_other_roots() {
        let dir = make_temp_dir("autoload-optout");
        std::fs::write(dir.join("01-default.toml"), "[shell]\nfoo = 1\n").expect("write 01");
        std::fs::write(
            dir.join("02-custom.toml"),
            "[include]\nautoload = false\n[shell]\nfoo = 2\n",
        )
        .expect("write 02");

        let res = merge_config_with_includes(&dir);
        assert!(res.first_error.is_empty());
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("foo"))
                .and_then(|v| v.as_integer()),
            Some(2)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_var_expanded_include_path() {
        let dir = make_temp_dir("env-inc");
        let inc_dir = dir.join("inc_folder");
        std::fs::create_dir_all(&inc_dir).expect("mkdir inc_folder");
        std::fs::write(inc_dir.join("inc.toml"), "[shell]\nfrom_env = 99\n")
            .expect("write inc.toml");

        unsafe {
            std::env::set_var("NOCTALIA_TEST_INC_DIR", inc_dir.to_str().unwrap());
        }

        std::fs::write(
            dir.join("config.toml"),
            "[include]\nfiles = ['$NOCTALIA_TEST_INC_DIR/inc.toml']\n",
        )
        .expect("write config");

        let res = merge_config_with_includes(&dir);
        assert!(res.first_error.is_empty());
        assert_eq!(
            res.merged
                .get("shell")
                .and_then(|v| v.get("from_env"))
                .and_then(|v| v.as_integer()),
            Some(99)
        );

        unsafe {
            std::env::remove_var("NOCTALIA_TEST_INC_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
