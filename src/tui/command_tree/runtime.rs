//! CommandRuntime — execution engine.

use crate::tui::command_tree::parser::ParsedCommand;
use crate::tui::command_tree::{
    CommandContext, CommandInvocation, CommandPalette, CommandResult, Handler, NodeKind,
    NodeProvider, PaletteLevel, ROOT, UiEffect,
};
use crate::tui::state::{AppState, CoreState};

/// Command execution engine. All input modalities converge here.
pub struct CommandRuntime;

impl CommandRuntime {
    pub fn execute(&self, parsed: &ParsedCommand, state: &mut AppState) -> CommandResult {
        let (inv, handler) = match self.resolve(parsed, &state.core) {
            Some(result) => result,
            None => return CommandResult::error("Command not found in tree"),
        };
        let handler_result = handler(&inv, state);
        let status = handler_result.status.clone();
        for effect in handler_result.effects {
            self.apply_effect(effect, state);
        }
        CommandResult {
            status,
            effects: Default::default(),
        }
    }

    fn resolve(
        &self,
        parsed: &ParsedCommand,
        core: &CoreState,
    ) -> Option<(CommandInvocation, Handler)> {
        let mut level: PaletteLevel = PaletteLevel::Static(ROOT);
        for (i, token) in parsed.tokens.iter().enumerate() {
            let node_info = {
                let nodes = level.list();
                let found = nodes.iter().find(|n| n.id.as_ref() == token.as_str())?;
                match &found.kind {
                    NodeKind::Execute { handler } => {
                        let h: Handler = *handler;
                        NodeInfo::Execute(h)
                    }
                    NodeKind::Branch { provider } => {
                        let p: NodeProvider = *provider;
                        NodeInfo::Branch(p)
                    }
                }
            };
            match node_info {
                NodeInfo::Execute(handler) => {
                    let command_path = parsed.tokens[..=i].to_vec();
                    let args = parsed.tokens[i + 1..].to_vec();
                    let inv = CommandInvocation::new(command_path, args);
                    return Some((inv, handler));
                }
                NodeInfo::Branch(provider) => {
                    let ctx = CommandContext { path: &[], core };
                    level = PaletteLevel::Dynamic(provider(&ctx));
                }
            }
        }
        None
    }

    fn apply_effect(&self, effect: UiEffect, state: &mut AppState) {
        match effect {
            UiEffect::ClosePalette => {
                state.popup_mode = crate::tui::state::PopupMode::None;
                state.ui.command_palette = CommandPalette::default();
            }
            UiEffect::KeepPalette => {}
            UiEffect::OpenPopup(mode) => {
                state.popup_mode = mode;
            }
            UiEffect::SetInput(text) => {
                state.ui.input = text;
                state.ui.input_cursor = state.ui.input.len();
            }
            UiEffect::PushEffect(_) => {}
        }
    }
}

pub(crate) enum NodeInfo {
    Branch(NodeProvider),
    Execute(Handler),
}
