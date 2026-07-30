//! Library half of the `noctalia-shell` crate: shell modules that are useful as ordinary Rust
//! APIs (and therefore unit-testable) rather than CLI-only glue. `main.rs` is the bin entry
//! point and CLI dispatch; see MIGRATION_PLAN.md Phase 4 for that half's task history.

pub mod hooks;
