//! Diagnostics — issues collected during schema-driven reads/validation.
//! Port of `src/config/schema/diagnostics.h`.

use serde::{Deserialize, Serialize};

/// Port of `Diagnostics::Severity` (diagnostics.h:15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Warning,
    Error,
}

/// Port of `Diagnostics::RecoveryScope` (diagnostics.h:16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryScope {
    /// Informational; the setting was accepted.
    Advisory,
    /// The value was rejected; the struct default stands.
    Value,
    /// A multi-field component (e.g. a calendar account) was dropped.
    Component,
    /// The entire document is unusable.
    Document,
}

/// One diagnostic entry.
/// Port of `Diagnostics::Entry` (diagnostics.h:18-25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEntry {
    pub severity: Severity,
    pub recovery_scope: RecoveryScope,
    pub code: String,
    pub path: String,
    pub message: String,
    pub owner_path: String,
}

/// Accumulates issues found while reading or validating a config table.
/// Port of `Diagnostics` (diagnostics.h:14-91).
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    pub entries: Vec<DiagnosticEntry>,
}

impl Diagnostics {
    pub fn warn(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warn_with_code(path, message, "config.warning");
    }

    pub fn warn_with_code(
        &mut self,
        path: impl Into<String>,
        message: impl Into<String>,
        code: impl Into<String>,
    ) {
        self.entries.push(DiagnosticEntry {
            severity: Severity::Warning,
            recovery_scope: RecoveryScope::Advisory,
            code: code.into(),
            path: path.into(),
            message: message.into(),
            owner_path: String::new(),
        });
    }

    pub fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.error_with_code(path, message, "config.invalid-value");
    }

    pub fn error_with_code(
        &mut self,
        path: impl Into<String>,
        message: impl Into<String>,
        code: impl Into<String>,
    ) {
        self.entries.push(DiagnosticEntry {
            severity: Severity::Error,
            recovery_scope: RecoveryScope::Value,
            code: code.into(),
            path: path.into(),
            message: message.into(),
            owner_path: String::new(),
        });
    }

    pub fn component_error(
        &mut self,
        path: impl Into<String>,
        owner_path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.component_error_with_code(path, owner_path, message, "config.invalid-component");
    }

    pub fn component_error_with_code(
        &mut self,
        path: impl Into<String>,
        owner_path: impl Into<String>,
        message: impl Into<String>,
        code: impl Into<String>,
    ) {
        self.entries.push(DiagnosticEntry {
            severity: Severity::Error,
            recovery_scope: RecoveryScope::Component,
            code: code.into(),
            path: path.into(),
            message: message.into(),
            owner_path: owner_path.into(),
        });
    }

    pub fn fatal(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.fatal_with_code(path, message, "config.invalid-document");
    }

    pub fn fatal_with_code(
        &mut self,
        path: impl Into<String>,
        message: impl Into<String>,
        code: impl Into<String>,
    ) {
        self.entries.push(DiagnosticEntry {
            severity: Severity::Error,
            recovery_scope: RecoveryScope::Document,
            code: code.into(),
            path: path.into(),
            message: message.into(),
            owner_path: String::new(),
        });
    }

    /// Port of `Diagnostics::hasErrors` (diagnostics.h:53-60).
    pub fn has_errors(&self) -> bool {
        self.entries.iter().any(|e| e.severity == Severity::Error)
    }

    /// Port of `Diagnostics::hasFatalErrors` (diagnostics.h:62-69).
    pub fn has_fatal_errors(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.severity == Severity::Error && e.recovery_scope == RecoveryScope::Document)
    }

    /// Port of `Diagnostics::introducedErrorsComparedTo` (diagnostics.h:71-90).
    pub fn introduced_errors_compared_to(&self, baseline: &Diagnostics) -> Diagnostics {
        let mut introduced = Diagnostics::default();
        for candidate in &self.entries {
            if candidate.severity != Severity::Error {
                continue;
            }
            let existed = baseline.entries.iter().any(|previous| {
                candidate.severity == previous.severity
                    && candidate.recovery_scope == previous.recovery_scope
                    && candidate.code == previous.code
                    && candidate.path == previous.path
                    && candidate.message == previous.message
                    && candidate.owner_path == previous.owner_path
            });
            if !existed {
                introduced.entries.push(candidate.clone());
            }
        }
        introduced
    }
}

/// Joins a parent path and a key into a dotted path.
/// Port of `joinPath` (diagnostics.h:95-105).
pub fn join_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        return key.to_string();
    }
    let mut out = String::with_capacity(parent.len() + 1 + key.len());
    out.push_str(parent);
    out.push('.');
    out.push_str(key);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn join_path_empty_parent() {
        assert_eq!(join_path("", "key"), "key");
    }

    #[test]
    fn join_path_dotted() {
        assert_eq!(join_path("shell", "animation"), "shell.animation");
    }

    #[test]
    fn diagnostics_has_errors() {
        let mut diag = Diagnostics::default();
        assert!(!diag.has_errors());
        diag.warn("path", "msg");
        assert!(!diag.has_errors());
        diag.error("path", "msg");
        assert!(diag.has_errors());
    }

    #[test]
    fn diagnostics_has_fatal_errors() {
        let mut diag = Diagnostics::default();
        diag.error("path", "msg");
        assert!(!diag.has_fatal_errors());
        diag.fatal("path", "msg");
        assert!(diag.has_fatal_errors());
    }

    #[test]
    fn introduced_errors_compared_to_baseline() {
        let mut baseline = Diagnostics::default();
        baseline.error("shell.speed", "out of range");

        let mut current = Diagnostics::default();
        current.error("shell.speed", "out of range"); // same
        current.error("audio.volume", "out of range"); // new

        let introduced = current.introduced_errors_compared_to(&baseline);
        assert_eq!(introduced.entries.len(), 1);
        assert_eq!(introduced.entries[0].path, "audio.volume");
    }
}
