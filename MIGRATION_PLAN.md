# Noctalia C++ → Rust Migration Plan

**Read this file and `PROGRESS.log` at the start of every session.** This plan is the
single source of truth for scope, ordering, and done-criteria. `PROGRESS.log` records
where the last session actually stopped.

## Ground rules

- **Scope**: port the entire shell **except the plugin system** — everything under
  `src/scripting/` (Luau host, `plugin_*`) is **out of scope**. Code elsewhere that only
  exists to serve plugins (e.g. plugin panel hosting hooks) gets stubbed behind a
  `// PLUGIN-STUB` marker, not ported.
- The C++ tree (`src/`, `tests/`, meson) stays in place untouched as the reference
  implementation until final cutover. Rust code lives in `crates/`.
- One task = one commit (or a few). A task is *done* only when its listed pass/fail test
  passes **and** `just check` is green. Update the checkbox here + append to
  `PROGRESS.log` in the same commit.
- Port the corresponding C++ test from `tests/` whenever one exists; it defines expected
  behavior. Where no test exists, the task lists what to write.
- Tasks are sized for ~1–2 hours. If one balloons, split it in this file first, commit
  the plan edit, then continue — a session may die at any moment (usage limit), so the
  tree must always be committable in minutes.

## Ordering rationale

Phases are ordered mechanical/testable → hard/stateful: pure data code first (config,
theme, /proc parsers), then IO services (D-Bus, network, audio), then Wayland, then the
renderer, UI toolkit, and shell modules. One deliberate deviation from "renderer dead
last": the renderer *core* (Phase 11) must precede the UI toolkit and shell modules that
draw with it. The genuinely hardest GL code — the shared GL context, cross-surface
texture lifetime, dmabuf/screencopy import, and wallpaper effects — is isolated into the
final phase (16) where it belongs.

## Architecture decisions (read before writing code)

1. **Event loop**: `calloop` on the main thread. The C++ code is a single-threaded
   poll-source design (`*_poll_source.h` everywhere); calloop is a 1:1 fit and is what
   `wayland-client` integrates with natively. Async-only libraries (zbus, reqwest) run
   on a `tokio` runtime in a **sidecar thread**, bridged to the main loop with
   `calloop::channel`. No async in crates that don't need it.
2. **Wayland**: smithay's `wayland-client` with the **`wayland-backend/client_system`
   feature (dlopen)** — the Rust-native backend cannot hand a `wl_display*` to EGL, and
   we need EGL. This is load-bearing; do not "simplify" it away.
3. **Text**: `pangocairo`/`cairo` via the gtk-rs FFI bindings (`pango`, `cairo-rs`),
   same libraries the C++ uses — pixel-parity beats purity for a port. Revisit
   cosmic-text only after cutover.
4. **Errors**: `thiserror` per-crate error enums in libraries, `anyhow` only in the
   `noctalia-shell` binary. No `.unwrap()`/`.expect()` outside tests without a
   justifying comment (clippy enforces this — see workspace lints).
5. **Time**: `jiff` (tz-correct, actively maintained, better DST handling than chrono).
6. **Serialization**: `serde` + `toml` for config reads, `toml_edit` where the C++
   preserves user formatting/comments (config export/migrations), `serde_json` for IPC
   and theme JSON output.

## Crate decisions & FFI fallbacks

