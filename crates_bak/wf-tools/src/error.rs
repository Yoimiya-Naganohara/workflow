//! Shared error type for all wf-tools tools.

/// Error type used by all tools in this crate.
///
/// Wraps a human-readable error message. Used as the associated
/// `Error` type in every `impl rig::tool::Tool` block.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolCallError(pub String);
