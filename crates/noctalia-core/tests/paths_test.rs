//! `paths::assets_root()` caches its result in a process-wide `OnceLock`, so the
//! env-override test needs to be the first call to it in the process — hence its
//! own test binary, mirroring the approach in `tests/log_test.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use noctalia_core::files::paths;

#[test]
fn resolves_assets_root_from_env_override() {
    let root = make_temp_asset_bundle();
    // SAFETY: single-threaded test binary, one #[test] fn.
    unsafe {
        std::env::set_var("NOCTALIA_ASSETS_DIR", &root);
    }

    assert_eq!(paths::assets_root(), root.as_path());
    assert_eq!(paths::asset_path("emoji.json"), root.join("emoji.json"));

    let _ = fs::remove_dir_all(&root);
}

fn make_temp_asset_bundle() -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("noctalia-assets-override-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("fonts")).expect("failed to create fonts dir");
    fs::create_dir_all(root.join("templates")).expect("failed to create templates dir");
    fs::create_dir_all(root.join("translations")).expect("failed to create translations dir");
    fs::write(root.join("emoji.json"), "{}").expect("failed to write emoji.json");
    fs::write(root.join("fonts").join("tabler.ttf"), b"").expect("failed to write tabler.ttf");
    fs::write(root.join("templates").join("builtin.toml"), "")
        .expect("failed to write builtin.toml");
    fs::write(root.join("translations").join("en.json"), "{}").expect("failed to write en.json");
    root
}