| C++ dependency | Rust replacement | Kind |
|---|---|---|
| tomlplusplus | `toml` + `toml_edit` + `serde` | pure Rust |
| nlohmann_json | `serde_json` | pure Rust |
| sdbus-c++, gio (bus) | `zbus` | pure Rust |
| libcurl | `reqwest` (rustls; **never** native-tls/openssl) | pure Rust |
| libxml2 (caldav) | `quick-xml` / `roxmltree` | pure Rust |
| md4c | `pulldown-cmark` | pure Rust |
| fzy | `nucleo-matcher` | pure Rust |
| libsecret + Secret Service | `oo7` | pure Rust |
| libsodium | RustCrypto crates matched per call-site (audit first: task 7.6) | pure Rust |
| libwebp | `image` + `image-webp` | pure Rust |
| libjxl | `jxl-oxide` | pure Rust |
| librsvg | `resvg` (librsvg itself is Rust but has no usable crate API) | pure Rust |
| stb / ico decoding | `image` | pure Rust |
| wayland-client (C) | `wayland-client` + `wayland-protocols{,-wlr,-misc}` + `smithay-client-toolkit` | pure Rust (but `client_system` dlopens libwayland for EGL) |
| libxkbcommon | `xkbcommon` crate | **FFI**, pkg-config |
| EGL | `khronos-egl` (dlopen) + `wayland-egl` | **FFI** |
| GLES2/epoxy | `glow` | pure Rust loader over driver |
| cairo/pango/harfbuzz/freetype/fontconfig | gtk-rs `cairo-rs`, `pango`, `pangocairo` | **FFI**, pkg-config |
| libpipewire | `pipewire` (pipewire-rs, official) | **FFI**, bindgen |
| wireplumber | none mature → talk to PipeWire registry/metadata directly via pipewire-rs; only if a needed WP-only feature appears, FFI `wireplumber-0.5` by hand | **FFI** decision deferred to task 9.1 |
| libpam | `pam` crate | **FFI** |
| polkit-agent-1 | none needed — register the agent over D-Bus with zbus, answer with the `pam` crate | pure Rust + PAM FFI |
| libqalculate | no crate → thin C++ shim via `cxx` in `crates/noctalia-qalc-sys` | **FFI**, hand-written shim |
| glib/gobject | only as transitive of cairo/pango bindings; never used directly | FFI |
| jemalloc | drop (Rust default allocator; revisit only if profiling says so) | — |
| drwav (sounds) | `hound` or `symphonia`; playback through pipewire-rs | pure Rust |

### Nix-sandbox red flags (build-time network / non-pkg-config -sys crates)

- **`openssl-sys`**: must never enter the tree. Every `reqwest`/`tungstenite`-ish dep
  gets `default-features = false, features = ["rustls-tls"]`. Check with
  `cargo tree -i openssl-sys` (part of `just check` would be overkill; run when adding deps).
- **`pipewire-sys` / any bindgen crate**: runs bindgen at build → needs libclang. The
  dev shell provides `rustPlatform.bindgenHook`. Fine offline.
- **`libsodium-sys`** (if RustCrypto replacement fails): default build compiles a
  vendored copy — offline-safe, but prefer `libsodium-sys-stable` +
  `SODIUM_USE_PKG_CONFIG=1` (shell exports it) so we link the Nix package.
- **`curl-sys`**: banned (transitively too) — reqwest covers everything.
- Anything that *downloads* at build time (protoc fetchers, prebuilt `.a` grabbers) is
  banned; there is no known need for any.

## Phases

### Phase 0 — Infrastructure (this file, CLAUDE.md, flake, workspace skeleton)
- [x] 0.1 MIGRATION_PLAN.md, CLAUDE.md, PROGRESS.log, flake devShell (`.#rust`,
  rust-overlay), `.envrc`, `just check`, empty workspace crates. Done: `just check`
  green on the skeleton; committed.

### Phase 1 — Foundations (`crates/noctalia-core`)
- [ ] 1.1 Logging — src: `src/core/log.{cpp,h}` → `core::log` on `tracing` +
  `tracing-subscriber`. Preserve the C++ log format/env filtering. Done: unit test
  captures output and matches C++ format for the same events.
- [ ] 1.2 File utilities — src: `src/core/files/*` → `core::files`. Done: port any
  covering tests from `tests/`; round-trip tests for each helper.
- [ ] 1.3 Atomic file writes — src: `src/config/atomic_file.{cpp,h}` → `core::atomic_file`
  (lives in core, config uses it). Done: test proves write-then-crash leaves either old
  or new content (tempdir + rename semantics), matching C++ behavior incl. permissions
  (see `tests/*permissions_test.cpp` patterns).
- [ ] 1.4 Timers & deferred calls — src: `src/core/timer_manager.{cpp,h}`,
  `src/core/deferred_call.{cpp,h}`, `src/core/frame_rate_limiter.h` → `core::timing` as
  calloop timer sources. Done: calloop-driven test fires ordered timers deterministically.
- [ ] 1.5 Event-loop skeleton — new `core::event_loop`: calloop `EventLoop` wrapper +
  tokio sidecar thread + channel bridge (Architecture decision 1). Done: test sends a
  message from a tokio task to a calloop callback and back.
