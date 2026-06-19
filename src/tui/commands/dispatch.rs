//! Command dispatch — node-based tree.

use super::node::{CommandStack, Node};
use crate::tui::state::{AppState, ChatMessage};

// ═══════════════════════════════════════════════════════════════
//  Dispatch
// ═══════════════════════════════════════════════════════════════

/// Get the current matching commands from the stack.
pub fn get_current_commands<'a>(
    stack: &'a CommandStack,
    input: &str,
) -> Vec<(usize, &'a dyn Node)> {
    let last = input.split_whitespace().last().unwrap_or("");
    stack
        .top()
        .iter()
        .enumerate()
        .filter(|(_, node)| node.name().contains(last))
        .map(|(i, node)| (i, node.as_ref()))
        .collect()
}

/// Display commands in a list.
pub fn display(commands: &[(usize, &dyn Node)]) {
    if commands.is_empty() {
        println!("No matching commands");
        return;
    }

    for (_, node) in commands {
        println!("  {:30} {}", node.name(), node.desc());
    }
}

/// Select a command by index.
pub fn select<'a>(commands: &'a [(usize, &dyn Node)], index: usize) -> Option<&'a dyn Node> {
    commands
        .iter()
        .find(|(i, _)| *i == index)
        .map(|(_, node)| *node)
}

// ═══════════════════════════════════════════════════════════════
//  Dispatch
// ═══════════════════════════════════════════════════════════════

/// Legacy dispatch function — parses input and executes the command.
pub fn dispatch(trimmed: &str, state: &mut AppState, now: &str) -> bool {
    let core = &mut state.core;
    let ui = &mut state.ui;

    match trimmed {
        "/connect" => {
            ui.input.clear();
            ui.input_cursor = 0;
            state.popup_mode = crate::tui::state::PopupMode::Providers;
            state.popup_selected = 0;
            state
                .effects
                .push(crate::tui::effect::Effect::FetchModelRegistry);
            true
        }
        "/models" | "/model" | "/m" => {
            if core.configured_providers.is_empty() {
                push_msg(
                    state,
                    "No providers configured. Use `/connect` first.".to_string(),
                );
                return true;
            }
            state.popup_mode = crate::tui::state::PopupMode::ModelPicker;
            state.popup_selected = 0;
            push_msg(state, "Select models to add to your pool".to_string());
            true
        }
        "/help" | "/?" => {
            let help_text = super::COMMANDS
                .iter()
                .map(|(cmd, desc)| format!("{:20} {}", cmd, desc))
                .collect::<Vec<_>>()
                .join("\n");
            push_msg(state, help_text);
            true
        }
        "/status" | "/info" => {
            let msg = format!(
                "System Status:\n  Providers: {}\n  Models: {}",
                core.configured_providers.len(),
                core.selected_models.len()
            );
            push_msg(state, msg);
            true
        }
        _ => {
            // Try node-based dispatch
            let stack = CommandStack::new();
            let input = trimmed.trim_start_matches('/');
            let commands = get_current_commands(&stack, input);

            if let Some((_, node)) = commands.first() {
                let args: Vec<String> =
                    input.split_whitespace().skip(1).map(String::from).collect();

                // Check if it's a branch node (has children)
                let children = node.children();
                if !children.is_empty() {
                    // Branch node — open popup
                    state.popup_mode = crate::tui::state::PopupMode::SubCommand {
                        parent: node.name().to_string(),
                        items: children
                            .iter()
                            .map(|c| (c.name().to_string(), c.desc().to_string()))
                            .collect(),
                    };
                    state.popup_selected = 0;
                    ui.input.clear();
                    ui.input_cursor = 0;
                    return true;
                }

                // Leaf node — execute
                return node.execute(&args, state, now);
            }

            false
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════

fn push_msg(state: &mut AppState, text: String) {
    state.core.messages.push(ChatMessage::system(text));
}
