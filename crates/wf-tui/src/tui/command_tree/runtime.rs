//! CommandRuntime — execution engine.

use crate::tui::command_tree::parser::ParsedCommand;
use crate::tui::command_tree::{
    CommandContext, CommandInvocation, CommandPalette, CommandResult, Handler, Node, NodeKind,
    NodeProvider, PaletteLevel, ROOT, UiEffect,
};
use crate::tui::state::{AppState, CoreState, PopupMode};

/// Command execution engine. All input modalities converge here.
pub struct CommandRuntime;

impl CommandRuntime {
    pub fn execute(&self, parsed: &ParsedCommand, state: &mut AppState) -> CommandResult {
        match self.resolve(parsed, &state.core) {
            ResolveResult::Handler(inv, handler) => {
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
            ResolveResult::Branch(children, resolved_path) => {
                if children.is_empty() {
                    state
                        .core
                        .messages
                        .push(crate::tui::state::ChatMessage::system(
                            "No items available.",
                        ));
                    return CommandResult::handled();
                }
                // 将已解析的路径写入 palette，使选中子项时能生成正确 tokens
                state.ui.command_palette.path = resolved_path
                    .into_iter()
                    .map(|s| crate::tui::command_tree::PathEntry { id: s })
                    .collect();
                state.ui.command_palette.level = PaletteLevel::Dynamic(children);
                state.popup_mode = PopupMode::CommandPalette;
                state.ui.command_palette.filter.clear();
                state.ui.command_palette.selected = 0;
                CommandResult::handled()
            }
            ResolveResult::NotFound => CommandResult::error("Command not found"),
        }
    }

    fn resolve(&self, parsed: &ParsedCommand, core: &CoreState) -> ResolveResult {
        let mut level: PaletteLevel = PaletteLevel::Static(ROOT);
        for (i, token) in parsed.tokens.iter().enumerate() {
            let node_info = {
                let nodes = level.list();
                let Some(found) = nodes.iter().find(|n| n.id.as_ref() == token.as_str()) else {
                    return ResolveResult::NotFound;
                };
                match &found.kind {
                    NodeKind::Execute { handler } => NodeInfo::Execute(*handler),
                    NodeKind::Branch { provider } => NodeInfo::Branch(*provider),
                }
            };
            match node_info {
                NodeInfo::Execute(handler) => {
                    let command_path = parsed.tokens[..=i].to_vec();
                    let args = parsed.tokens[i + 1..].to_vec();
                    let inv = CommandInvocation::new(command_path, args);
                    return ResolveResult::Handler(inv, handler);
                }
                NodeInfo::Branch(provider) => {
                    let ctx = CommandContext { path: &[], core };
                    level = PaletteLevel::Dynamic(provider(&ctx));
                }
            }
        }
        // 所有 token 用完，最后一个 level 是 Branch →
        // 弹出子命令列表供用户选择
        match level {
            PaletteLevel::Dynamic(children) => {
                ResolveResult::Branch(children, parsed.tokens.clone())
            }
            PaletteLevel::Static(_) => ResolveResult::NotFound,
        }
    }

    fn apply_effect(&self, effect: UiEffect, state: &mut AppState) {
        match effect {
            UiEffect::ClosePalette => {
                state.popup_mode = PopupMode::None;
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

pub(crate) enum ResolveResult {
    Handler(CommandInvocation, Handler),
    /// 分支，附带 resolve 已匹配的路径段（供 palette 导航用）
    Branch(Vec<Node>, Vec<String>),
    NotFound,
}

pub(crate) enum NodeInfo {
    Branch(NodeProvider),
    Execute(Handler),
}