- [ ] 1.6 Process spawning — src: `src/core/process/*` → `core::process`. Done: spawn/
  reap/env tests ported.
- [ ] 1.7 Misc core — src: `src/core/random.h`, `scoped_timer.h`, `build_info.*`,
  `ui_phase.*`, `src/debug/*` → `core::{random,build_info,ui_phase}`. Done: compiles,
  trivial unit tests, `git_revision` generated via `build.rs` (no network).
- [ ] 1.8 i18n — src: `src/i18n/*` (6 files) → `core::i18n`. Done: port
  `tests/i18n_language_tag_test.cpp` + `tests/i18n_supported_languages_test.cpp`.
- [ ] 1.9 Time/clock formatting — src: `src/time/*` (5 files) → `core::time` on `jiff`.
  Done: format-table test comparing against C++ outputs for fixed instants/locales.

### Phase 2 — Config (`crates/noctalia-config`)
- [ ] 2.1 Config data model — src: `src/config/config_types.{cpp,h}`,
  `color_spec.h`, `config_limits.h` → serde structs in `config::types`. Done:
  deserialize `example.toml` losslessly; defaults match C++ defaults (spot-check table).
- [ ] 2.2 Widget config — src: `src/config/widget_config.{cpp,h}`,
  `widget_setting_value.h` → `config::widget`. Done: port `tests/config_widget_test.cpp`.
- [ ] 2.3 Schema — src: `src/config/schema/*` → `config::schema`. Done: port
  `tests/config_schema_roundtrip_test.cpp` (roundtrip equality).
- [ ] 2.4 Validation — src: `src/config/config_validate.{cpp,h}` → `config::validate`.
  Done: port `tests/config_validate/*` and `config_validate_cli_test.sh` cases as Rust
  tests; identical accept/reject verdicts.
- [ ] 2.5 Merge & overrides — src: `config_merge.{cpp,h}`, `config_overrides.cpp` →
  `config::merge`. Done: ported merge tests; deep-merge semantics identical.
- [ ] 2.6 Migrations — src: `config_migrations.{cpp,h}` → `config::migrations` using
  `toml_edit` (must preserve user comments/format exactly as C++ does — verify against
  C++ behavior first; if C++ rewrites the file, plain `toml` is fine). Done: port
  `tests/config_migration_test.cpp`.
- [ ] 2.7 Export — src: `config_export.{cpp,h}` → `config::export`. Done: byte-compare
  exported output with C++ binary for the same input config.
- [ ] 2.8 State store — src: `state_store.{cpp,h}` → `config::state_store`. Done:
  ported round-trip + permission tests.
- [ ] 2.9 Config service & polling — src: `config_service.{cpp,h}`,
  `config_poll_source.h` → `config::service` (calloop file-watch source). Done: test:
  touch file → reload event with debounce matching C++.
- [ ] 2.10 Path resolution — Done: port `tests/config_path_resolution_test.cpp`
  (XDG dirs, env overrides).

### Phase 3 — Theme engine (`crates/noctalia-theme`)
- [ ] 3.1 Color core — src: `src/theme/color.{cpp,h}`, `contrast.{cpp,h}` →
  `theme::color`. Include the achromatic-HSV interpolation fix (commit 73c7c73c).
  Done: numeric golden tests vs C++ for parse/blend/contrast.
- [ ] 3.2 Palettes — src: `builtin_palettes.*`, `fixed_palette.*`, `custom_palettes.*`,
  `community_palettes.*`, `palette.h` → `theme::palette`. Done: builtin table equality
  against C++ dump.
- [ ] 3.3 M3 scheme generation — src: `m3_schemes.cpp`, `scheme.{cpp,h}`,
  `palette_generator.*`, `palette_transform.*` → `theme::scheme`. Done: golden outputs
  for ≥5 seed colors match C++ exactly.
- [ ] 3.4 Image loading (theme) — src: `src/theme/image_loader.*` → `theme::image` on
  `image`/`jxl-oxide`/`resvg`. Done: port `tests/ico_decoder_test.cpp`,
  `image_file_loader_data_uri_test.cpp`, `image_source_log_test.cpp`; decode one sample
  of each format (png/jpg/webp/jxl/svg/ico) from `assets/`.
- [ ] 3.5 Template engine — src: `template_engine.{cpp,h}` → `theme::template`. Done:
  port existing template tests; identical rendered output for builtin templates.
