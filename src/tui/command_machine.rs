//! Centralized state machine for command/subcommand popup transitions.
//!
//! Encapsulates all logic for transitioning between popup modes
//! (`Idle` → `Commands` → `SubCommand` ↔ `Commands` ↔ `Idle`)
//! based on user input events, including the auto-switch on space/backspace.

use crate::tui::commands;
use crate::tui::state::{AppState, PopupMode};

/// Events that can drive state transitions.
pub enum InputEvent {
    Char(char),
    Backspace,
    Enter,
    Esc,
    Up,
    Down,
}

/// Result returned after processing an event.
pub enum EventResult {
    /// Event was consumed by the state machine (modified popup/input/selection).
    Handled,
    /// The machine says to dispatch this command (Enter on a selected item).
    Dispatch(String),
    /// Event was not consumed — caller should handle it (e.g. normal typing in Idle).
    Passthrough,
}

/// Run the state machine: given a key event and the current app state,
/// mutate the state (popup mode, input, selection) and return what the
/// caller should do next.
///
/// Call this BEFORE any other input handling — popup transitions take priority.
pub fn on_event(state: &mut AppState, event: InputEvent) -> EventResult {
    // First, check if we're in a special popup that uses input for filtering
    // (Providers, ModelPicker, KeyInput) — let the handler deal with those.
    match state.popup_mode {
        PopupMode::Providers | PopupMode::ModelPicker | PopupMode::KeyInput => {
            return on_filter_popup(state, event);
        }
        _ => {}
    }

    match event {
        InputEvent::Char(c) => on_char(state, c),
        InputEvent::Backspace => on_backspace(state),
        InputEvent::Enter => on_enter(state),
        InputEvent::Esc => on_esc(state),
        InputEvent::Up => on_up(state),
        InputEvent::Down => on_down(state),
    }
}

// ── Per-event handlers ──

fn on_char(state: &mut AppState, c: char) -> EventResult {
    match state.popup_mode {
        PopupMode::None => {
            // Not in a popup — let handler insert the char normally
            if c == '/' {
                // Starting a command
                state.ui.input.clear();
                state.ui.input.push('/');
                state.ui.input_cursor = 1;
                state.popup_mode = PopupMode::Commands;
                state.ui.command_popup_selection = 0;
                EventResult::Handled
            } else {
                EventResult::Passthrough
            }
        }
        PopupMode::Commands | PopupMode::SubCommand { .. } => {
            // Insert char into input
            let byte_idx = char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor);
            state.ui.input.insert(byte_idx, c);
            state.ui.input_cursor += 1;
            state.ui.command_popup_selection = 0;
            state.popup_selected = 0;
            state.ui.input_history_idx = None;

            // Re-sync popup mode based on new input
            sync(state);
            EventResult::Handled
        }
        _ => EventResult::Passthrough,
    }
}

fn on_backspace(state: &mut AppState) -> EventResult {
    match state.popup_mode {
        PopupMode::None => return EventResult::Passthrough,
        PopupMode::Commands | PopupMode::SubCommand { .. } => {
            if state.ui.input_cursor == 0 {
                return EventResult::Handled;
            }
            state.ui.input_cursor -= 1;
            let byte_idx = char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor);
            state.ui.input.remove(byte_idx);
            state.ui.command_popup_selection = 0;
            state.popup_selected = 0;
            state.ui.input_history_idx = None;
            sync(state);
            EventResult::Handled
        }
        _ => EventResult::Passthrough,
    }
}

fn on_enter(state: &mut AppState) -> EventResult {
    match &state.popup_mode {
        PopupMode::Commands => {
            let prefix = state.ui.input.trim().to_lowercase();
            let matches: Vec<_> = commands::COMMANDS
                .iter()
                .filter(|(cmd, _)| cmd.starts_with(&prefix))
                .collect();
            if let Some((cmd, _)) = matches.get(state.popup_selected.min(matches.len().saturating_sub(1))) {
                let full_cmd = cmd.to_string();
                state.popup_mode = PopupMode::None;
                EventResult::Dispatch(full_cmd)
            } else if prefix.starts_with('/') {
                // No COMMANDS match — dispatch the raw input directly.
                // This handles sub-commands like "/role default", "/pool stats", etc.
                state.popup_mode = PopupMode::None;
                EventResult::Dispatch(prefix.clone())
            } else {
                EventResult::Handled
            }
        }
        PopupMode::SubCommand { parent, items } => {
            // Resolve items (dynamic if items empty)
            let resolved: Vec<(String, String)> = if items.is_empty() {
                commands::resolve_dynamic_items(parent, &state.core)
            } else {
                items.iter().map(|(n, d)| (n.clone(), d.clone())).collect()
            };
            let filter_text = subcommand_filter_text(&state.ui.input, parent);
            let filtered: Vec<_> = resolved
                .iter()
                .filter(|(name, _)| filter_text.is_empty() || name.to_lowercase().contains(&filter_text.to_lowercase()))
                .collect();
            if let Some((name, _)) = filtered.get(state.popup_selected.min(filtered.len().saturating_sub(1))) {
                let full_cmd = format!("{} {}", parent, name);
                state.popup_mode = PopupMode::None;
                EventResult::Dispatch(full_cmd)
            } else {
                state.popup_mode = PopupMode::None;
                EventResult::Handled
            }
        }
        _ => EventResult::Passthrough,
    }
}

