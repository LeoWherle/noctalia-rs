//! Port of `src/core/process/*` — task 1.6, split across 1.6.1-1.6.6 (see
//! MIGRATION_PLAN.md) because the C++ source spans genuinely distinct concerns
//! at ~30 public functions. So far this covers the core synchronous exec
//! machinery (1.6.1): `RunResult`/`RunOptions`/`EnvOverride` and `run_sync`/
//! `run_sync_with_options`, built on `runSyncProcess`'s fork/exec/pipe/poll loop
//! (timeout, cancellation, output-byte-limit truncation, process-group
//! signaling); and worker-thread async execution (1.6.2): `RunCallbacks` and
//! the `run_async*`/`run_sync_shell` family in `async_exec`, wrapping
//! `run_sync_process` from a spawned thread instead of adding new fork/exec
//! machinery. Detached double-fork spawning, `/proc` scanning, systemd
//! integration, and fd diagnostics are separate subtasks layered on top of
//! this module.
//!
//! Two deliberate divergences from the C++, both load-bearing enough to record
//! here rather than just in a code comment:
//!
//! - `RunResult::out`/`err` are `Vec<u8>`, not `String`: C++'s `std::string` is
//!   just a byte buffer with no UTF-8 requirement, and process output isn't
//!   guaranteed to be valid UTF-8either. Using `String` would mean either
//!   lossy-converting (silently corrupting binary output the C++ preserves
//!   faithfully) or a fallible API the C++ doesn't have.
//! - Built on `std::process::Command` rather than hand-rolled `fork`+`execvp`
//!   (Option B over Option A, deliberately): `Command` already handles the
//!   fork/exec edge cases correctly and safely; we only need to bolt on the
//!   C++-specific behavior it doesn't provide (process-group signaling,
//!   non-blocking streaming reads with truncation, timeout, cancellation) via
//!   minimal `unsafe` at the libc boundary. One real behavioral consequence:
//!   Rust's `Command::spawn()` detects a failed `execvp` inside the child via
//!   an internal CLOEXEC pipe and reports it back as an `Err` from `spawn()`
//!   itself — indistinguishable here from a failed `fork`/`clone`. The C++,
//!   which hand-rolls `fork`+`execvp`, lets a failed `execvp` fall through to
//!   `_exit(127)` in the child, observed by the parent as a normal exit code;
//!   a failed `fork()` instead returns early with exit code -1. Both cases
//!   collapse to exit code -1 here (see `run_sync_process`'s spawn-failure
//!   arm) since `Command` can't reliably distinguish them.
//!
//! One more, non-load-bearing: output-streaming callbacks aren't wrapped in
//! anything like C++'s `catch (...)` around `(*callback)(...)`. That
//! `catch(...)` exists because an exception escaping a C-style callback/thread
//! boundary is undefined behavior in C++; a panicking Rust closure just
//! unwinds normally through this function's caller, which is ordinary,
//! expected Rust behavior, not something that needs to be defended against
//! here.

mod async_exec;
mod core_exec;
mod detached;
mod matching;

pub use async_exec::{
    RunCallbacks, run_async, run_async_shell, run_async_shell_with_options, run_async_with_options,
    run_sync_shell,
};
pub use core_exec::{EnvOverride, RunOptions, RunResult, run_sync, run_sync_with_options};
pub use detached::{
    command_exists, launch_detached, launch_detached_shell, launch_detached_tracked,
    launch_first_available, resolve_privilege_escalator, terminate_tracked,
};
pub use matching::{command_line_matches_all, desktop_portal_available, flatpak_app_installed};

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    /// Shared by every test under `process::*` that either mutates process
    /// environment (`std::env::set_var`/`remove_var`) or triggers
    /// `detached`'s raw `libc::fork()` (directly, or indirectly via
    /// `launch_detached*`/`launch_first_available`). Held for the whole
    /// risky window by both kinds of test so the two families can never run
    /// concurrently within this test binary — see `detached`'s module doc
    /// comment for why a raw `fork()` isn't covered by std's own `ENV_LOCK`
    /// the way `std::process::Command::spawn` is (which is what makes
    /// `core_exec`'s and `async_exec`'s own env-touching tests safe without
    /// this lock, on their own — they only ever race against other
    /// `Command::spawn` callers, never a raw `fork()`).
    pub(crate) static ENV_MUTATION_LOCK: Mutex<()> = Mutex::new(());
}
