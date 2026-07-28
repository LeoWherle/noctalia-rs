# Dev shell for the Rust migration (see MIGRATION_PLAN.md, CLAUDE.md).
# Provides the pinned toolchain plus every C library the FFI layer touches.
# If a -sys crate fails to find a library, add it HERE — never vendor/download
# in Cargo: build-time network access breaks under the Nix sandbox.
{
  pkgs,
  rust-bin,
}:
let
  toolchain = rust-bin.stable.latest.default.override {
    extensions = [
      "rust-src"
      "rust-analyzer"
    ];
  };
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    toolchain
    just
    pkg-config
    # bindgen-based crates (pipewire-sys, possible wireplumber FFI): provides
    # libclang + the right include flags inside the sandboxed shell.
    rustPlatform.bindgenHook
    # cxx shim for libqalculate (noctalia-qalc-sys) needs a C++ compiler; mkShell's
    # stdenv cc covers it.
    gdb
  ];

  buildInputs = with pkgs; [
    # Wayland + input
    wayland # also provides libwayland-egl; wayland-backend/client_system dlopens this
    wayland-protocols
    libxkbcommon
    # EGL / GLES loaders (drivers come from the host via NixOS libglvnd paths)
    libglvnd
    # Text & 2D (gtk-rs FFI bindings: cairo-rs, pango, pangocairo)
    cairo
    pango
    harfbuzz
    freetype
    fontconfig
    glib # transitive requirement of cairo/pango pkg-config
    # Buses & services
    dbus # zbus is pure Rust; kept for dbus-run-session in integration tests
    systemd # libudev/sd-* if needed; logind itself is spoken over zbus
    # Audio
    pipewire
    wireplumber # only if the Phase 8 FFI-fallback decision triggers
    # Auth / privilege
    linux-pam
    polkit # reference/tooling; the agent itself is implemented over zbus
    # Secrets/crypto fallbacks (primary path is oo7 + RustCrypto, pure Rust)
    libsecret
    libsodium
    # Images (primary path is image/jxl-oxide/resvg, pure Rust; kept for parity
    # tooling and in case a decoder gap forces FFI fallback)
    libwebp
    libjxl
    librsvg
    # Calculator (no Rust crate exists — hand-written cxx shim links this)
    libqalculate
    libxml2 # librsvg/libqalculate transitive; caldav itself uses quick-xml
  ];

  env = {
    # If libsodium FFI is ever needed, link the Nix package instead of the
    # vendored source build (libsodium-sys-stable honors this).
    SODIUM_USE_PKG_CONFIG = "1";
  };

  shellHook = ''
    export NOCTALIA_ASSETS_DIR="$PWD/assets"
    echo " Noctalia RUST migration shell | 'just check' = the definition of done"
  '';
}
