//! Slash command dispatch table.
//!
//! Maps user-input commands (``/connect``, ``/models``, ``/sh``, etc.)
//! to their implementations.  Synchronous operations are handled
//! inline; async operations push an [`Effect`] onto the state's
//! effect queue.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::tui::effect::Effect;
use crate::tui::state::{AppState, ChatMessage};

/// Try to dispatch a command.  Returns ``true`` if the input was a
/// recognised command (even if it failed), ``false`` if it should
/// be treated as a normal chat message.
pub fn dispatch(trimmed: &str, state: &mut AppState, _self_state: &Arc<RwLock<AppState>>, now: &str) -> bool {
    let core = &mut state.core;
    let ui = &mut state.ui;

    match trimmed {
        "/connect" => {
            ui.input.clear();
            ui.input_cursor = 0;
            state.active_dialog = Some(crate::tui::dialogs::ActiveDialog::Provider(
                crate::tui::dialogs::provider::ProviderDialog::new(),
            ));
            state.effects.push(Effect::FetchModelRegistry);
            true
        }

        "/models" | "/model" => {
            if core.configured_providers.is_empty() {
                core.messages
                    .push(ChatMessage::system("No providers configured. Use `/connect` first."));
                ui.input.clear();
                ui.input_cursor = 0;
                return true;
            }
            state.active_dialog = Some(crate::tui::dialogs::ActiveDialog::ModelPicker(
                crate::tui::dialogs::model_picker::ModelPicker::new(),
            ));
            core.messages
                .push(ChatMessage::system("Select models to add to your pool"));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/keymap" => {
            let bindings = state.keymap.all_bindings();
            let mut lines = vec!["Keyboard Shortcuts:".to_string(), String::new()];
            for (key, action) in &bindings {
                lines.push(format!("  {:20} {}", key, crate::tui::keymap::format_action(action)));
            }
            core.messages.push(ChatMessage::system(lines.join("\n")));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/help" | "/?" => {
            let help_text = [
                "/connect        - Configure a provider (API key + custom)",
                "/models         - Manage model pool (add/remove models)",
                "/custom         - Add/list/remove custom providers",
                "/clear          - Clear conversation",
                "/sh <cmd>       - Run a shell command",
                "/help           - Show this help",
                "",
                "Ctrl+X          - Stop current response",
                "Ctrl+C          - Quit",
            ]
            .join("\n");
            core.messages.push(ChatMessage::system(help_text));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/clear" | "/new" => {
            core.messages.clear();
            core.messages.push(ChatMessage::system(
                "Workflow Agent — conversation cleared. Describe your goal and I'll help.",
            ));
            core.responsible_agent_id = None;
            ui.input.clear();
            ui.input_cursor = 0;
            ui.chat_scroll = 0;
            true
        }

        "/sh" => {
            core.messages.push(ChatMessage::system("Usage: /sh <command>"));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Pool commands (sync sub-commands inline, query via Effect) ──
        "/pool" => {
            let help = [
                "Pool commands:",
                "  /pool stats      - Show experience pool statistics",
                "  /pool flush      - Flush bedrock to disk",
                "  /pool clear      - Clear both tracks",
                "  /pool query <q>  - Query experiences by text similarity",
                "  /pool export     - Export experiences as JSON",
                "  /pool import     - Import experiences from JSON",
            ]
            .join("\n");
            core.messages.push(ChatMessage::system(help));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/pool stats" => {
            let msg = if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    [
                        "Experience Pool Statistics:",
                        &format!("  Total entries:    {}", rt.experience_count()),
                        &format!("  Bedrock (A-track): {}", rt.bedrock_count()),
                        &format!("  Fluid  (B-track): {}", rt.fluid_count()),
                        &format!("  Pending suspend:  {}", rt.pending_suspended()),
                        &format!("  Remaining budget: {}", rt.remaining_budget()),
                        &format!("  Available permits:{}", rt.available_permits()),
                    ]
                    .join("\n")
                } else {
                    "Runtime locked".to_string()
                }
            } else {
                "Runtime not available".to_string()
            };
            core.messages.push(ChatMessage::system(msg));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/pool flush" => {
            let msg = if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    match rt.flush_experience_pool() {
                        Ok(()) => "Experience pool flushed to disk".to_string(),
                        Err(e) => format!("Flush failed: {}", e),
                    }
                } else {
                    "Runtime locked".to_string()
                }
            } else {
                "Runtime not available".to_string()
            };
            let is_err = msg.contains("failed") || msg.contains("locked") || msg.contains("not available");
            let status = if is_err {
                crate::tui::state::MessageStatus::Error
            } else {
                crate::tui::state::MessageStatus::Completed
            };
            core.messages.push(ChatMessage {
                role: crate::tui::state::MessageRole::System,
                content: msg,
                timestamp: now.to_string(),
                status,
            });
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/pool clear" => {
            let msg = if core.runtime.is_some() {
                "Pool clear requires runtime write access — not available via CLI".to_string()
            } else {
                "Runtime not available".to_string()
            };
            core.messages.push(ChatMessage::system(msg));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/pool export" => {
            core.messages.push(ChatMessage::system(
                "Export not yet implemented. Pool file is at ~/.workflow/experience_a.bin",
            ));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/pool import" => {
            core.messages.push(ChatMessage::system("Import not yet implemented"));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Custom provider commands (all sync) ──
        "/custom" => {
            let help = [
                "Custom Provider Commands:",
                "  /custom add                              - Add a new custom provider (wizard)",
                "  /custom list                             - List configured custom providers",
                "  /custom remove <name>                    - Remove a custom provider",
            ]
            .join("\n");
            core.messages.push(ChatMessage::system(help));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/custom add" => {
            state.active_dialog = Some(crate::tui::dialogs::ActiveDialog::CustomWizard(
                crate::tui::dialogs::custom_wizard::CustomWizard::new(),
            ));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/custom list" => {
            let custom: Vec<_> = core
                .models
                .providers()
                .iter()
                .filter(|p| p.id.starts_with("custom-"))
                .collect();
            if custom.is_empty() {
                core.messages
                    .push(ChatMessage::system("No custom providers configured."));
            } else {
                let mut lines = vec![format!("Custom Providers ({}):", custom.len())];
                for p in &custom {
                    let model_count = p.models.len();
                    let has_key = if let Some(env) = p.env.first() {
                        if core.api_keys.contains_key(env) { "✓" } else { "⌁" }
                    } else {
                        ""
                    };
                    let api_url = p.api.as_deref().unwrap_or("-");
                    lines.push(format!(
                        "  {}  {} ({} model(s)) — {}",
                        has_key, p.name, model_count, api_url
                    ));
                }
                core.messages.push(ChatMessage::system(lines.join("\n")));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Sub-commands with arguments ──
        _ if trimmed.starts_with("/sh ") => {
            let arg = trimmed.strip_prefix("/sh ").unwrap().trim();
            if !arg.is_empty() {
                core.messages.push(ChatMessage::system(format!("$ {}", arg)));
                state.effects.push(Effect::ExecuteShell {
                    command: arg.to_string(),
                });
            }
            ui.input.clear();
            ui.input_cursor = 0;
            ui.chat_scroll = 0;
            true
        }

        _ if trimmed.starts_with("/pool query ") || trimmed.starts_with("/pool q ") => {
            let query_text = trimmed.split_once(' ').map(|x| x.1).unwrap_or("").trim().to_string();
            if query_text.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /pool query <text>"));
            } else if let Some(runtime) = core.runtime.clone() {
                state.effects.push(Effect::PoolQuery {
                    query_text,
                    runtime,
                    now: now.to_string(),
                });
            } else {
                core.messages
                    .push(ChatMessage::system("Runtime not available for query"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/custom remove ") => {
            let name = trimmed.strip_prefix("/custom remove ").unwrap().trim().to_string();
            if !name.is_empty() {
                let custom_id = crate::models::CustomProvider::slug(&name);
                // Sync: remove from persistence
                if let Err(e) = crate::persistence::remove_custom_provider(&custom_id) {
                    core.messages
                        .push(ChatMessage::system(format!("Failed to remove custom provider: {}", e)));
                } else {
                    // Sync: update state
                    let provider_id = format!("custom-{}", custom_id);
                    core.models.remove_custom_provider(&custom_id);
                    core.configured_providers
                        .retain(|p| p != &provider_id && p != &custom_id);
                    core.provider_clients.remove(&provider_id);
                    let env_key = format!("CUSTOM_{}_API_KEY", custom_id.to_uppercase());
                    core.api_keys.remove(&env_key);
                    core.selected_models.retain(|sm| sm.provider_id != provider_id);
                    core.messages
                        .push(ChatMessage::system(format!("Custom provider \"{}\" removed", name)));
                }
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Pool sub-commands (catch-all: /pool <something>) ──
        // These should be caught by the specific matches above.
        // If we get here, it's an unknown sub-command.
        _ if trimmed.starts_with("/pool ") => {
            let rest = trimmed.strip_prefix("/pool ").unwrap().trim();
            core.messages.push(ChatMessage::system(format!(
                "Unknown pool command: {}. Use /pool for help.",
                rest
            )));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Role management commands ──
        "/role" | "/role help" => {
            let help = [
                "Role Template Commands:",
                "  /role list                    - List all role templates",
                "  /role show <name>             - Show role template details",
                "  /role create                  - Create a new role template (wizard)",
                "  /role edit <name>             - Edit an existing role template (wizard)",
                "  /role delete <name>           - Delete a role template",
                "  /role embed                    - Compute embeddings for all roles missing them",
                "  /role optimize <name>           - Optimize role prompt from experience",
            ]
            .join("\n");
            core.messages.push(ChatMessage::system(help));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/role list" => {
            if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    let templates = rt.all_role_templates();
                    if templates.is_empty() {
                        core.messages.push(ChatMessage::system("No role templates found."));
                    } else {
                        let mut lines = vec!["Role Templates:".to_string()];
                        for t in &templates {
                            let embedded = if t.embedding.is_some() { "✓" } else { "✗" };
                            lines.push(format!(
                                "  id={:<3}  {:<30}  label={:<20}  embedded={}",
                                t.template_id,
                                t.role,
                                t.label,
                                embedded
                            ));
                        }
                        core.messages.push(ChatMessage::system(lines.join("\n")));
                    }
                } else {
                    core.messages.push(ChatMessage::system("Runtime locked"));
                }
            } else {
                core.messages.push(ChatMessage::system("Runtime not available"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/role show ") => {
            let role_name = trimmed.strip_prefix("/role show ").unwrap().trim().to_string();
            if role_name.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /role show <name>"));
            } else if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    match rt.get_role_template(&role_name) {
                        Some(t) => {
                            let embedded = if t.embedding.is_some() { "yes" } else { "no" };
                            let details = format!(
                                "Role: {}\n  Label:        {}\n  ID:           {}\n  Embedded:     {}\n  Prompt ({}):\n{}\n{}\n{}",
                                t.role,
                                t.label,
                                t.template_id,
                                embedded,
                                t.system_prompt.len(),
                                "─".repeat(36),
                                t.system_prompt,
                                "─".repeat(36)
                            );
                            core.messages.push(ChatMessage::system(details));
                        }
                        None => {
                            core.messages.push(ChatMessage::system(format!(
                                "Role '{}' not found. Use /role list to see available roles.",
                                role_name
                            )));
                        }
                    }
                } else {
                    core.messages.push(ChatMessage::system("Runtime locked"));
                }
            } else {
                core.messages.push(ChatMessage::system("Runtime not available"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/role create" => {
            state.active_dialog = Some(crate::tui::dialogs::ActiveDialog::RoleWizard(
                crate::tui::dialogs::role::RoleWizard::new(),
            ));
            core.messages.push(ChatMessage::system("Creating new role template..."));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/role edit ") => {
            let role_name = trimmed.strip_prefix("/role edit ").unwrap().trim().to_string();
            if role_name.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /role edit <name>"));
            } else if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    match rt.get_role_template(&role_name) {
                        Some(t) => {
                            state.active_dialog = Some(
                                crate::tui::dialogs::ActiveDialog::RoleWizard(
                                    crate::tui::dialogs::role::RoleWizard::from_template(t),
                                ),
                            );
                        }
                        None => {
                            core.messages.push(ChatMessage::system(format!(
                                "Role '{}' not found. Use /role list to see available roles.",
                                role_name
                            )));
                        }
                    }
                } else {
                    core.messages.push(ChatMessage::system("Runtime locked"));
                }
            } else {
                core.messages.push(ChatMessage::system("Runtime not available"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/role embed" => {
            if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    let n = rt.all_role_templates().len();
                    core.messages.push(ChatMessage::system(format!(
                        "Computing embeddings for {} role template(s)...",
                        n
                    )));
                    rt.compute_role_embeddings_async();
                } else {
                    core.messages.push(ChatMessage::system("Runtime locked"));
                }
            } else {
                core.messages.push(ChatMessage::system("Runtime not available"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/role optimize ") => {
            let role_name = trimmed.strip_prefix("/role optimize ").unwrap().trim().to_string();
            if role_name.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /role optimize <name>"));
            } else if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    match rt.get_role_template(&role_name) {
                        Some(role) => {
                            let experiences = rt.get_experiences_by_role(role.template_id);
                            if experiences.len() < crate::runtime::optimizer::MIN_EXPERIENCES {
                                core.messages.push(ChatMessage::system(format!(
                                    "Need at least {} experiences for '{}', have {}. Keep using the role to gather more data.",
                                    crate::runtime::optimizer::MIN_EXPERIENCES,
                                    role_name,
                                    experiences.len()
                                )));
                            } else if rt.provider.is_some() {
                                core.messages.push(ChatMessage::system(format!(
                                    "Optimizing role '{}' from {} experiences...",
                                    role_name,
                                    experiences.len()
                                )));
                                state.effects.push(Effect::OptimizeRole {
                                    role_name: role_name.clone(),
                                    runtime: core.runtime.clone().unwrap(),
                                });
                            } else {
                                core.messages.push(ChatMessage::system(
                                    "No LLM provider configured. Connect a provider first via /connect.",
                                ));
                            }
                        }
                        None => {
                            core.messages.push(ChatMessage::system(format!(
                                "Role '{}' not found.",
                                role_name
                            )));
                        }
                    }
                } else {
                    core.messages.push(ChatMessage::system("Runtime locked"));
                }
            } else {
                core.messages.push(ChatMessage::system("Runtime not available"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/role delete ") => {
            let role_name = trimmed.strip_prefix("/role delete ").unwrap().trim().to_string();
            if role_name.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /role delete <name>"));
            } else if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    match rt.get_role_template(&role_name) {
                        Some(t) => {
                            // Delete via the role template store
                            rt.delete_role_template(t.template_id);
                            core.messages.push(ChatMessage::system(format!(
                                "Role '{}' deleted.",
                                role_name
                            )));
                        }
                        None => {
                            core.messages.push(ChatMessage::system(format!(
                                "Role '{}' not found.",
                                role_name
                            )));
                        }
                    }
                } else {
                    core.messages.push(ChatMessage::system("Runtime locked"));
                }
            } else {
                core.messages.push(ChatMessage::system("Runtime not available"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // Not a recognised command — let caller handle as chat message.
        _ => false,
    }
}

/// List of all registered commands, used by the command popup.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/connect", "Configure a provider (API key + custom)"),
    ("/models", "Manage model pool (add/remove models)"),
    ("/clear", "Clear conversation"),
    ("/sh", "Run a shell command"),
    ("/pool", "Manage experience pool (flush/clear/stats/export/import)"),
    ("/keymap", "Show keyboard shortcuts"),
    ("/custom", "Add/list/remove custom providers"),
    ("/help", "Show help"),
    ("/role", "Manage role templates (list/show/create/edit/delete)"),
];