- [ ] 3.6 Template application — src: `template_apply_service.*`,
  `builtin_templates.*`, `community_templates.*`, `custom_schemes.cpp` →
  `theme::apply`. Done: dry-run apply produces identical file set/contents in tempdir.
- [ ] 3.7 App-theme outputs — src: `kde_color_scheme.*`, `firefox_theme/*`,
  `json_output.*` → `theme::outputs`. Done: byte-identical outputs vs C++ for fixtures.
- [ ] 3.8 Theme CLI — src: `src/theme/cli.{cpp,h}` → wired in Phase 4 binary. Done:
  CLI snapshot tests.

### Phase 4 — IPC & CLI (`crates/noctalia-ipc`, first real `noctalia-shell` binary code)
- [ ] 4.1 IPC protocol + server — src: `src/ipc/*` (9 files) → `ipc::{proto,server}`
  (unix socket, serde_json, calloop source). Done: ported IPC tests; C++ client binary
  can talk to the Rust server for one command (manual check noted in PROGRESS.log).
- [ ] 4.2 CLI — src: `src/config/cli.{cpp,h}`, `src/theme/cli.*` → `clap` in the
  binary. Done: `--help`/subcommand snapshot matches documented C++ surface;
  `config validate` + theme subcommands work end-to-end.
- [ ] 4.3 Hooks — src: `src/hooks/*` (4 files) → `shell::hooks`. Done: port
  `tests/hook_manager_test.cpp`, `tests/battery_hook_state_test.cpp`.

### Phase 5 — System monitors (`crates/noctalia-system`)
- [ ] 5.1 CPU stat + temp — src: `cpu_stat.*`, `cpu_temp_sensor.*` → `system::cpu`.
  Done: port `tests/cpu_stat_test.cpp`, `cpu_temp_sensor_test.cpp` (fixture /proc data).
- [ ] 5.2 Memory/disk/net counters — src: rest of `src/system` stat readers incl. disk
  mounts → `system::{mem,disk,net}`. Done: port `tests/disk_mounts_test.cpp` + fixtures.
- [ ] 5.3 Brightness — src: `brightness_service.*`, `brightness_poll_source.h` →
  `system::brightness` (sysfs + logind SetBrightness via Phase 6 when available; sysfs
  first). Done: fixture-driven tests for device enumeration and value mapping.
- [ ] 5.4 Battery warning logic — src: `battery_warning_monitor.*` → `system::battery`.
  Done: state-machine test ported (thresholds, hysteresis).
- [ ] 5.5 App identity + desktop entries — src: `app_identity.*` and desktop-entry code
  in `src/system` (see `tests/desktop_entry_launch_test.cpp`) → `system::apps` using
  `freedesktop-desktop-entry` crate (pure Rust) if it matches semantics, else hand-port.
  Done: port `tests/app_identity_test.cpp`, `desktop_entry_launch_test.cpp`,
  `icon_resolver_test.cpp` (icon resolver may live here or ui — follow C++ placement).
- [ ] 5.6 Remaining `src/system` services (audit dir, list them in PROGRESS.log, split
  if >2h) → `system::*`. Done: each has at least a smoke test; ported tests green.

### Phase 6 — D-Bus (`crates/noctalia-dbus`, zbus)
- [ ] 6.1 Bus plumbing — src: `session_bus.*`, `system_bus.*`, `*_poll_source.h` →
  `dbus::bus`: zbus on the tokio sidecar, events bridged to calloop (uses 1.5). Done:
  integration test round-trips a call against a mock `zbus` server.
- [ ] 6.2 UPower — src: `src/dbus/upower/*` → `dbus::upower`. Done: mock-server test
  covering device add/remove/percentage/state.
- [ ] 6.3 logind — src: `src/dbus/logind/*` + `src/dbus/session/*` if present →
  `dbus::logind` (lock/unlock signals, inhibitors, session control). Done: mock tests;
  manual `loginctl lock-session` check logged.
- [ ] 6.4 MPRIS — src: `src/dbus/mpris/*` → `dbus::mpris` (player discovery, metadata,
  control, seek). Done: mock player test; manual check against an actual player.
- [ ] 6.5 NetworkManager — src: `src/dbus/network/*` → `dbus::network`. Done: mock
  tests for device/AP/connection state mapping.
