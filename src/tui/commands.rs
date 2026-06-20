//! Command dispatch — thin adapter to CommandRuntime.
//!
//! In Phase 2c+ this file no longer contains any business logic.
//! All commands live in the command tree (`command_tree.rs`).
//! This file exists only as a stable entry point for `handle_input_submit`
//! in `handler.rs` and the command-line interface.

use crate::tui::state::{AppState, CoreState};

// ═══════════════════════════════════════════════════════════════
//  Command dispatch
// ═══════════════════════════════════════════════════════════════

/// Dispatch a command via the CommandRuntime.
/// Returns ``true`` if the command was handled, ``false`` if unrecognized.
pub fn dispatch(trimmed: &str, state: &mut AppState, _now: &str) -> bool {
    let parsed = crate::tui::command_tree::parse(trimmed);
    let runtime = crate::tui::command_tree::CommandRuntime;
    matches!(
        runtime.execute(&parsed, state).status,
        crate::tui::command_tree::CommandStatus::Handled
    )
}

// ═══════════════════════════════════════════════════════════════
//  Command registry (for help text and autocomplete)
// ═══════════════════════════════════════════════════════════════

/// Resolve dynamic items for the old popup system (subcommand completions).
/// Used by `popup.rs` for `PopupMode::SubCommand` rendering.
pub fn resolve_dynamic_items(parent: &str, core: &CoreState) -> Vec<(String, String)> {
    // Phase 2c: dynamic items are provided by the command tree's NodeProviders.
    // This function is kept for backward compat with the old popup rendering.
    crate::tui::command_tree::resolve_dynamic_items(parent, core)
}

/// All registered commands for auto-generated help and command popup.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/connect", "Configure a provider"),
    ("/models", "Open model picker"),
    ("/status", "Show system status"),
    ("/pool", "Pool management (stats/flush/clear/query)"),
    ("/reflect", "Reflection control (on/off/status/rule/max)"),
    ("/role", "Role templates (list/show/create/edit/delete)"),
    ("/agent", "Agent management (list/inspect)"),
    ("/sh", "Run a shell command"),
    ("/clear", "Clear conversation"),
    ("/refresh", "Refresh system prompt cache"),
    ("/sessions", "Switch to a saved session"),
    (
        "/memo",
        "Role memo management (list/show/write/delete/roles)",
    ),
    ("/think", "Set reasoning display level (0/1/2)"),
    ("/help", "Show help"),
];

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_help_returns_true() {
        let mut state = AppState::default();
        // dispatch("/help") should be handled by the runtime's help handler
        assert!(dispatch("/help", &mut state, "12:00:00"));
    }

    #[test]
    fn test_dispatch_unknown_returns_false() {
        let mut state = AppState::default();
        assert!(!dispatch("not a command", &mut state, "12:00:00"));
    }

    #[test]
    fn test_commands_has_expected_entries() {
        let names: Vec<&str> = COMMANDS.iter().map(|(c, _)| *c).collect();
        assert!(names.contains(&"/help"));
        assert!(names.contains(&"/role"));
        assert!(names.contains(&"/pool"));
        assert!(names.contains(&"/status"));
    }
}
