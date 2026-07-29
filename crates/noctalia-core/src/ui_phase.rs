//! Port of `src/core/ui_phase.{cpp,h}`.
//!
//! The C++'s `#ifndef NDEBUG` guard (assertion bodies compiled in for debug
//! builds, compiled out entirely for release) maps to Rust's `cfg(debug_assertions)`,
//! which rustc sets for the `dev` profile and clears for `release` — the same
//! debug/release split meson's NDEBUG convention encodes, so this preserves intent,
//! not just syntax.

use std::cell::Cell;

#[cfg(debug_assertions)]
use crate::log::Logger;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiPhase {
    Idle,
    PrepareFrame,
    Update,
    Layout,
    Render,
}

pub fn ui_phase_name(phase: UiPhase) -> &'static str {
    match phase {
        UiPhase::Idle => "Idle",
        UiPhase::PrepareFrame => "PrepareFrame",
        UiPhase::Update => "Update",
        UiPhase::Layout => "Layout",
        UiPhase::Render => "Render",
    }
}

thread_local! {
    static CURRENT_UI_PHASE: Cell<UiPhase> = const { Cell::new(UiPhase::Idle) };
}

#[cfg(debug_assertions)]
static LOG: Logger = Logger::new("ui_phase");

pub fn current_ui_phase() -> UiPhase {
    CURRENT_UI_PHASE.with(|phase| phase.get())
}

/// RAII phase guard: sets the current thread's UI phase for its lifetime and
/// restores the previous one on drop, matching `UiPhaseScope`.
pub struct UiPhaseScope {
    previous: UiPhase,
}

impl UiPhaseScope {
    pub fn new(phase: UiPhase) -> Self {
        let previous = current_ui_phase();
        CURRENT_UI_PHASE.with(|p| p.set(phase));
        Self { previous }
    }
}

impl Drop for UiPhaseScope {
    fn drop(&mut self) {
        CURRENT_UI_PHASE.with(|p| p.set(self.previous));
    }
}

#[cfg(debug_assertions)]
fn assert_not_rendering(operation: &str) {
    let phase = current_ui_phase();
    if phase == UiPhase::Render {
        LOG.error(format_args!(
            "UI phase violation: {operation} is not allowed during {}",
            ui_phase_name(phase)
        ));
        std::process::abort();
    }
}

/// Debug-only assertion that no scene mutation is happening during the render
/// phase; a no-op in release builds. Matches `uiAssertSceneMutationAllowed`,
/// which has an identical body to `uiAssertNotRendering` in the C++.
pub fn ui_assert_scene_mutation_allowed(operation: &str) {
    #[cfg(debug_assertions)]
    assert_not_rendering(operation);
    #[cfg(not(debug_assertions))]
    let _ = operation;
}

/// Debug-only assertion that the given operation isn't happening during the
/// render phase; a no-op in release builds. Matches `uiAssertNotRendering`.
pub fn ui_assert_not_rendering(operation: &str) {
    #[cfg(debug_assertions)]
    assert_not_rendering(operation);
    #[cfg(not(debug_assertions))]
    let _ = operation;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn names_all_phases() {
        assert_eq!(ui_phase_name(UiPhase::Idle), "Idle");
        assert_eq!(ui_phase_name(UiPhase::PrepareFrame), "PrepareFrame");
        assert_eq!(ui_phase_name(UiPhase::Update), "Update");
        assert_eq!(ui_phase_name(UiPhase::Layout), "Layout");
        assert_eq!(ui_phase_name(UiPhase::Render), "Render");
    }

    #[test]
    fn scope_sets_and_restores_phase_including_nested() {
        assert_eq!(current_ui_phase(), UiPhase::Idle);
        {
            let _outer = UiPhaseScope::new(UiPhase::Update);
            assert_eq!(current_ui_phase(), UiPhase::Update);
            {
                let _inner = UiPhaseScope::new(UiPhase::Render);
                assert_eq!(current_ui_phase(), UiPhase::Render);
            }
            assert_eq!(current_ui_phase(), UiPhase::Update);
        }
        assert_eq!(current_ui_phase(), UiPhase::Idle);
    }

    #[test]
    fn assert_functions_are_noop_outside_render_phase() {
        assert_eq!(current_ui_phase(), UiPhase::Idle);
        ui_assert_scene_mutation_allowed("test-op");
        ui_assert_not_rendering("test-op");
    }
}