fn on_esc(state: &mut AppState) -> EventResult {
    // Clear input for filter popups
    if matches!(state.popup_mode, PopupMode::Providers | PopupMode::ModelPicker) {
        state.ui.input.clear();
        state.ui.input_cursor = 0;
    }
    state.popup_mode = PopupMode::None;
    state.ui.command_popup_selection = 0;
    state.popup_selected = 0;
    EventResult::Handled
}

fn on_up(state: &mut AppState) -> EventResult {
    match state.popup_mode {
        PopupMode::Providers | PopupMode::ModelPicker | PopupMode::KeyInput => {
            state.popup_selected = state.popup_selected.saturating_sub(1);
            EventResult::Handled
        }
        PopupMode::Commands => {
            state.ui.command_popup_selection = state.ui.command_popup_selection.saturating_sub(1);
            EventResult::Handled
        }
        PopupMode::SubCommand { .. } => {
            state.popup_selected = state.popup_selected.saturating_sub(1);
            EventResult::Handled
        }
        PopupMode::None => EventResult::Passthrough,
    }
}

fn on_down(state: &mut AppState) -> EventResult {
    match state.popup_mode {
        PopupMode::Providers | PopupMode::ModelPicker | PopupMode::KeyInput => {
            state.popup_selected += 1;
            EventResult::Handled
        }
        PopupMode::Commands => {
            state.ui.command_popup_selection += 1;
            EventResult::Handled
        }
        PopupMode::SubCommand { .. } => {
            state.popup_selected += 1;
            EventResult::Handled
        }
        PopupMode::None => EventResult::Passthrough,
    }
}

// ── Filter popups (Providers, ModelPicker, KeyInput) ──

fn on_filter_popup(state: &mut AppState, event: InputEvent) -> EventResult {
    match event {
        InputEvent::Char(c) => {
            let byte_idx = char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor);
            state.ui.input.insert(byte_idx, c);
            state.ui.input_cursor += 1;
            state.popup_selected = 0;
            EventResult::Handled
        }
        InputEvent::Backspace => {
            if state.ui.input_cursor > 0 {
                state.ui.input_cursor -= 1;
                let byte_idx = char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor);
                state.ui.input.remove(byte_idx);
                state.popup_selected = 0;
            }
            EventResult::Handled
        }
        InputEvent::Esc => {
            state.ui.input.clear();
            state.ui.input_cursor = 0;
            state.popup_mode = PopupMode::None;
            state.popup_selected = 0;
            EventResult::Handled
        }
        InputEvent::Up => {
            state.popup_selected = state.popup_selected.saturating_sub(1);
            EventResult::Handled
        }
        InputEvent::Down => {
            state.popup_selected += 1;
            EventResult::Handled
        }
        InputEvent::Enter => EventResult::Passthrough, // Let the handler deal with it
    }
}

// ── Core sync logic (replaces sync_popup_mode) ──

/// Re-sync the popup mode based on current input.
/// Called after every char insertion or deletion — only closes Commands
/// when the input no longer starts with `/`.  Never auto-switches to
/// SubCommand — that only happens via Enter + dispatch.
fn sync(state: &mut AppState) {
    // Don't touch filter-type popups
    if matches!(
        state.popup_mode,
        PopupMode::Providers | PopupMode::ModelPicker | PopupMode::KeyInput
    ) {
        return;
    }

    let input = &state.ui.input;
    if !input.starts_with('/') || input.trim() == "/" {
        // Not a command prefix anymore — close Commands popup
        if state.popup_mode == PopupMode::Commands {
            state.popup_mode = PopupMode::None;
        }
        // SubCommand is preserved (set by dispatch, not auto-switched)
        return;
    }
}

/// Extract filter text for subcommand popup — matches popup.rs logic.
/// If input starts with parent+space, extract the suffix.
/// Otherwise use the whole input (dispatch cleared the parent).
fn subcommand_filter_text<'a>(input: &'a str, parent: &str) -> &'a str {
    if input.starts_with(parent) {
        let after = &input[parent.len()..];
        if after.starts_with(' ') { &after[1..] } else { "" }
    } else {
        input
    }
}

// ── Helpers ──

/// Split input into (parent_command, text_after_space) at the first space.
/// Returns `(None, rest)` if there's no space.
pub(crate) fn split_at_space(input: &str) -> (Option<&str>, &str) {
    if let Some(pos) = input.find(' ') {
        (Some(&input[..pos]), &input[pos + 1..])
    } else {
        (None, input)
    }
}

fn char_idx_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(s.len())
}
