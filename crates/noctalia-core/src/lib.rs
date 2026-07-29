//! Foundations: logging, files, atomic writes, timers, event loop (calloop + tokio sidecar), process, i18n, time. Plan Phase 1.
//!
//! Skeleton crate — contents arrive with its migration phase; see MIGRATION_PLAN.md.

pub mod atomic_file;
pub mod build_info;
pub mod color;
pub mod event_loop;
pub mod files;
pub mod i18n;
pub mod limits;
pub mod log;
pub mod process;
pub mod profiling;
pub mod random;
pub mod time;
pub mod timing;
pub mod ui_phase;
