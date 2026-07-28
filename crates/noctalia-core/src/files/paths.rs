//! Port of `src/core/files/resource_paths.{cpp,h}`.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::log::Logger;

const LOG: Logger = Logger::new("paths");

// Provisional: meson injects the real install prefix/datadir
// (`get_option('prefix')`/`get_option('datadir')`) at compile time. The Rust build
// has no install-layout story yet — that lands with the Phase 16.5 cutover task
// (`nix/package.nix` gains a Rust build) — so these are placeholders until then.
const INSTALL_PREFIX: &str = "/usr";
const INSTALL_DATADIR: &str = "share";

// Set by build.rs to `<repo root>/assets`, mirroring meson's
// `-DNOCTALIA_SOURCE_ASSETS_DIR="<source root>/assets"`.
const SOURCE_ASSETS_DIR: &str = env!("NOCTALIA_SOURCE_ASSETS_DIR");

fn installed_assets_root() -> PathBuf {
    let datadir = Path::new(INSTALL_DATADIR);
    if datadir.is_absolute() {
        datadir.join("noctalia").join("assets")
    } else {
        Path::new(INSTALL_PREFIX)
            .join(datadir)
            .join("noctalia")
            .join("assets")
    }
}

fn source_assets_root() -> PathBuf {
    PathBuf::from(SOURCE_ASSETS_DIR)
}

fn is_asset_root(root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }

    root.join("emoji.json").exists()
        && root.join("fonts").join("tabler.ttf").exists()
        && root.join("templates").join("builtin.toml").exists()
        && root.join("translations").join("en.json").exists()
}

fn executable_path() -> Option<PathBuf> {
    std::fs::read_link("/proc/self/exe").ok()
}

fn append_unique(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidate.as_os_str().is_empty() {
        return;
    }
    if !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn asset_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(env_override) = env::var("NOCTALIA_ASSETS_DIR")
        && !env_override.is_empty()
    {
        let override_path = PathBuf::from(&env_override);
        if is_asset_root(&override_path) {
            candidates.push(override_path);
            return candidates;
        }
        LOG.warn(format_args!(
            "NOCTALIA_ASSETS_DIR is not a valid asset bundle: {}",
            override_path.display()
        ));
    }

    if let Some(exe_dir) = executable_path().as_deref().and_then(Path::parent) {
        append_unique(&mut candidates, exe_dir.join("assets"));
        if let Some(exe_parent) = exe_dir.parent() {
            append_unique(&mut candidates, exe_parent.join("assets"));

            let datadir = Path::new(INSTALL_DATADIR);
            if !datadir.as_os_str().is_empty() && !datadir.is_absolute() {
                append_unique(
                    &mut candidates,
                    exe_parent.join(datadir).join("noctalia").join("assets"),
                );
            }
            append_unique(
                &mut candidates,
                exe_parent.join("share").join("noctalia").join("assets"),
            );
        }
    }

    append_unique(&mut candidates, installed_assets_root());
    append_unique(&mut candidates, source_assets_root());
    candidates
}

fn resolve_assets_root() -> PathBuf {
    for candidate in asset_candidates() {
        if is_asset_root(&candidate) {
            LOG.debug(format_args!("using assets from {}", candidate.display()));
            return candidate;
        }
    }

    let fallback = installed_assets_root();
    LOG.warn(format_args!(
        "could not locate a valid asset bundle; defaulting to {}",
        fallback.display()
    ));
    fallback
}

static ASSETS_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn assets_root() -> &'static Path {
    ASSETS_ROOT.get_or_init(resolve_assets_root)
}

pub fn asset_path(relative_path: &str) -> PathBuf {
    assets_root().join(relative_path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    fn make_temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    #[test]
    fn empty_root_is_not_an_asset_root() {
        assert!(!is_asset_root(Path::new("")));
    }

    #[test]
    fn incomplete_bundle_is_not_an_asset_root() {
        let root = make_temp_dir("noctalia-asset-root-incomplete");
        assert!(!is_asset_root(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn complete_bundle_is_an_asset_root() {
        let root = make_temp_dir("noctalia-asset-root-complete");
        fs::create_dir_all(root.join("fonts")).expect("mkdir fonts");
        fs::create_dir_all(root.join("templates")).expect("mkdir templates");
        fs::create_dir_all(root.join("translations")).expect("mkdir translations");
        fs::write(root.join("emoji.json"), "{}").expect("write emoji.json");
        fs::write(root.join("fonts").join("tabler.ttf"), b"").expect("write tabler.ttf");
        fs::write(root.join("templates").join("builtin.toml"), "").expect("write builtin.toml");
        fs::write(root.join("translations").join("en.json"), "{}").expect("write en.json");

        assert!(is_asset_root(&root));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn source_assets_root_points_at_the_real_repo_assets() {
        // This is the repo's actual `assets/` dir (build.rs resolves it relative to
        // the crate's manifest dir), so it should always satisfy `is_asset_root`.
        assert!(is_asset_root(&source_assets_root()));
    }
}
