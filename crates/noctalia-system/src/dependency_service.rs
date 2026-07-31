//! Port of `src/system/dependency_service.{cpp,h}` (task 5.6.1): tracks whether optional CLI
//! tools the shell knows about are present on `$PATH`, backed by already-ported
//! `noctalia_core::process::command_exists`. Fully self-contained — no forward-phase blocker.
//!
//! No C++ test exists for this file (confirmed: no `dependency_service_test.cpp` in `tests/`) —
//! task 5.6's own done bar is a smoke test.

use std::collections::HashMap;

use noctalia_core::process;

/// Optional CLI tools the shell knows about. Adding a new tracked tool is one line.
const TRACKED_TOOLS: [&str; 1] = ["ddcutil"];

/// Port of `DependencyService`.
pub struct DependencyService {
    present: HashMap<String, bool>,
}

impl Default for DependencyService {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyService {
    pub fn new() -> Self {
        let mut service = Self {
            present: HashMap::new(),
        };
        service.rescan();
        service
    }

    pub fn has(&self, name: &str) -> bool {
        self.present.get(name).copied().unwrap_or(false)
    }

    pub fn has_ddcutil(&self) -> bool {
        self.has("ddcutil")
    }

    pub fn rescan(&mut self) {
        self.present.clear();
        for name in TRACKED_TOOLS {
            self.present
                .insert(name.to_string(), process::command_exists(name));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn has_matches_command_exists_for_tracked_tools() {
        let service = DependencyService::new();
        assert_eq!(service.has("ddcutil"), process::command_exists("ddcutil"));
    }

    #[test]
    fn has_ddcutil_matches_has() {
        let service = DependencyService::new();
        assert_eq!(service.has_ddcutil(), service.has("ddcutil"));
    }

    #[test]
    fn has_returns_false_for_untracked_names() {
        let service = DependencyService::new();
        assert!(!service.has("definitely-not-a-tracked-tool"));
        assert!(!service.has(""));
    }

    #[test]
    fn rescan_repopulates_the_present_map() {
        let mut service = DependencyService::new();
        service.present.clear();
        assert!(!service.has("ddcutil"));
        service.rescan();
        assert_eq!(service.has("ddcutil"), process::command_exists("ddcutil"));
    }
}
