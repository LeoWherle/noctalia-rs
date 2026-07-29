//! Config schema framework — field descriptors, engine, diagnostics, and ranges.
//! Port of `src/config/schema/*`.
//!
//! The per-section schema definitions (the 2300-line `config_schema.cpp`) and
//! the section registry (`config_sections.{h,cpp}`) will be added in follow-up
//! commits as part of task 2.3.

pub mod config_schema;
pub mod config_sections;
pub mod diagnostics;
pub mod engine;
pub mod field;
pub mod ranges;
pub mod widget_schema;
