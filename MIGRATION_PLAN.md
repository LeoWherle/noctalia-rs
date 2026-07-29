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
   `wayland-client` integrates with natively. Async-only libraries (zbus) and blocking
   IO (libcurl) run on a `tokio` runtime in a **sidecar thread**, bridged to the main loop with
   `calloop::channel`. No async in crates that don't need it.
2. **Wayland**: smithay's `wayland-client` with the **`wayland-backend/client_system`
   feature (dlopen)** — the Rust-native backend cannot hand a `wl_display*` to EGL, and
   we need EGL. This is load-bearing; do not "simplify" it away.
3. **Text**: `pangocairo`/`cairo` via the gtk-rs FFI bindings (`pango`, `cairo-rs`),
   same libraries the C++ uses — pixel-parity beats purity for a port.
   [future-candidate: cosmic-text (harfrust/swash/fontdb)] — Phase B task B.4;
   fontconfig-matching parity is the known risk there.
4. **Errors**: `thiserror` per-crate error enums in libraries, `anyhow` only in the
   `noctalia-shell` binary. No `.unwrap()`/`.expect()` outside tests without a
   justifying comment (clippy enforces this — see workspace lints).
5. **Time**: `jiff` (tz-correct, actively maintained, better DST handling than chrono).
6. **Serialization**: `serde` + `toml` for config reads, `toml_edit` where the C++
   preserves user formatting/comments (config export/migrations), `serde_json` for IPC
   and theme JSON output.

## Dependency strategy — two-phase, per module

**Phase A (default for everything in Phases 1–16): FFI against the same C library
the C++ code already uses**, matching existing behavior as closely as possible, with
tests proving parity. The goal is a correct, working baseline fast — not the "best"
dependency choice. Use maintained binding crates where they exist (`pango`,
`cairo-rs`, `pipewire`, `pam`, `xkbcommon`, `curl`, `libsecret`, `jpegxl-rs`, …);
write thin bindgen/cxx shims where they don't (md4c, libqalculate). Vendored
`third_party/` C stays vendored (fzy, drwav, wuffs, material-color-utilities):
compile it with the `cc` crate — offline-safe, and behavior stays bit-identical
(e.g. fzy match scores).

Exception — C++-only dependencies with no C ABI cannot be FFI'd; use the canonical
Rust equivalent and prove parity by porting the C++ tests:
- tomlplusplus → `toml`/`toml_edit` + `serde`
- nlohmann_json → `serde_json`
- sdbus-c++ (C++ binding over sd-bus) → `zbus` — the parity target is the
  wire-level D-Bus interface, pinned down by ported tests and mock bus servers
- Luau → dropped entirely (plugin system is out of scope)

**Phase B (tracked, never assumed): measured swaps to pure-Rust crates.** Modules
with a plausible pure-Rust alternative carry a `[future-candidate: crate(s)]` tag on
their Phase-A task and a matching task in Phase 17. Do **not** act on these during
Phase A. Phase B is gated on (a) Phase A working and tested across the whole shell
and (b) real profiling data (CPU/RAM/VRAM) showing where resources actually go
(task B.1).

Before starting Phase B work on any module, do this research first and record it in
the task entry:
1. Read the actual C API calls the module makes today — the specific functions and
   types used, not the library in the abstract.
