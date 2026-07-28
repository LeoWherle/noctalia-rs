fn main() {
    // Mirrors meson's `-DNOCTALIA_SOURCE_ASSETS_DIR="<source root>/assets"` (see
    // meson.build). `crates/noctalia-core` is two levels below the repo root.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let source_assets_dir = std::path::Path::new(&manifest_dir).join("../../assets");
    println!(
        "cargo:rustc-env=NOCTALIA_SOURCE_ASSETS_DIR={}",
        source_assets_dir.display()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
