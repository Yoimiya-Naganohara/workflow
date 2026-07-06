//! Runtime event types — re-exported from wf-core to break circular deps.
//!
//! The canonical definition lives in `wf_core::event::RuntimeEvent`.
//! This module re-exports it so existing `crate::runtime::event::RuntimeEvent`
//! imports continue to work.

pub use wf_core::event::RuntimeEvent;