- [ ] 6.6 Bluetooth (BlueZ) — src: `src/dbus/bluetooth/*` → `dbus::bluetooth`. Done:
  mock tests for adapter/device/pairing state mapping.
- [ ] 6.7 Notification daemon — src: `src/dbus/notification/*` →
  `dbus::notifications`: serve `org.freedesktop.Notifications`. Done: `notify-send`
  reaches a test sink; capabilities/actions/hints match C++ daemon's.
- [ ] 6.8 StatusNotifier tray — src: `src/dbus/tray/*` → `dbus::tray` (StatusNotifierWatcher
  + Host + Item proxies, dbusmenu). Done: mock SNI item test; manual check with a real
  tray app logged.
- [ ] 6.9 Accounts + power profiles + idle inhibit — src: `src/dbus/accounts/*`,
  `src/dbus/power/*`, `src/dbus/idle/*` → `dbus::{accounts,power,idle}`. Done: mock
  tests each.
- [ ] 6.10 Polkit agent (protocol only) — src: `src/dbus/polkit/*` → `dbus::polkit`:
  agent registration + BeginAuthentication plumbing; PAM answering lands in 13.x UI
  phase. Done: agent registers against a mock authority; session objects tracked.

### Phase 7 — Networking, calendar, secrets (`crates/noctalia-net`, `crates/noctalia-calendar`)
- [ ] 7.1 HTTP layer — src: `src/net/*` (7 files) → `net::http` on reqwest(rustls) in
  the tokio sidecar. Done: wiremock-based tests for retry/timeout/etag behavior ported
  from C++ semantics.
- [ ] 7.2 iCal parsing + recurrence — src: `src/calendar/` parsing code →
  `calendar::ical` (hand-port on `rrule` crate only if its results match; the C++
  recurrence tests decide). Done: port `tests/ical_recurrence_test.cpp` — every case.
- [ ] 7.3 CalDAV — src: `src/calendar` caldav discovery/sync → `calendar::caldav` with
  `quick-xml`. Done: port `tests/calendar_discovery_state_test.cpp`; wiremock fixtures.
- [ ] 7.4 Google Calendar — src: google client code → `calendar::google`. Done: port
  `tests/google_client_calendar_list_test.cpp`.
- [ ] 7.5 Calendar cache + credentials — src: cache/credential store →
  `calendar::store` (secrets via `oo7`). Done: port
  `tests/calendar_cache_permissions_test.cpp`, `calendar_credential_store_test.cpp`.
- [ ] 7.6 Sodium audit — grep all libsodium call sites (`src/security/*`, clipboard?),
  document each primitive in this file, map to RustCrypto crates, port
  `src/security/*` (8 files) → `noctalia-core::crypto` or a `security` module. Done:
  interop test — Rust decrypts what C++ encrypted (fixtures) where data persists.

### Phase 8 — Audio (`crates/noctalia-audio`)
- [ ] 8.1 PipeWire connection — src: `src/pipewire/` core (14 files) → `audio::pw` on
  pipewire-rs; registry, default sink/source tracking via metadata (replaces
  wireplumber — decision point: if default-node/route logic turns out to need
  WirePlumber APIs, stop and write the FFI decision into this file first). Done:
  integration test against a real user session (documented manual steps) + unit tests
  for state mapping; `tests/audio_glyphs_test.cpp` ported.
- [ ] 8.2 Volume/mute control + per-node streams — → `audio::control`. Done: manual
  matrix vs C++ (volume up/down/mute on sink/source/stream) logged in PROGRESS.log.
- [ ] 8.3 Sound playback (notification sounds) — drwav usage → `audio::playback` with
  `hound` + pipewire stream. Done: plays a wav fixture; format conversion test.
- [ ] 8.4 VU/peak monitoring if present in C++ (audit `src/pipewire`) → `audio::meter`.
  Done: captures levels from a test stream.

### Phase 9 — Compositor backends (`crates/noctalia-compositors`)
- [ ] 9.1 Backend traits + detection — src: `compositor_detect.*`,
  `compositor_platform.*`, `compositor_runtime.*`, `workspace_backend.h`,
  `output_backend.h`, `keyboard_backend.h` → `compositors::api`. Done: detection unit
  tests with env fixtures ($HYPRLAND_INSTANCE_SIGNATURE, $NIRI_SOCKET, $SWAYSOCK…).
