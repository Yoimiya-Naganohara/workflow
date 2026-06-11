//! Dialog system — each dialog owns its state, event handling, and rendering.
//!
//! `ActiveDialog` is the single enum dispatched by the event loop.
//! `DialogTransition` controls dialog lifecycle (close, switch, stay).

pub mod command_popup;
pub mod custom_wizard;
pub mod key;
pub mod model_picker;
pub mod provider;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use super::state::CoreState;

/// Lifecycle signal returned by a dialog's `handle_key`.
#[derive(Debug)]
pub enum DialogTransition {
    /// Dialog stays active and unchanged.
    None,
    /// Dialog closes (returns to chat).
    Close,
    /// Replace this dialog with a different one.
    Switch(ActiveDialog),
}

/// Every active dialog variant.  One field per dialog module.
#[derive(Debug)]
pub enum ActiveDialog {
    Provider(provider::ProviderDialog),
    Key(key::KeyDialog),
    ModelPicker(model_picker::ModelPicker),
    CustomWizard(custom_wizard::CustomWizard),
}

// ── Dispatch methods ──

impl ActiveDialog {
    /// Route a key event to the active dialog.
    /// Returns the transition signal.
    pub fn handle_key(&mut self, state: &mut CoreState, key: KeyEvent) -> DialogTransition {
        match self {
            Self::Provider(d) => d.handle_key(state, key),
            Self::Key(d) => d.handle_key(state, key),
            Self::ModelPicker(d) => d.handle_key(state, key),
            Self::CustomWizard(d) => d.handle_key(state, key),
        }
    }

    /// Render the active dialog into the given frame area.
    pub fn render(&self, f: &mut Frame, area: Rect, state: &CoreState) {
        match self {
            Self::Provider(d) => d.render(f, area, state),
            Self::Key(d) => d.render(f, area, state),
            Self::ModelPicker(d) => d.render(f, area, state),
            Self::CustomWizard(d) => d.render(f, area, state),
        }
    }

    /// Handle a scroll-down mouse event.
    pub fn scroll_down(&mut self, state: &CoreState) {
        match self {
            Self::Provider(d) => d.scroll_down(state),
            Self::ModelPicker(d) => d.scroll_down(state),
            _ => {}
        }
    }

    /// Handle a scroll-up mouse event.
    pub fn scroll_up(&mut self, state: &CoreState) {
        match self {
            Self::Provider(d) => d.scroll_up(state),
            Self::ModelPicker(d) => d.scroll_up(state),
            _ => {}
        }
    }

    /// Whether the dialog expects rendered as a centered overlay (true) or
    /// as an inline popup above the input box (false).
    /// Provider and Key dialogs render inline; ModelPicker and CustomWizard
    /// use the full-screen overlay.
    pub fn is_overlay(&self) -> bool {
        matches!(self, Self::ModelPicker(_) | Self::CustomWizard(_))
    }

    /// Whether the dialog should be rendered as an inline popup (the
    /// complement of `is_overlay`).  Inline popups sit between the chat
    /// messages and the input box.
    pub fn is_popup(&self) -> bool {
        !self.is_overlay()
    }
}

// ── Teardown / cleanup ──

impl ActiveDialog {
    /// Called when the dialog is closing (user pressed Esc or
    /// the dialog transitioned to Close).  Lets the dialog
    /// persist any final state if needed.
    pub fn on_close(&mut self, _state: &mut CoreState) {}
}
