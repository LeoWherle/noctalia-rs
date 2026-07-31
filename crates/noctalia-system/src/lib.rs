//! System monitors: CPU/mem/disk/net stats, brightness, battery logic, desktop entries. Plan Phase 5.

pub mod app_identity;
pub mod battery_warning;
pub mod brightness;
pub mod cpu_stat;
pub mod cpu_temp;
pub mod day_night_schedule;
pub mod ddc;
pub mod dependency_service;
pub mod desktop_entry;
pub mod desktop_entry_launch;
pub mod disk;
pub mod distro_info;
pub mod format_units;
pub mod icon_resolver;
pub mod intel_gpu;
pub mod internal_app_metadata;
pub mod mem;
pub mod net;
pub mod rfkill_helper;
pub mod terminal_launch;