- [ ] 9.2 Hyprland — src: `src/compositors/hyprland/*` + `src/wayland/hyprland/*` →
  `compositors::hyprland` (IPC socket, hand-rolled; don't take on a hyprland crate
  dep). Done: recorded-IPC replay tests; manual check on Hyprland logged.
- [ ] 9.3 Niri — src: `src/compositors/niri/*` → `compositors::niri` (JSON IPC via
  serde). Done: replay tests from captured niri IPC.
- [ ] 9.4 Sway — src: `src/compositors/sway/*` → `compositors::sway` (i3 IPC framing).
  Done: replay tests.
- [ ] 9.5 ext-workspace protocol backend — src: `src/compositors/ext_workspace/*` (+
  `src/wayland/wayland_workspaces.*`) → depends on Phase 10 connection; implement
  against `wayland-protocols` ext-workspace-v1. Done: smithay-based mock-compositor
  test or replay; manual on labwc.
- [ ] 9.6 Remaining backends — `dwl`, `labwc`, `mango`, `kde`, `triad` →
  `compositors::*`. Done: replay tests each (one task per backend if any exceeds 2h).
- [ ] 9.7 Workspace alert service — src: `workspace_alert_service.*` →
  `compositors::alerts`. Done: unit tests on synthetic workspace events.

### Phase 10 — Wayland core (`crates/noctalia-wayland`)
- [ ] 10.1 Connection + registry + outputs — src: `wayland_connection.*`,
  `output_probe.*` → `wl::conn` (wayland-client `client_system`, calloop-integrated,
  fractional-scale + output metadata). Done: connects under a nested compositor
  (headless sway/labwc in CI-ish script `tools/`), enumerates outputs.
- [ ] 10.2 Seat + input — src: `wayland_seat.*`, `src/core/input/*`,
  `keyboard_layout_poll_source.h`, `key_repeat_poll_source.h` → `wl::seat` (pointer,
  keyboard w/ xkbcommon, touch, repeat timers on calloop). Done: nested-compositor
  test drives synthetic input (wtype/virtual pointer) and asserts events.
- [ ] 10.3 Layer surfaces — src: `layer_surface.{cpp,h}` → `wl::layer` (zwlr-layer-shell,
  anchors/margins/exclusive zones, fractional scale + viewport). Done: surface appears
  with correct geometry under nested compositor (screenshot compare via grim).
- [ ] 10.4 Popups & subsurfaces — src: `popup_surface.*`, `subsurface.*`,
  `toplevel_surface.*`, `surface.*` → `wl::surface`. Done: xdg-popup positions match
  C++ under nested compositor for the same anchors.
- [ ] 10.5 Clipboard — src: `clipboard_service.*`, `clipboard_poll_source.h`,
  `src/core/text_clipboard.h` → `wl::clipboard` (data-control protocol). Done: port
  `tests/clipboard_service_test.cpp`, `clipboard_storage_permissions_test.cpp`;
  interop with `wl-copy`/`wl-paste`.
- [ ] 10.6 Foreign toplevels — src: `ext_foreign_toplevels.*`, `wayland_toplevels.*` →
  `wl::toplevels`. Done: nested-compositor test lists/activates windows.
- [ ] 10.7 Text input + virtual keyboard — src: `text_input_service.*`,
  `virtual_keyboard_service.*` → `wl::text_input`. Done: manual IME/on-screen-keyboard
  check logged; protocol unit tests for state machine.

### Phase 11 — Renderer core (`crates/noctalia-render`)
- [ ] 11.1 EGL bootstrap — src: `src/render/backend/*`, `render_context.{cpp,h}` →
  `render::egl` (khronos-egl + wayland-egl window, GLES2 context, no shared context
  yet). Done: clears a layer surface to a color under nested compositor.
- [ ] 11.2 GL abstractions — src: `src/render/core/*` → `render::gl` on `glow`
  (buffers, textures, framebuffers, state cache). Done: offscreen (surfaceless EGL)
  unit tests render triangles to FBO and readback-assert pixels.
- [ ] 11.3 Shader programs — src: `src/render/programs/*` → `render::programs` (port
  GLSL verbatim; keep sources as `.glsl` includes). Done: each program links on GLES2
  in offscreen tests; rect/rounded-rect/shadow renders match C++ readback goldens.
