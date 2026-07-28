//! Port of `src/core/process/*` — task 1.6, split across 1.6.1-1.6.6 (see
//! MIGRATION_PLAN.md) because the C++ source spans genuinely distinct concerns
//! at ~30 public functions. This task (1.6.1) covers only the core synchronous
//! exec machinery: `RunResult`/`RunOptions`/`EnvOverride` and `run_sync`/
//! `run_sync_with_options`, built on `runSyncProcess`'s fork/exec/pipe/poll loop
//! (timeout, cancellation, output-byte-limit truncation, process-group
//! signaling). Async execution (a worker thread wrapping this), detached
//! double-fork spawning, `/proc` scanning, systemd integration, and fd
//! diagnostics are separate subtasks layered on top of this module.
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

mod core_exec;

pub use core_exec::{EnvOverride, RunOptions, RunResult, run_sync, run_sync_with_options};
