# CLAUDE.md — Noctalia Rust migration

This repo is the C++23 Noctalia Wayland shell being ported to Rust **in place**. The
C++ tree (`src/`, `tests/`, meson) is the reference implementation — read it freely,
modify it never (except final-cutover tasks that say so). Rust code lives in `crates/`.
The plugin system (`src/scripting/`) is **out of migration scope**.

## Standing rules (non-negotiable)

1. **At the start of every session, read `MIGRATION_PLAN.md` and `PROGRESS.log` first.**
   Then `git status` + `git log --oneline -10`. If the tree is dirty, a previous session
   was cut off mid-task: get it back to a clean, committable, `just check`-green state
   before doing anything else.
2. **Before ending any turn, if you touched code, update `PROGRESS.log` and commit —
   never leave the working tree dirty or in a broken state, even mid-task.** Sessions
   are killed without warning by usage limits; assume every turn is the last. Half-done
   work is fine to commit if it compiles and `just check` passes (mark it clearly in
   PROGRESS.log with the exact next step).
3. **Before marking any task done, review the full diff in a fresh subagent context**
   (an agent that did not write the code), with the task's pass/fail criterion from
   MIGRATION_PLAN.md in the prompt. Address its findings before checking the box.
4. Work happens on the `rust-migration` branch. If a session finds itself on `main`,
   check out `rust-migration` first.
5. Never `git push --force`, never rewrite published history, never commit secrets.
6. **Dependency choices follow the two-phase strategy** (MIGRATION_PLAN.md,
   "Dependency strategy"). Phase A ports FFI-first against the same C libraries the
   C++ already uses — parity beats purity, a working baseline beats the "best"
   crate. Pure-Rust swaps are Phase B only: tracked as `[future-candidate]` tasks,
   gated on a fully working shell plus real profiling data, and decided by the
   recorded research process (read the actual C API calls used, vet 2–3 candidate
   crates for maintenance and API coverage, benchmark against the Phase-A baseline
   on realistic input). Allocator and GLES-binding choices are Phase B decisions
   too — do not pre-empt them. Never swap a dependency "while you're in there."

## Quality bar — "done" means all of this

The one canonical command: **`just check`** (runs fmt --check, clippy -D warnings,
tests). Run it inside the dev shell: `nix develop .#rust -c just check` (or rely on
direnv). Additionally:

- Zero compiler warnings. `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo fmt --check` clean.
- All tests passing.
- No bare `.unwrap()` / `.expect()` in non-test code. Workspace lints deny
  `clippy::unwrap_used` / `clippy::expect_used`; where a panic is genuinely impossible
  or acceptable, use a scoped `#[allow(clippy::unwrap_used)]` **with a comment
  justifying it**. Test modules may blanket-allow both.
- No `unsafe` without a `// SAFETY:` comment. FFI crates keep unsafe at the boundary.
- New dependencies follow standing rule 6: Phase A binds the same C library the C++
  uses (canonical Rust equivalent only where the C++ dep has no C ABI:
  toml/serde_json/zbus). No `openssl-sys` (system libcurl does TLS itself), nothing
  that downloads at build time (Nix sandbox — see MIGRATION_PLAN.md "sandbox red
  flags"). After adding a dep, run `cargo tree -i openssl-sys` (expect "nothing
  depends on it").
- Match C++ behavior over "improving" it. Divergences must be deliberate and recorded
  in PROGRESS.log.

## Environment

- NixOS. Dev shell: `nix develop .#rust` (direnv auto-loads via `.envrc`). The shell
  provides the pinned Rust toolchain (rust-overlay, pinned by `flake.lock`) and every
  C library the FFI layer needs. If a build fails on a missing system lib, fix
  `nix/rust-devshell.nix`, don't work around it.
- The C++ build still works: `just build` / `just test` (meson). Use it to generate
  reference/golden outputs when a task calls for parity checks.
- `just check` is the Rust gate; `just --list` shows everything.

## Layout

- `MIGRATION_PLAN.md` — phases, tasks, crate choices, architecture decisions. The only
  place task status checkboxes live.
- `PROGRESS.log` — append-only session journal. Newest entry at the **bottom**.
- `crates/` — Rust workspace members, one per migration layer (see plan Phase list).
- `src/`, `tests/` — C++ reference. `tests/` C++ cases define expected behavior; port
  them with the code they cover.