- [ ] 11.4 Render targets & damage — src: `render_target.{cpp,h}` → `render::target`
  (damage tracking, partial redraw, buffer-age). Done: offscreen damage tests; visual
  correctness under nested compositor with forced partial damage.
- [ ] 11.5 Scene graph — src: `src/render/scene/*` → `render::scene`. Done: scene-diff
  unit tests ported; golden-image tests for representative scenes.
- [ ] 11.6 Animation — src: `src/render/animation/*` → `render::animation`. Done: port
  `tests/animation_manager_test.cpp` (curves, timing, cancellation).
- [ ] 11.7 Text rendering — src: `src/render/text/*` → `render::text` (pangocairo →
  GL texture upload path, glyph/run caching mirroring C++). Done: golden-image tests
  for LTR/RTL/emoji/fallback strings at 1x and fractional scale.

### Phase 12 — UI toolkit (`crates/noctalia-ui`)
- [ ] 12.1 Widget tree + layout — src: `src/ui/` core (audit; list files in
  PROGRESS.log) → `ui::{widget,layout}`. Done: layout unit tests ported/golden trees.
- [ ] 12.2 Input dispatch & focus — → `ui::input`. Done: synthetic-event unit tests
  (hover, click, scroll, keyboard focus traversal).
- [ ] 12.3 Icon resolver + image widgets — (with 5.5) → `ui::icons`. Done:
  `tests/icon_resolver_test.cpp` ported; themes resolve identically.
- [ ] 12.4 Markdown — md4c usage → `ui::markdown` on pulldown-cmark. Done: rendered
  AST tests for the markdown corpus the C++ supports.
- [ ] 12.5–12.x Widget set — port `src/ui/widgets` in batches of 3–6 related widgets
  per task (buttons/toggles; sliders; lists/scroll; text inputs w/ 10.7; menus;
  graphs — reuse `capsule_group_reconcile_test.cpp` etc.). Done per batch: golden-image
  render + input-behavior tests. **Before starting, enumerate the batches here.**

### Phase 13 — Shell services & first modules (`crates/noctalia-shell`)
- [ ] 13.1 Surface/panel infrastructure — src: `src/shell/surface/*`, `src/shell/panel/*`,
  `screen_position.h` → `shell::panel`. Done: empty panel on all outputs, correct
  exclusive zones, hotplug survives (manual matrix logged).
- [ ] 13.2 Bar core — src: `src/shell/bar/*` layout/capsules → `shell::bar`. Done:
  bar renders configured layout; `capsule_group_reconcile_test` ported.
