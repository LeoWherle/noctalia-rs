fn main() {
    // Mirrors meson's `-DNOCTALIA_SOURCE_ASSETS_DIR="<source root>/assets"` (see
    // meson.build). `crates/noctalia-core` is two levels below the repo root.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let source_assets_dir = std::path::Path::new(&manifest_dir).join("../../assets");
    println!(
        "cargo:rustc-env=NOCTALIA_SOURCE_ASSETS_DIR={}",
        source_assets_dir.display()
    );

    // Mirrors meson's `vcs_tag()` call in meson.build line for line: same `git
    // describe` invocation, same "unknown" fallback when git isn't on `PATH` or
    // the tree isn't a git checkout (e.g. a Nix store path with `.git` filtered
    // out). `git describe` only reads local repository metadata, so this needs
    // no network access even in the Nix build sandbox. Read back via `env!()` in
    // `src/build_info.rs`.
    //
    // Deliberately no `rerun-if-changed` directives anywhere in this build
    // script: cargo's default with none present is to always rerun it, which is
    // what both this and the assets-dir env var above need — the assets-dir
    // value is cheap to recompute and the git revision must actually track the
    // checked-out commit, matching `vcs_tag()`'s own always-regenerate
    // behavior. (An earlier version of this file restricted reruns to
    // `build.rs` changes, which was fine when the assets-dir env var was the
    // only output but would have left the git revision stale across ordinary
    // commits/checkouts once this second purpose was added.)
    let git_revision = git_describe().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=NOCTALIA_GIT_REVISION={git_revision}");
}

fn git_describe() -> Option<String> {
    let output = std::process::Command::new("git")
        .args([
            "describe",
            "--tags",
            "--always",
            "--dirty=-dirty",
            "--abbrev=12",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let revision = String::from_utf8(output.stdout).ok()?;
    let revision = revision.trim();
    (!revision.is_empty()).then(|| revision.to_string())
}
