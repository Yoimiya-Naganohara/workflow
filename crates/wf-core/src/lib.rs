//! wf-core — Foundation types, constants, SIMD, metrics, guard, and task graph.
//!
//! This is the lowest-level crate with no dependencies on other workspace crates.

#![allow(clippy::module_inception)]

pub mod conflict;
pub mod constants;
pub mod event;
pub mod guard;
pub mod metrics;
pub mod simd;
pub mod task_graph;
pub mod types;

// Re-export commonly used items at crate root for ergonomic access.
pub use constants::*;
pub use types::*;