- [ ] 13.3–13.x Bar widgets — batches (clock, workspaces, sysmon incl. HSV-blend
  feature from #3672, battery incl. glyph fix 9e406c6c, audio, network, tray…), one
  task per 2–4 widgets. Done per batch: side-by-side visual parity with C++ bar +
  interaction checks. **Enumerate batches before starting.**
- [ ] 13.4 Tooltip + OSD — src: `src/shell/tooltip/*`, `src/shell/osd/*`. Done: visual
  parity; OSD triggers on volume/brightness.
- [ ] 13.5 Notification popups — src: `src/shell/notification/*` (+6.7). Done:
  notify-send end-to-end with actions, timeouts, do-not-disturb.

### Phase 14 — Shell modules, the long tail (`crates/noctalia-shell`)
Each is one or more tasks; split on first contact and record in this file:
- [ ] 14.1 Launcher — src: `src/shell/launcher/*`, `src/launcher/*` (27 files),
  fzy→nucleo. Done: `tests/dock_pinned_apps_test.cpp`-adjacent tests ported; fuzzy
  ranking spot-matches C++ for a fixture corpus (note: nucleo ≠ fzy scores; assert
  top-3 stability, not exact scores). Includes qalculate integration via
  `noctalia-qalc-sys` shim.
- [ ] 14.2 Control center — src: `src/shell/control_center/*` (incl. dynamic-span
  shortcuts, #3676). Done: visual parity + toggle actions work.
- [ ] 14.3 Dock — src: `src/shell/dock/*`. Done: pinned-apps tests ported.
- [ ] 14.4 Clipboard manager UI — src: `src/shell/clipboard/*`. Done: history
  add/search/paste e2e under nested compositor.
- [ ] 14.5 Wallpaper (static paths) — src: `src/shell/wallpaper/*` minus GL effects →
  still-image wallpaper per output. Done: correct scaling modes vs C++ screenshots.
- [ ] 14.6 Settings UI — src: `src/shell/settings/*`. Done: every page opens; edits
  round-trip through Phase 2 config.
- [ ] 14.7 Setup wizard — src: `src/shell/setup_wizard/*`. Done: fresh-config flow e2e.
- [ ] 14.8 Calendar UI — src: `src/calendar` UI + `src/shell` calendar pieces. Done:
  visual parity, event list matches fixtures.
- [ ] 14.9 Switcher, overview, desktop, backdrop, hot/screen corners, profile —
  src: respective `src/shell/*` dirs. Done: per-module manual matrix + any ported tests.
- [ ] 14.10 Idle — src: `src/idle/*` (+`src/dbus/idle`). Done: idle timeout fires
  under nested compositor (ext-idle-notify), inhibitors respected.

### Phase 15 — Security-critical surfaces (`crates/noctalia-shell`)
Do these late and carefully; they can lock you out of your session.
- [ ] 15.1 PAM auth core — src: `src/auth/*` (4 files) → `shell::auth` on `pam` crate,
  isolated authentication worker process like C++ (verify C++ design first). Done:
  PAM conversation test against a test service file; wrong/right password paths.
- [ ] 15.2 Lockscreen — src: `src/shell/lockscreen/*` (ext-session-lock protocol).
  Done: locks/unlocks under **nested** compositor; never test on your live session
  until nested passes 20/20 scripted attempts.
- [ ] 15.3 Greeter — src: `src/shell/greeter/*`. Done: greetd protocol e2e in a VM or
  nested seat; documented manual run.
- [ ] 15.4 Polkit agent UI — src: `src/shell/polkit/*` (+6.10). Done: `pkexec true`
  prompts and authenticates.
- [ ] 15.5 Session menu — src: `src/shell/session/*` (logout/reboot/suspend via 6.3).
  Done: actions fire correct logind calls (mock-verified) + manual check.

### Phase 16 — The hard GL: shared contexts, capture, effects, cutover
- [ ] 16.1 Shared GL context & texture lifetime — src: `gl_shared_context.{cpp,h}` +
  texture-lifetime code in `src/render` → `render::shared`. Cross-surface texture
  sharing, context loss, output hotplug. Done: stress test — repeated output
  add/remove + surface create/destroy under nested compositor, zero GL errors, stable
  RSS over 1000 cycles; run under `RUST_LOG=trace` + apitrace once, diff GL call
  pattern vs C++ for one frame.
- [ ] 16.2 Screencopy capture — src: `src/capture/*` (8 files: screencopy, region
  overlay, screenshot service) → `capture` module (wlr-screencopy, shm + dmabuf
  paths, GL import). Done: screenshot output byte-compare vs grim on nested
  compositor; region overlay interaction manual check.
- [ ] 16.3 Wallpaper renderer & effects — src: `wallpaper_renderer.{cpp,h}` + effect
  programs → `render::wallpaper`. Done: golden-image tests per effect; transition
  animations visually compared frame-by-frame (apitrace/renderdoc capture).
- [ ] 16.4 Overview/screenshot GL paths that import captured buffers — finish anything
  deferred from 14.9/16.2. Done: overview shows live thumbnails, no leaks (16.1 stress
  harness re-run).
- [ ] 16.5 main.cpp parity + cutover — src: `src/app/*` (16 files), `main.cpp` →
  `noctalia-shell::main` (startup order, sd-notify via `sd-notify` crate, signal
  handling, crash guard). Done: full-day daily-drive checklist in PROGRESS.log; meson
  build marked deprecated in README; `nix/package.nix` gains a Rust build.

## Session-restart protocol (for a cold session)

1. Read `CLAUDE.md`, this file, then the **tail of `PROGRESS.log`**.
2. `git log --oneline -10` and `git status` — the tree must be clean; if it isn't, the
   previous session died mid-task: read the diff, finish or revert to a committable
   state *first*.
3. Find the first unchecked task above; cross-check against PROGRESS.log's "next step".
4. `nix develop .#rust -c just check` must pass before you write anything new.