2. Identify 2–3 real candidate crates and check: current maintenance activity,
   whether their API actually covers the calls found in step 1 (not just "similar
   purpose"), and known gaps for our use case.
3. If the module is performance-relevant, benchmark the candidate against the
   working Phase-A FFI baseline on realistic input — not an isolated microbenchmark.
4. Swap only on a clear measured win in correctness, dependency footprint, or the
   CPU/VRAM/RAM goal.

Deliberately undecided until Phase B (do not guess up front):
- **Allocator** (system vs jemalloc vs mimalloc vs snmalloc): Phase A uses the
  system allocator; task B.2 decides on profiling data. The C++ build's jemalloc
  option is not an argument either way.
- **GLES binding surface** (`glow` vs raw bindgen over GLES2 headers): task 11.2
  picks whichever ports fastest, records it in PROGRESS.log as *provisional*;
  task B.3 decides for real against frame-time data.

Never Phase B — no realistic pure-Rust replacement exists: EGL/GLES (driver-level
API), libpipewire/wireplumber (pipewire-rs is itself a binding), PAM (C ABI plugin
system), libxkbcommon, libqalculate (reimplementing a CAS is scope creep),
libwayland (EGL interop requires the C implementation), wuffs (already memory-safe
by design).

### Nix-sandbox red flags (build-time network / non-pkg-config -sys crates)

- **`openssl-sys`**: must never enter the tree (system libcurl does its own TLS; if
  a future Phase-B HTTP crate needs TLS, it's rustls). Check with
  `cargo tree -i openssl-sys` when adding deps.
- **`pipewire-sys` / any bindgen crate** (incl. our own md4c/qalculate shims): runs
  bindgen at build → needs libclang. The dev shell provides
  `rustPlatform.bindgenHook`. Fine offline.
- **`curl-sys`**: links system libcurl via pkg-config when available (the dev shell
  guarantees it). It has a vendored-source fallback — offline-safe but wrong; verify
  once in build output that it linked the Nix libcurl.
- **`libsodium-sys`**: default build compiles a vendored copy — offline-safe, but
  use `libsodium-sys-stable` + `SODIUM_USE_PKG_CONFIG=1` (shell exports it) so we
  link the Nix package.
- **webp/jxl bindings**: pick pkg-config-linking variants (`libwebp-sys2`,
  `jpegxl-rs` with its pkg-config feature), not vendored-build ones.
- Anything that *downloads* at build time (protoc fetchers, prebuilt `.a` grabbers) is
  banned; there is no known need for any.

## Phases

### Phase 0 — Infrastructure (this file, CLAUDE.md, flake, workspace skeleton)
- [x] 0.1 MIGRATION_PLAN.md, CLAUDE.md, PROGRESS.log, flake devShell (`.#rust`,
  rust-overlay), `.envrc`, `just check`, empty workspace crates. Done: `just check`
  green on the skeleton; committed.

### Phase 1 — Foundations (`crates/noctalia-core`)
- [x] 1.1 Logging — src: `src/core/log.{cpp,h}` → `core::log` on `tracing` +
  `tracing-subscriber`. Preserve the C++ log format/env filtering. Done: unit test
  captures output and matches C++ format for the same events.
- [x] 1.2 File utilities — src: `src/core/files/*` → `core::files`. Done: port any
  covering tests from `tests/`; round-trip tests for each helper.
- [x] 1.3 Atomic file writes — src: `src/config/atomic_file.{cpp,h}` → `core::atomic_file`
  (lives in core, config uses it). Done: test proves write-then-crash leaves either old
  or new content (tempdir + rename semantics), matching C++ behavior incl. permissions
  (see `tests/*permissions_test.cpp` patterns).
- [x] 1.4 Timers & deferred calls — src: `src/core/timer_manager.{cpp,h}`,
  `src/core/deferred_call.{cpp,h}`, `src/core/frame_rate_limiter.h` → `core::timing` as
  calloop timer sources. Done: calloop-driven test fires ordered timers deterministically.
- [x] 1.5 Event-loop skeleton — new `core::event_loop`: calloop `EventLoop` wrapper +
  tokio sidecar thread + channel bridge (Architecture decision 1). Done: test sends a
  message from a tokio task to a calloop callback and back.
- [x] 1.6 Process spawning — src: `src/core/process/*` → `core::process`. Split (session
  hitting `src/core/process/process.cpp` at 968 lines + `process_fds.cpp`, ~30 public
  functions across genuinely distinct concerns, vs. `tests/process_test.cpp` at 249
  lines): see 1.6.1-1.6.6 below, ordered core-first since later ones depend on the
  fork/exec/pipe machinery 1.6.1 builds.
  - [x] 1.6.1 Core sync process execution — `RunResult`/`RunOptions`/`RunCallbacks`
    types, `runSyncProcess` (fork/exec/pipe capture/poll loop/timeout/cancellation/
    output-byte-limit truncation — the hardest single piece: `terminateAndWait`'s
    process-group signaling, `pollTimeoutMs`'s deadline-clamped poll wait,
    `drainAvailable`'s non-blocking read+truncate+callback), `runSync(args[, options])`
    overloads, env overrides (`applyEnvOverrides`/`EnvOverride`). Done: port
    `syncAppliesEnvOverrides`; new tests for timeout-triggers-terminateAndWait and
    output-byte-limit truncation (both flagged `outTruncated`/`errTruncated` and the
    interaction with a live callback), since the C++ test only exercises these via the
    async path (1.6.2).
  - [x] 1.6.2 Async execution — worker-thread `runAsync(args, callbacks, options)` /
    `runAsync(command, callbacks, options)` wrapping 1.6.1's `runSyncProcess`, plus
    `runAsync(command)`/`runSync(command)` shell-string composition via `/bin/sh -lc`.
    Done: port `capturedAsyncDeliversCallbacksAndResult`,
    `capturedAsyncDeliversCompletionOnly`, `stringCommandsSupportShellComposition`, the
    empty-callback-set-should-not-launch case.
  - [x] 1.6.3 Detached spawning — `doubleForkExecDetached` (double-fork + setsid so the
    grandchild isn't a direct child; activation-token/working-dir env for the
    grandchild), `runAsync(args, activationToken, workingDir)`, `launchDetachedTracked`/
    `terminateTracked`, `launchFirstAvailable`, `commandExists`/`resolvePrivilegeEscalator`
    (PATH search). Done: port `detachedAsyncInheritsLaunchEnvironment`,
    `commandExistsRejectsDirectories`.
  - [x] 1.6.4 Process listing & matching — `/proc` command-line scanning with the
    250ms TTL cache (`cachedProcessCommandLines`/`readProcessCommandLines`),
    `commandLineMatchesAll`, `desktopPortalAvailable`, `flatpakAppInstalled` (XDG data
    root enumeration). Done: fixture-driven tests (spawn known marker processes or use
    this test binary's own `/proc/self`; a temp flatpak data root for
    `flatpakAppInstalled`) — no direct C++ test exists for these, "round-trip tests for
    each helper" is the bar.
  - [x] 1.6.5 systemd user-manager integration — `cgroupIndicatesSystemdUserManager`,
    `runningUnderSystemdUserManager`, `escapeSystemdUnitName`, `startSystemdService`,
    `runAsyncAsSystemdService`. Done: port `cgroupDetectsSystemdUserManager` (pure
    string-matching, no live systemd needed); `runAsyncAsSystemdService`/
    `startSystemdService` end-to-end needs a live `systemd --user` + `systemd-run` —
    manual check logged in PROGRESS.log, matching the pattern used elsewhere in this
    plan for live-service-dependent behavior (e.g. task 6.3's `loginctl lock-session`).
  - [x] 1.6.6 Process FD diagnostics — src: `src/core/process/process_fds.{cpp,h}` →
    `process::fds` (or a submodule of `core::process`): `raiseOpenFileLimit`,
    `describeOpenFileDescriptors`. Done: no C++ test exists; unit tests for the
    fd-target bucketing (`socket:`/`pipe:`/`memfd:`/long-path truncation) and a smoke
    test that `raiseOpenFileLimit` doesn't lower the soft limit and
    `describeOpenFileDescriptors` output is well-formed.
- [x] 1.7 Misc core — src: `src/core/random.h`, `scoped_timer.h`, `build_info.*`,
  `ui_phase.*` → `core::{random,profiling,build_info,ui_phase}`. Done: compiles,
  trivial unit tests, `git_revision` generated via `build.rs` (no network).
  `src/debug/*` (`DebugService`, the `dev.noctalia.Debug` D-Bus service) split out
  to 6.11 — it depends on D-Bus bus plumbing and a notification manager, neither
  of which exist yet this early in the migration.
- [x] 1.8 i18n — src: `src/i18n/*` (6 files) → `core::i18n`. Done: port
  `tests/i18n_language_tag_test.cpp` + `tests/i18n_supported_languages_test.cpp`.
- [x] 1.9 Time/clock formatting — src: `src/time/*` (5 files) → `core::time` on `jiff`.
  Done: format-table test comparing against C++ outputs for fixed instants/locales.

### Phase 2 — Config (`crates/noctalia-config`)
- [ ] 2.1 Config data model — src: `src/config/config_types.{cpp,h}`,
  `color_spec.h`, `config_limits.h` → serde structs in `config::types`. Split
  (session: `config_types.h` is 1631 lines + 579 in the `.cpp`, ~50 nested
  struct types across ~30 independent top-level `example.toml` sections —
  comparable to task 1.6's `process.cpp` split, same reasoning): see
  2.1.1-2.1.9 below, ordered so each subtask's structs have no forward
  dependency on a later one. Every subtask's done bar is deserializing its
  slice of `example.toml` losslessly + a defaults spot-check; only 2.1.9 (the
  root `Config` struct) exercises the whole file, since only then does every
  field exist.
  - [x] 2.1.1 Color primitives — src: `render/core/color.h`'s `Color`,
    `ui/palette.h`'s `ColorRole`/`ColorRoleToken`/`ColorSpec` (**not**
    `Palette`/scheme generation — that's task 3.1's full `theme::color`),
    `config/color_spec.{h,cpp}` (`colorSpecFromConfigString`/
    `colorSpecToConfigString`, actually implemented in `config_types.cpp`
    despite being declared in `color_spec.h`), `config_limits.h`'s 4
    clipboard-history constants. Placed in `noctalia-core` (new
    `core::color`), not `noctalia-config` or `noctalia-theme`: both Phase 2
    (this task) and Phase 3 (task 3.1) need the exact same small POD
    (`role: Option<ColorRole>, fixed: Color, alpha: f32`) as their
    foundation, and `noctalia-core` is the one crate both already depend on
    — avoids a config↔theme crate dependency either direction. Done: parse/
    serialize round-trip test for every `ColorRole` token + a handful of hex
    strings from `example.toml`.
  - [x] 2.1.2 Bar & widget settings — `BarCapsuleGroupStyle`/
    `BarDeadZoneOverride`/`BarMonitorOverride`/`BarDeadZoneConfig`/
    `BarConfig`, `WidgetBarCapsuleSpec`, `WidgetConfig` (the settings-map
    struct + its `findSetting`/`getString`/`getInt`/`getDouble`/`getBool`/
    `getColorSpec`/`getOptionalColorSpec`/`getStringList`/`getStringMap`/
    `hasSetting` accessors), the capsule-group reconciliation helpers
    (`findBarCapsuleGroupStyle`/`capsuleSpecFromGroup`/
    `capsuleGroupRefsForBarScope`/`capsuleGroupRefsForMonitorScope`/
    `reconcileCapsuleGroups`/`isCapsuleGroupToken`/`capsuleGroupTokenId`/
    `makeCapsuleGroupToken`/`resolveWidgetContentScale`/
    `resolveWidgetBarCapsuleSpec`), `outputMatchesSelector`. Depends on
    2.1.1 (`ColorSpec`) and on task 2.2's `WidgetSettingValue`
    (`widget_setting_value.h` — `WidgetConfig::settings` is keyed by it and
    won't compile without it): do 2.2 first, or pull just
    `WidgetSettingValue` forward into this subtask if 2.2 hasn't landed
    yet — check PROGRESS.log/plan state before starting. Done: deserialize
    `[bar.main]`/`[dock]`'s widget-lane entries from `example.toml`;
    capsule-group reconciliation gets its own unit tests (no C++ test
    exists — verified via grep).
  - [x] 2.1.3 Shell config — `ShellConfig` + nested `AnimationConfig`/
    `ShadowConfig`/`PanelConfig`/`LauncherConfig`/`ScreenCornersConfig`/
    `MprisConfig`/`ScreenshotConfig`/`PrivacyConfig`, `ShellSessionConfig`
    + `ShellSessionPowerConfig`, `ShellGreeterSyncConfig`,
    `ShortcutConfig`, `SessionPanelActionConfig`, `DmenuEntryConfig`,
    `LauncherProviderConfig`, `defaultSessionPanelActions`,
    `defaultControlCenterShortcuts`. Done: deserialize `[shell]` and its
    `[shell.*]` subtables from `example.toml`.
    Note (session 21): discovered `SessionPanelActionConfig::shortcut` needs
    `KeyChord` (`src/core/input/key_chord.h`), owned wholesale by task 10.2
    (`wl::seat`, needs `xkbcommon` FFI for `parseKeyChordSpec`/
    `keyChordToString`) — too heavy to pull forward whole mid config-data-model
    task. Pulled forward only the `KeyChord` POD (`sym`/`modifiers`, no parsing
    logic) into new `noctalia-core::input` (same "small shared POD" reasoning
    as 2.1.1's `ColorSpec`); `shortcut` itself is `#[serde(skip)]` until 10.2's
    string<->KeyChord bridge lands. Task 2.1.6 (`KeybindsConfig`) can reuse
    `noctalia_core::input::KeyChord` directly. Also: every enum-like C++ field
    here (`PasswordMaskStyle`/`ClipboardAutoPasteMode`/`ShadowDirection`/
    `PanelTransparencyMode`/`PanelPlacement`/`SessionActionButtonVariant`) is a
    plain `String` holding the `config_schema.cpp` `enumField` key, not a real
    Rust enum — same "string-vs-enum validation is schema-engine territory"
    call 2.1.2 made for `BarConfig::layer`/`position`.
  - [x] 2.1.4 Wallpaper, backdrop, lockscreen, dock — `WallpaperMonitorOverride`/
    `WallpaperAutomationConfig`/`WallpaperConfig`/`WallpaperFillMode`/
    `WallpaperTransition`/`WallpaperFavorite`, `BackdropConfig`,
    `LockscreenConfig`, `DockConfig`. Done: deserialize `[wallpaper]`,
    `[backdrop]`, `[lockscreen]`, `[dock]`.
  - [x] 2.1.5 Desktop widgets, OSD, notifications — `DesktopWidgetsGridState`/
    `DesktopWidgetState`/`DesktopWidgetsConfig`, `LockscreenWidgetsConfig`,
    `OsdKindsConfig`/`OsdConfig`, `NotificationConfig`/
    `NotificationFilterConfig`, `ShadowDirectionOffset`. Done: deserialize
    `[desktop_widgets]`, `[osd]`/`[osd.kinds]`, `[notification]`.
  - [x] 2.1.6 Idle, keybinds, hotcorners, accessibility — `IdleBehaviorConfig`/
    `IdleConfig`/`IdleActionRequest`/`ResolvedIdleBehavior`/
    `defaultIdleBehaviors`/`commandIdleAction`/`idleAction`,
    `KeybindsConfig`/`defaultKeybindSet`, `HotCornersConfig` + `Corner`,
    `AccessibilityConfig`. Done: deserialize `[idle.behavior.*]`,
    `[keybinds]`, `[accessibility]`; `defaultKeybindSet` spot-checked
    against a few real `KeybindAction` values.
    Note (session 21): `KeyChord` (the POD only — `sym`/`modifiers`, no
    string parsing) already landed as `noctalia_core::input::KeyChord`,
    pulled forward by task 2.1.3. `KeybindsConfig`'s `Vec<KeyChord>` fields
    can use it directly; the TOML string<->`KeyChord` bridge itself is still
    task 10.2's job (needs `xkbcommon` FFI), so plan for the same
    `#[serde(skip)]` treatment 2.1.3 gave `SessionPanelActionConfig::shortcut`.
  - [x] 2.1.7 System, audio, brightness, battery, nightlight, location,
    storage — `SystemConfig` + `MonitorConfig`, `AudioConfig`,
    `BrightnessConfig` + `BrightnessMonitorOverride`, `BatteryConfig` +
    `BatteryDeviceWarningThreshold`, `NightLightConfig`, `LocationConfig`,
    `StorageConfig`. Done: deserialize `[system.monitor]`, `[audio]`,
    `[brightness]`, `[nightlight]`, `[location]`.
  - [x] 2.1.8 Weather, calendar, hooks, control center — `WeatherConfig`,
    `CalendarConfig` + `Account`, `HooksConfig`, `ControlCenterConfig` +
    `CalendarTabConfig`, `hookKindFromKey`/`hookKindKey`. Done: deserialize
    `[weather]`, `[calendar]`, `[control_center.calendar]`, `[hooks]`.
  - [x] 2.1.9 Theme, plugins, root `Config` — `ThemeConfig` + nested
    `TemplateColorConfig`/`TemplateInputPathModesConfig`/
    `TemplateCompareColorConfig`/`UserTemplateConfig`/`TemplatesConfig`,
    `PluginSourceConfig`/`PluginsConfig`/`defaultPluginSources`/
    `isDefaultPluginSourceName`/`isValidPluginSourceName`, the top-level
    `Config` struct and `ConfigChangeSet` (`any()` + the field list —
    `computeConfigChangeSet` itself is `config_overrides.cpp`, task 2.5,
    not here). Done: deserialize `[theme]`/`[theme.templates]`, then the
    **whole** `example.toml` losslessly through the assembled `Config`
    struct — this is where task 2.1's original done-bar actually lands.
- [x] 2.2 Widget config — src: `src/config/widget_config.{cpp,h}`,
  `widget_setting_value.h` → `config::widget`. Done: port `tests/config_widget_test.cpp`.
  Note (session 20): `widget_setting_value.h` itself already landed as part of
  2.1.2 (pulled forward, per this task's own dependency note above) —
  `noctalia-config::types::widget_setting_value` (`WidgetSettingValue`,
  `WidgetSettingValueAs`/`IntoWidgetSettingValue` traits). What's left here is
  `widget_config.cpp`'s three functions (`readWidgetSettingValue`,
  `seedBuiltinWidgets`, `readBarWidgetConfig`) plus porting
  `tests/config_widget_test.cpp`, which also exercises `resolveWidgetBarCapsuleSpec`
  (2.1.2's `noctalia-config::types::bar::resolve_widget_bar_capsule_spec`, already
  available).
- [x] 2.3 Schema — src: `src/config/schema/*` → `config::schema`. Done: port
  `tests/config_schema_roundtrip_test.cpp` (roundtrip equality).
- [ ] 2.4 Validation — src: `src/config/config_validate.{cpp,h}` → `config::validate`.
  Done: port `tests/config_validate/*` and `config_validate_cli_test.sh` cases as Rust
  tests; identical accept/reject verdicts.
  **Split (session 30, discovered `config_validate.cpp` pulls in most of the
  shell's config-consuming surface, most of it un-ported and phases away)**:
  - [x] 2.4.1 Core section/schema validation — the portable-now slice of
    `validateMergedConfig`/`appendMergedConfigDiagnostics`: `checkSection` over
    every registered `schema::sections()` entry (unknown keys +
    `checkAgainstDefaults`), the unknown-top-level-key check
    (`schema::isKnownRootKey`), `validateIncludeShape`, `validateCalendarSyntax`,
    `validateBars` (bar/monitor-override *schema* only —
    `BarConfig`/`BarMonitorOverride` via `barFieldsSchema`/
    `barMonitorOverrideSchema`, already ported in 2.1.2/2.3 — NOT
    `validateBarWidgets`, which needs Phase 13's widget-type registry).
    `validateLocation` needs `day_night_schedule::normalizedClock`
    (`src/system/day_night_schedule.cpp`), a tiny (~15-line) pure HH:MM format
    check with no `system::day_night` scheduling logic attached — pulled forward
    as a private helper (same "minimal piece" pattern as 2.1.3's `KeyChord` POD),
    leaving `GeoCoordinates`/`resolveCoordinates`/`evaluate`/`isManualMode`/
    `hasUsableCustomTimes` for task 5.6 (system services audit) to port for real.
    Done: unit tests against `validate_merged_config`/the individual check
    functions fed a `toml::Table` directly (no CLI, no merge, no migrations yet)
    covering the subset of `tests/config_validate/*` cases these paths reach.
    Fresh-context review (rule 3) first-pass caught `validate_merged_config`
    producing ~30 false-positive "unknown setting" warnings against the real
    repo-root `example.toml`, all traced to incompleteness bugs in already-
    committed task 2.3 schemas (not this task's own logic) — fixed in the same
    session: `shell_animation_schema`/`shell_shadow_schema` missing
    `enabled`/`alpha`; `shell_panel_schema` missing 14 of 22 fields;
    `shell_launcher_schema` missing 5 fields *and* reading `providers` via
    `array_of` instead of `named_map` (real shape is `[shell.launcher.
    providers.<name>]`, a table of named sub-tables — `array_of` only matches a
    TOML array, so provider configs were silently never read at all, found
    independently while fixing the rest); `wallpaper_automation_schema` missing
    `recursive`; `templates_schema` missing `enable_community_templates`/
    `community_ids` and using TOML key `"user_template"` instead of the real
    `"user"`; `control_center_schema` using wrong keys `"sidebar_mode"`/
    `"sidebar_section_mode"`/`"calendar_tab"` instead of `"sidebar"`/
    `"sidebar_section"`/`"calendar"` and missing `width`/`show_shortcut_labels`/
    `hidden_tabs`; `keybinds_schema` entirely empty (real `keybindActionField`
    needs `parseKeyChordSpec`, task 10.2's `xkbcommon` FFI — fixed with 8 no-op
    `custom_field`s that only mark the keys *known*, deferring actual parsing).
    A pre-existing roundtrip test asserting `check_path("control_center.
    sidebar_mode")` was itself wrong (no such string exists in the C++ test
    suite) — fixed to assert the real `"control_center.sidebar"` key. Second
    fresh-context verification pass confirmed all 9 fixes against
    `config_schema.cpp` field-by-field (keys, completeness, ranges,
    `named_map`/`array_of`/`sub_table` shape) and re-ran `just check` clean;
    `whole_example_toml_produces_zero_diagnostics` (strengthened from a
    weaker no-fatal-errors check) is the regression guard. Task 2.3's
    checkbox is not reopened — these were the specific gaps this task's own
    correctness bar exposed, not a full re-audit; other latent gaps in
    untouched sections of `config_schema.rs` may still exist.
  - [ ] 2.4.2 Bar/desktop/lockscreen widget-type validation —
    `validateBarWidgets`/`validateDesktopWidgets`/`validateLockscreenWidgets` (+
    the `invalid-timezone.toml` case, needing `time/time_format.h`'s
    `isValidTimezone`). Blocked on Phase 13's `shell::settings::
    widget_settings_registry`/`shell::desktop::desktop_widget_settings_registry`
    (widget-type → setting-schema tables for every built-in widget, 1595+494
    lines) and Phase 15's lockscreen widget list existing in Rust first. Do this
    alongside/after whichever of those lands.
  - [ ] 2.4.3 Launcher provider validation — `validateLauncherProviders` (the
    `warn-only.toml` provider cases). Blocked on task 14.1 (`launcher::
    kBuiltinProviders` + real provider config). The plugin-provider branch
    (`scripting::isValidPluginId`/enabled-plugins check) is scripting-adjacent —
    port it behind a `// PLUGIN-STUB` marker per the ground-rules scripting
    policy (treat every plugin-tagged provider as "not a plugin" rather than
    pulling in `scripting::isValidPluginId`), not a full plugin-registry
    integration.
  - [ ] 2.4.4 Plugin settings validation — `validatePluginSettings`. Out of
    scope per ground rules (exists only to serve the plugin system) —
    `// PLUGIN-STUB`, do not port; `plugin_settings.*` tables in a validated
    config always pass silently.
  - [ ] 2.4.5 Whole-source validation entry points —
    `validateConfigSources`/`validateConfigFile` (`mergeSources`,
    `formatParseError`, the `syntax-error.toml`/`generated-config`/
    exported-full-config CLI cases). Blocked on task 2.5 (merge,
    `mergeConfigWithIncludes`/`ConfigService::deepMerge`) and 2.6 (migrations,
    `normalizeLegacyConfig`/`storedConfigVersion`/`applyPendingConfigMigrations`)
    — both are real, direct calls in these two functions, not incidental. Do
    after 2.5+2.6 land. The CLI-level `config_validate_cli_test.sh` (needs a real
    `noctalia config validate` binary + `config export full`, task 2.7) is the
    final done-bar for this subtask, not for 2.4 as a whole.
- [x] 2.5 Merge & overrides — src: `config_merge.{cpp,h}`, `config_overrides.cpp` →
  `config::merge`. Done: ported merge tests; deep-merge semantics identical.
  **Split (session 31, `config_overrides.cpp` is 2467 lines, almost all of it
  `ConfigService::*` methods needing a live service — task 2.9, not ported —
  plus a genuinely-portable-now pure comparison-logic slice; `deepMerge` itself
  turns out to live in `config_service.cpp`, not `config_merge.cpp`)**:
  - [x] 2.5.1 Deep merge — `ConfigService::deepMerge` (`config_service.cpp:1353`,
    despite the name living in `config_merge.{cpp,h}`'s task grouping): recursive
    TOML table merge, table-into-table recurses, everything else (including
    arrays) replaces wholesale. Pure, no deps beyond `toml`. Done: unit tests
    (no C++ test exists — grep confirms zero references to `deepMerge` under
    `tests/`) covering nested-table recursion, array wholesale-replace,
    table-over-non-table and non-table-over-table replacement.
  - [x] 2.5.2 Config change-set computation — `computeConfigChangeSet`
    (`config_overrides.cpp:712`) + its equality helpers (`vectorEqual`,
    `widgetSettingEqual`/`widgetSettingsEqual` with int/double coercion,
    `pluginsConfigEqual`, `desktopWidgetEqual`/`desktopWidgetsConfigEqual`/
    `lockscreenWidgetsConfigEqual`, `barBaseConfigEqual`/
    `applyMonitorOverrideForComparison`/`barMonitorOverrideEqual`/
    `barConfigEqual`, `widgetConfigEqual`/`widgetMapEqual`, `configEqual`).
    Pure, operates only on already-ported `Config`/`BarConfig`/
    `BarMonitorOverride`/`WidgetConfig`/`DesktopWidgetsConfig`/
    `LockscreenWidgetsConfig`/`PluginsConfig` — portable now. `configEqual`
    (override-effectiveness equality, distinct from `computeConfigChangeSet`'s
    per-section dirty-flags) is in scope too, same file/helpers. Done: no C++
    test exists (grep confirms) — unit tests proving int/double widget-setting
    coercion, the bar monitor-override resolution + comparison (every override
    field, matching `applyMonitorOverrideForComparison` exactly), and
    `ConfigChangeSet`/`configEqual` round-trips against representative diffs.
  - [x] 2.5.3 Include-aware directory merge — `mergeConfigWithIncludes`
    (`config_merge.cpp`): scans a config dir for sorted `*.toml`, expands each
    file's `[include]` table (files + directories, cycle detection, `autoload`
    opt-out), overlaying via 2.5.1's `deep_merge`. Needs `FileUtils::
    expandEnvVars`/`resolvePath` (`util/file_utils.h`) — pull forward only
    those two functions (same "minimal piece" pattern as 2.1.3's `KeyChord`),
    not the whole header (also has XDG-base-dir expansion, private-permission
    helpers, etc. unrelated to this task — a future task owns porting the rest
    as its callers need it). Done: unit tests against real temp directories (no
    C++ test exists) covering multi-file sorted merge, `[include].files`
    (file + directory forms), cycle detection, `autoload = false` opt-out, and
    an env-var-expanded include path.
  - [ ] 2.5.4 Live override CRUD — the remaining ~1900 lines of
    `config_overrides.cpp`: every `ConfigService::*` method (bar/monitor
    override create/move/rename/delete, `setOverride`/`clearOverride`(s),
    plugin source/enable management, theme mode/scheme setters, dock/
    setup-wizard/desktop-widgets-state setters, override-path effectiveness
    queries). All genuinely need a live `ConfigService` (task 2.9) to exist —
    fold this into task 2.9's own scope rather than porting it standalone
    against nothing to call it on.
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
- [ ] 3.4 Image loading (theme) — src: `src/theme/image_loader.*` → `theme::image`.
  Phase A FFI: libwebp (`libwebp-sys2`), libjxl (`jpegxl-rs`), librsvg (thin bindgen
  if no maintained binding fits), own ico decoder straight-ported (wuffs stays
  vendored via `cc`). [future-candidate: image, image-webp, jxl-oxide, resvg → B.6]
  Done: port `tests/ico_decoder_test.cpp`,
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
  in `src/system` (see `tests/desktop_entry_launch_test.cpp`) → `system::apps`,
  hand-ported to match C++ semantics exactly.
  [future-candidate: freedesktop-desktop-entry → B.14] Done: port `tests/app_identity_test.cpp`, `desktop_entry_launch_test.cpp`,
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
- [ ] 6.11 Debug D-Bus service — src: `src/debug/debug_service.{cpp,h}` →
  `dbus::debug` (or similar): serves `dev.noctalia.Debug`
  (`EmitInternalNotification`/`SetVerboseLogs`/`GetVerboseLogs`). Split out of 1.7
  (see its note) since it needs 6.1's bus plumbing plus a ported notification
  manager — there's no dedicated task for that yet (it folds under 6.7 or
  wherever `NotificationManager` lands; revisit when scheduling this task). Done:
  mock-bus test round-trips all three methods; `SetVerboseLogs` observably
  changes `core::log`'s level.

### Phase 7 — Networking, calendar, secrets (`crates/noctalia-net`, `crates/noctalia-calendar`)
- [ ] 7.1 HTTP layer — src: `src/net/*` (7 files) → `net::http`. Phase A FFI: `curl`
  crate over system libcurl — same engine as the C++, so TLS/proxy/redirect semantics
  carry over for free; runs on the sidecar thread. [future-candidate:
  reqwest(rustls), ureq → B.7] Done: wiremock-based tests for retry/timeout/etag
  behavior ported from C++ semantics.
- [ ] 7.2 iCal parsing + recurrence — src: `src/calendar/` parsing code →
  `calendar::ical`, hand-ported (it's Noctalia's own code, no C dep).
  [future-candidate: rrule crate, if its results match the ported tests → Phase B]
  Done: port `tests/ical_recurrence_test.cpp` — every case.
- [ ] 7.3 CalDAV — src: `src/calendar` caldav discovery/sync → `calendar::caldav`.
  Phase A FFI: libxml2 via the `libxml` crate, mirroring the C++ parsing paths.
  [future-candidate: quick-xml, roxmltree → B.8] Done: port
  `tests/calendar_discovery_state_test.cpp`; wiremock fixtures.
- [ ] 7.4 Google Calendar — src: google client code → `calendar::google`. Done: port
  `tests/google_client_calendar_list_test.cpp`.
- [ ] 7.5 Calendar cache + credentials — src: cache/credential store →
  `calendar::store`. Phase A FFI: libsecret via the gtk-rs `libsecret` bindings,
  same keyring semantics as C++. [future-candidate: secret-service, oo7 → B.9]
  Done: port `tests/calendar_cache_permissions_test.cpp`,
  `calendar_credential_store_test.cpp`.
- [ ] 7.6 Security/crypto — src: `src/security/*` (8 files) → `noctalia-core::crypto`
  or a `security` module. Phase A FFI: libsodium via `libsodium-sys-stable`
  (pkg-config) with thin safe wrappers — byte-identical formats guaranteed. While
  porting, document every primitive/call-site in the task (that audit is the input
  Phase B needs). [future-candidate: RustCrypto per-primitive → B.10] Done:
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
- [ ] 8.3 Sound playback (notification sounds) — drwav usage → `audio::playback`:
  vendored drwav compiled via `cc` + bindgen shim, pipewire stream out.
  [future-candidate: hound, symphonia → B.13] Done: plays a wav fixture; format
  conversion test.
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
  Note (session 21): `key_chord.h`'s `KeyChord` POD (`sym`/`modifiers`) already
  landed as `noctalia_core::input::KeyChord`, pulled forward by task 2.1.3 for
  `SessionPanelActionConfig::shortcut`/2.1.6's `KeybindsConfig`. What's left here
  is `key_chord.cpp`'s real logic — `parseKeyChordSpec`/`keyChordToString`/
  `keyChordDisplayLabel`/`keyChordMatches`/`isPrintableKey`/`isPlainPrintableKey`,
  all `xkbcommon`-FFI-backed — plus wiring a TOML string<->`KeyChord` bridge back
  into the 2.1.x config structs that currently `#[serde(skip)]` it.
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
- [ ] 11.2 GL abstractions — src: `src/render/core/*` → `render::gl` (buffers,
  textures, framebuffers, state cache). Binding surface is **provisional** per the
  dependency strategy: pick `glow` or raw bindgen over GLES2 headers, whichever
  ports fastest, and record the pick + rationale in PROGRESS.log; B.3 decides for
  real on frame-time data. Done: offscreen (surfaceless EGL) unit tests render
  triangles to FBO and readback-assert pixels.
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
- [ ] 12.4 Markdown — md4c usage → `ui::markdown`. Phase A FFI: md4c via a thin
  bindgen shim (small C API, exact parser parity). [future-candidate:
  pulldown-cmark, comrak → B.11] Done: rendered AST tests for the markdown corpus
  the C++ supports.
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
- [ ] 14.1 Launcher — src: `src/shell/launcher/*`, `src/launcher/*` (27 files).
  Fuzzy match: vendored fzy compiled via `cc` — match scores stay bit-identical.
  [future-candidate: nucleo-matcher → B.12] Done: launcher-adjacent tests ported;
  fuzzy ranking exactly matches C++ for a fixture corpus. Includes qalculate
  integration via the `noctalia-qalc-sys` cxx shim (permanent FFI — see strategy).
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
  `noctalia-shell::main` (startup order, sd_notify readiness — a one-datagram
  protocol, hand-implement or `sd-notify` crate, no libsystemd FFI warranted, signal
  handling, crash guard). Done: full-day daily-drive checklist in PROGRESS.log; meson
  build marked deprecated in README; `nix/package.nix` gains a Rust build.

### Phase 17 — Phase B: measured dependency swaps
**Gate: do not start any of these until 16.5 (cutover) is done AND B.1 has produced
numbers.** Every task here follows the 4-step research process in "Dependency
strategy" and records its findings inline before any code changes.
- [ ] B.1 Profiling baseline — repeatable harness (perf + heaptrack for CPU/RSS,
  GPU memory via DRM fdinfo; scripted scenario: cold start, bar idle 10 min,
  launcher open/search, notification storm, lock/unlock). Run against C++ and
  Rust-Phase-A builds. Done: numbers + ranked hot spots committed (docs/profiling/).
- [ ] B.2 Allocator [future-candidate: tikv-jemallocator, mimalloc, snmalloc] vs
  system malloc, decided on B.1 scenarios. Done: decision + numbers recorded here.
- [ ] B.3 GLES binding [future-candidate: glow vs raw bindgen] — re-decide 11.2's
  provisional pick on frame-time/overhead data. Done: decision recorded, loser removed.
- [ ] B.4 Text stack [future-candidate: cosmic-text (harfrust/swash/fontdb)] vs
  pangocairo FFI. Riskiest swap: fontconfig matching parity is the known gap; needs
  a render-parity corpus (scripts incl. RTL/CJK/emoji) + shaping benchmarks.
- [ ] B.5 2D raster [future-candidate: tiny-skia] — only where cairo rasters outside
  the text path; step-1 audit of actual cairo API usage decides if this even exists
  as a separable swap.
- [ ] B.6 Image decoders [future-candidate: image, image-webp, jxl-oxide, resvg] vs
  libwebp/libjxl/librsvg — correctness corpus + decode-time benchmarks.
- [ ] B.7 HTTP [future-candidate: reqwest(rustls), ureq] vs libcurl.
- [ ] B.8 XML [future-candidate: quick-xml, roxmltree] vs libxml2 (caldav only —
  tiny surface, likely an easy win, still needs the checklist).
- [ ] B.9 Secrets [future-candidate: secret-service, oo7] vs libsecret.
- [ ] B.10 Crypto [future-candidate: RustCrypto per-primitive] vs libsodium — input
  is the 7.6 call-site audit; on-disk/interop format compatibility is mandatory.
- [ ] B.11 Markdown [future-candidate: pulldown-cmark, comrak] vs md4c — parser
  behavior diff over the notification/UI markdown corpus.
- [ ] B.12 Fuzzy match [future-candidate: nucleo-matcher] vs vendored fzy — ranking
  is UX-visible; needs a side-by-side corpus comparison, not score equality.
- [ ] B.13 WAV decode [future-candidate: hound, symphonia] vs vendored drwav.
- [ ] B.14 Desktop entries [future-candidate: freedesktop-desktop-entry] vs the 5.5
  hand-port.

## Session-restart protocol (for a cold session)

1. Read `CLAUDE.md`, this file, then the **tail of `PROGRESS.log`**.
2. `git log --oneline -10` and `git status` — the tree must be clean; if it isn't, the
   previous session died mid-task: read the diff, finish or revert to a committable
   state *first*.
3. Find the first unchecked task above; cross-check against PROGRESS.log's "next step".
4. `nix develop .#rust -c just check` must pass before you write anything new.
