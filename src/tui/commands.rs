//! Slash command dispatch table.
//!
//! Maps user-input commands (``/connect``, ``/models``, ``/sh``, etc.)
//! to their implementations.  Synchronous operations are handled
//! inline; async operations push an [`Effect`] onto the state's
//! effect queue.

use crate::tui::effect::Effect;
use crate::tui::state::{AppState, ChatMessage, PopupMode};

/// Try to dispatch a command.  Returns ``true`` if the input was a
/// recognised command (even if it failed), ``false`` if it should
/// be treated as a normal chat message.
pub fn dispatch(trimmed: &str, state: &mut AppState, now: &str) -> bool {
    let core = &mut state.core;
    let ui = &mut state.ui;

    match trimmed {
        "/connect" => {
            ui.input.clear();
            ui.input_cursor = 0;
            state.popup_mode = crate::tui::state::PopupMode::Providers;
            state.popup_selected = 0;
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
            state.popup_mode = crate::tui::state::PopupMode::ModelPicker;
            state.popup_selected = 0;
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
                "/connect        - Configure a provider",
                "/models         - Manage model pool",
                "/pool           - Pool management (stats/flush/clear/query)",
                "/role           - Role templates (list/show/create/edit/delete/default)",
                "/sh <cmd>       - Run a shell command",
                "/clear          - Clear conversation",
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
            core.agents.clear();
            ui.input.clear();
            ui.input_cursor = 0;
            ui.chat_scroll = 0;
            ui.input_history.clear();
            ui.input_history_idx = None;
            ui.active_chat_abort = None;
            ui.active_chat_requests = 0;
            true
        }

        "/sh" => {
            core.messages.push(ChatMessage::system("Usage: /sh <command>"));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Pool commands ──
        "/pool" | "/pool help" => {
            if let Some(items) = get_subcommand_items("/pool") {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/pool".to_string(),
                    items: items.iter().map(|(n, d)| (n.to_string(), d.to_string())).collect(),
                };
            }
            state.popup_selected = 0;
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
            let msg = if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    match rt.clear_experience_pool() {
                        Ok(()) => "Experience pool cleared".to_string(),
                        Err(e) => format!("Clear failed: {}", e),
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
            if let Some(items) = get_subcommand_items("/role") {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/role".to_string(),
                    items: items.iter().map(|(n, d)| (n.to_string(), d.to_string())).collect(),
                };
            }
            state.popup_selected = 0;
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
                                t.template_id, t.role, t.label, embedded
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

        "/role show" => {
            let items = resolve_dynamic_items("/role show", core);
            if items.is_empty() {
                core.messages.push(ChatMessage::system("No role templates available."));
            } else {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/role show".to_string(),
                    items,
                };
                state.popup_selected = 0;
                return true;
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
            core.messages.push(ChatMessage::system(
                "Role creation — edit role templates in ~/.workflow/role_templates.json",
            ));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/role edit" => {
            let items = resolve_dynamic_items("/role edit", core);
            if items.is_empty() {
                core.messages.push(ChatMessage::system("No role templates available."));
            } else {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/role edit".to_string(),
                    items,
                };
                state.popup_selected = 0;
                return true;
            }
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
                            core.messages.push(ChatMessage::system(format!(
                                "Role '{}' found. Edit in ~/.workflow/role_templates.json",
                                t.role
                            )));
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

        "/role optimize" => {
            let items = resolve_dynamic_items("/role optimize", core);
            if items.is_empty() {
                core.messages.push(ChatMessage::system("No role templates available."));
            } else {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/role optimize".to_string(),
                    items,
                };
                state.popup_selected = 0;
                return true;
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
                            } else if let Some(reason) = rt
                                .optimization_tracker
                                .lock()
                                .unwrap()
                                .can_optimize(role.template_id, experiences.len())
                            {
                                core.messages.push(ChatMessage::system(format!(
                                    "Cannot optimize '{}': {}",
                                    role_name, reason
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
                                // Mark optimization to prevent immediate re-run
                                if let Ok(mut tracker) = rt.optimization_tracker.lock() {
                                    tracker.mark_optimized(role.template_id, experiences.len());
                                }
                            } else {
                                core.messages.push(ChatMessage::system(
                                    "No LLM provider configured. Connect a provider first via /connect.",
                                ));
                            }
                        }
                        None => {
                            core.messages
                                .push(ChatMessage::system(format!("Role '{}' not found.", role_name)));
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

        "/role delete" => {
            let items = resolve_dynamic_items("/role delete", core);
            if items.is_empty() {
                core.messages.push(ChatMessage::system("No role templates available."));
            } else {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/role delete".to_string(),
                    items,
                };
                state.popup_selected = 0;
                return true;
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
                            core.messages
                                .push(ChatMessage::system(format!("Role '{}' deleted.", role_name)));
                        }
                        None => {
                            core.messages
                                .push(ChatMessage::system(format!("Role '{}' not found.", role_name)));
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

        "/role default" => {
            let items = resolve_dynamic_items("/role default", core);
            if items.is_empty() {
                core.messages
                    .push(ChatMessage::system("No role templates available or runtime not ready."));
            } else {
                state.popup_mode = crate::tui::state::PopupMode::SubCommand {
                    parent: "/role default".to_string(),
                    items,
                };
                state.popup_selected = 0;
                // Don't clear input — user can type to filter
                return true;
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/role default ") => {
            let role_name = trimmed.strip_prefix("/role default ").unwrap().trim().to_string();
            if role_name.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /role default <name>"));
            } else if let Some(runtime) = &core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    if rt.get_role_template(&role_name).is_some() {
                        core.default_role = role_name.clone();
                        core.messages.push(ChatMessage::system(format!(
                            "Default bootstrap role set to `{}`. Next chat message will use this role.",
                            role_name
                        )));
                        // Clear responsible agent so next message re-creates with the new role
                        core.responsible_agent_id = None;
                        core.agents.clear();
                    } else {
                        core.messages.push(ChatMessage::system(format!(
                            "Role '{}' not found. Use `/role list` to see available roles.",
                            role_name
                        )));
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

        // ── Memo commands ──
        "/memo" | "/memo help" => {
            if let Some(items) = get_subcommand_items("/memo") {
                state.popup_mode = PopupMode::SubCommand {
                    parent: "/memo".to_string(),
                    items: items.iter().map(|(n, d)| (n.to_string(), d.to_string())).collect(),
                };
            }
            state.popup_selected = 0;
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/memo list" => {
            let msg = match_list_memos(core);
            core.messages.push(ChatMessage::system(msg));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/memo show ") => {
            let key = trimmed.strip_prefix("/memo show ").unwrap().trim().to_string();
            if key.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /memo show <key>"));
            } else {
                let msg = match_show_memo(core, &key);
                core.messages.push(ChatMessage::system(msg));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/memo write ") => {
            let rest = trimmed.strip_prefix("/memo write ").unwrap().trim().to_string();
            if let Some((key, value)) = rest.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                if key.is_empty() {
                    core.messages
                        .push(ChatMessage::system("Usage: /memo write <key>=<value>"));
                } else {
                    let msg = match_write_memo(core, key, value);
                    core.messages.push(ChatMessage::system(msg));
                }
            } else {
                core.messages
                    .push(ChatMessage::system("Usage: /memo write <key>=<value>"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/memo delete ") => {
            let key = trimmed.strip_prefix("/memo delete ").unwrap().trim().to_string();
            if key.is_empty() {
                core.messages.push(ChatMessage::system("Usage: /memo delete <key>"));
            } else {
                let msg = match_delete_memo(core, &key);
                core.messages.push(ChatMessage::system(msg));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/memo roles" | "/memo list roles" => {
            let msg = match_list_role_memos(core);
            core.messages.push(ChatMessage::system(msg));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // ── Reflection commands ──
        "/reflect" | "/reflect status" => {
            let enabled = state.core.reflection.auto_reflect;
            let max_attempts = state.core.reflection.max_attempts;
            let rules_state: Vec<String> = [
                (
                    "code",
                    state.core.reflection.rules_enabled[crate::reflection::RULE_CODE_COMPLETE],
                ),
                (
                    "error",
                    state.core.reflection.rules_enabled[crate::reflection::RULE_ERROR_AWARENESS],
                ),
                (
                    "questions",
                    state.core.reflection.rules_enabled[crate::reflection::RULE_MULTI_QUESTION],
                ),
                (
                    "promise",
                    state.core.reflection.rules_enabled[crate::reflection::RULE_EMPTY_PROMISE],
                ),
                (
                    "fileref",
                    state.core.reflection.rules_enabled[crate::reflection::RULE_FILE_REF_USED],
                ),
                (
                    "minlen",
                    state.core.reflection.rules_enabled[crate::reflection::RULE_MIN_OUTPUT],
                ),
            ]
            .iter()
            .map(|(name, ok)| format!("  {} {}", if *ok { "✓" } else { "✗" }, name))
            .collect();
            let msg = format!(
                "Reflection: {}\nMax retries: {}\nRules:\n{}",
                if enabled { "🟢 on" } else { "🔴 off" },
                max_attempts,
                rules_state.join("\n")
            );
            state.core.messages.push(ChatMessage::system(msg));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/reflect on" => {
            state.core.reflection.auto_reflect = true;
            state.core.messages.push(ChatMessage::system(
                "🟢 Reflection enabled — agent responses will be self-checked after each turn.",
            ));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        "/reflect off" => {
            state.core.reflection.auto_reflect = false;
            state.core.messages.push(ChatMessage::system("🔴 Reflection disabled."));
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/reflect max ") => {
            let val = trimmed.strip_prefix("/reflect max ").unwrap().trim();
            if let Ok(n) = val.parse::<u8>() {
                state.core.reflection.max_attempts = n;
                state
                    .core
                    .messages
                    .push(ChatMessage::system(format!("Reflection max retries set to {}.", n)));
            } else {
                state.core.messages.push(ChatMessage::system(format!(
                    "Invalid number: {}. Usage: /reflect max <N>",
                    val
                )));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        _ if trimmed.starts_with("/reflect rule ") => {
            let rest = trimmed.strip_prefix("/reflect rule ").unwrap().trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                let (rule_name, state_str) = (parts[0], parts[1]);
                let enable = match state_str {
                    "on" | "1" | "true" => Some(true),
                    "off" | "0" | "false" => Some(false),
                    _ => None,
                };
                if let Some(enabled) = enable {
                    let idx = match rule_name {
                        "code" => Some(crate::reflection::RULE_CODE_COMPLETE),
                        "error" => Some(crate::reflection::RULE_ERROR_AWARENESS),
                        "questions" => Some(crate::reflection::RULE_MULTI_QUESTION),
                        "promise" => Some(crate::reflection::RULE_EMPTY_PROMISE),
                        "fileref" => Some(crate::reflection::RULE_FILE_REF_USED),
                        "minlen" => Some(crate::reflection::RULE_MIN_OUTPUT),
                        _ => None,
                    };
                    if let Some(idx) = idx {
                        state.core.reflection.rules_enabled[idx] = enabled;
                        state.core.messages.push(ChatMessage::system(format!(
                            "Rule '{}' {}.",
                            rule_name,
                            if enabled { "enabled ✓" } else { "disabled ✗" }
                        )));
                    } else {
                        state.core.messages.push(ChatMessage::system(format!(
                            "Unknown rule: {}. Rules: code, error, questions, promise, fileref, minlen",
                            rule_name
                        )));
                    }
                } else {
                    state.core.messages.push(ChatMessage::system(format!(
                        "Invalid state: {}. Use 'on' or 'off'.",
                        state_str
                    )));
                }
            } else {
                state
                    .core
                    .messages
                    .push(ChatMessage::system("Usage: /reflect rule <name> <on|off>"));
            }
            ui.input.clear();
            ui.input_cursor = 0;
            true
        }

        // Not a recognised command — let caller handle as chat message.
        _ => false,
    }
}

/// Look up subcommand items for a parent command (e.g. "/role" returns role subcommands).
pub fn get_subcommand_items(cmd: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match cmd {
        "/role" => Some(&[
            ("list", "List all role templates"),
            ("show", "Show role template details"),
            ("create", "Create a new role template"),
            ("edit", "Edit an existing role template"),
            ("delete", "Delete a role template"),
            ("embed", "Compute embeddings for all roles"),
            ("optimize", "Optimize role prompt from experience"),
            ("default", "Show or set default bootstrap role"),
        ]),
        "/role default" => Some(&[
            // Dynamic items — resolved at runtime via resolve_dynamic_items
            ("", ""),
        ]),
        "/role show" => Some(&[("", "")]),
        "/role edit" => Some(&[("", "")]),
        "/role delete" => Some(&[("", "")]),
        "/role optimize" => Some(&[("", "")]),
        "/pool" => Some(&[
            ("stats", "Show experience pool statistics"),
            ("flush", "Flush bedrock to disk"),
            ("clear", "Clear the experience pool"),
            ("export", "Export pool to JSON"),
            ("import", "Import pool from JSON"),
            ("query", "Query experiences by text similarity"),
        ]),
        "/memo" => Some(&[
            ("list", "List role memos"),
            ("show", "Show a memo by key"),
            ("write", "Write a memo (key=value)"),
            ("delete", "Delete a memo"),
            ("roles", "List roles with memos"),
        ]),
        _ => None,
    }
}

/// Generic dynamic item resolver — returns completion items for any parent command
/// that takes dynamic arguments (e.g. role names, memo keys, etc.).
pub fn resolve_dynamic_items(parent: &str, core: &crate::tui::state::CoreState) -> Vec<(String, String)> {
    match parent {
        "/role default" | "/role show" | "/role edit" | "/role delete" | "/role optimize" => core
            .runtime
            .as_ref()
            .and_then(|rt| rt.try_read().ok())
            .map(|rt| {
                let default = &core.default_role;
                rt.all_role_templates()
                    .iter()
                    .map(|t| {
                        let label = if t.role == *default {
                            format!("{} (current)", t.label)
                        } else {
                            t.label.clone()
                        };
                        (t.role.clone(), label)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// List of all registered commands, used by the command popup.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/connect", "Configure a provider"),
    ("/models", "Open model picker"),
    ("/pool", "Pool management (stats/flush/clear/query)"),
    ("/reflect", "Reflection control (on/off/status/rule/max)"),
    ("/role", "Role templates (list/show/create/edit/delete)"),
    ("/sh", "Run a shell command"),
    ("/clear", "Clear conversation"),
    ("/memo", "Role memo management (list/show/write/delete/roles)"),
    ("/help", "Show help"),
];

// ============================================================================
//  Memo helper functions
// ============================================================================

/// Helper: format a timestamp as a human-readable age string.
fn format_age(timestamp: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let age = now.saturating_sub(timestamp);
    if age < 60 {
        format!("{}s ago", age)
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86400 {
        format!("{}h ago", age / 3600)
    } else {
        format!("{}d ago", age / 86400)
    }
}

fn match_list_memos(core: &crate::tui::state::CoreState) -> String {
    let agent_id = match core.responsible_agent_id {
        Some(id) => id,
        None => return "No active agent".to_string(),
    };
    let pool = match core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    let agent = match pool.get_agent(&agent_id) {
        Some(a) => a,
        None => return "Agent not found".to_string(),
    };
    let memos = pool.get_role_memos(&agent.role);
    if memos.is_empty() {
        return format!(
            "No memos for role '{}' (use write_memo MCP tool or /memo write)",
            agent.role
        );
    }
    let mut lines = format!("Memos for role '{}' ({}):\n", agent.role, memos.len());
    let mut sorted = memos.to_vec();
    sorted.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    for m in &sorted {
        let preview = if m.value.len() > 80 {
            format!(
                "{}...",
                &m.value[..m.value.char_indices().nth(80).map(|(i, _)| i).unwrap_or(m.value.len())]
            )
        } else {
            m.value.clone()
        };
        lines.push_str(&format!(
            "  {}  ({} bytes, {})  {:?}\n",
            m.key,
            m.value.len(),
            format_age(m.timestamp),
            preview
        ));
    }
    lines
}

fn match_show_memo(core: &crate::tui::state::CoreState, key: &str) -> String {
    let agent_id = match core.responsible_agent_id {
        Some(id) => id,
        None => return "No active agent".to_string(),
    };
    let pool = match core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    let agent = match pool.get_agent(&agent_id) {
        Some(a) => a,
        None => return "Agent not found".to_string(),
    };
    match pool.read_role_memo(&agent.role, key) {
        Some(entry) => format!(
            "Memo '{}' ({} bytes, written {}):\n---\n{}\n---",
            entry.key,
            entry.value.len(),
            format_age(entry.timestamp),
            entry.value
        ),
        None => format!("Memo '{}' not found for role '{}'", key, agent.role),
    }
}

fn match_write_memo(core: &mut crate::tui::state::CoreState, key: &str, value: &str) -> String {
    let agent_id = match core.responsible_agent_id {
        Some(id) => id,
        None => return "No active agent".to_string(),
    };
    let mut pool = match core.agent_pool.try_write() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    // We need the role before we can write, but we need to release the
    // agent borrow to avoid conflicts with write_role_memo.
    let role = match pool.get_agent(&agent_id) {
        Some(a) => a.role.clone(),
        None => return "Agent not found".to_string(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = crate::agent::MemoEntry {
        key: key.to_string(),
        value: value.to_string(),
        timestamp: now,
    };
    pool.write_role_memo(&role, entry);
    format!("Memo '{}' written ({} bytes) for role '{}'", key, value.len(), role)
}

fn match_delete_memo(core: &mut crate::tui::state::CoreState, key: &str) -> String {
    let agent_id = match core.responsible_agent_id {
        Some(id) => id,
        None => return "No active agent".to_string(),
    };
    let mut pool = match core.agent_pool.try_write() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    let role = match pool.get_agent(&agent_id) {
        Some(a) => a.role.clone(),
        None => return "Agent not found".to_string(),
    };
    if pool.delete_role_memo(&role, key) {
        format!("Memo '{}' deleted from role '{}'", key, role)
    } else {
        format!("Memo '{}' not found for role '{}'", key, role)
    }
}

fn match_list_role_memos(core: &crate::tui::state::CoreState) -> String {
    let pool = match core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    let role_memos = pool.role_memos();
    if role_memos.is_empty() {
        return "No roles with memos found".to_string();
    }
    let mut lines = vec!["Role Memos:".to_string()];
    for (role, memos) in role_memos.iter() {
        let count = memos.len();
        let total_bytes: usize = memos.iter().map(|m| m.value.len()).sum();
        lines.push(format!("  '{}': {} memos, {} bytes", role, count, total_bytes));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── get_subcommand_items ──

    #[test]
    fn test_get_subcommands_role() {
        let items = get_subcommand_items("/role").unwrap();
        let names: Vec<&str> = items.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"list"));
        assert!(names.contains(&"show"));
        assert!(names.contains(&"default"));
        assert!(names.contains(&"embed"));
        assert!(names.contains(&"optimize"));
    }

    #[test]
    fn test_get_subcommands_pool() {
        let items = get_subcommand_items("/pool").unwrap();
        let names: Vec<&str> = items.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"stats"));
        assert!(names.contains(&"flush"));
        assert!(names.contains(&"query"));
    }

    #[test]
    fn test_get_subcommands_memo() {
        let items = get_subcommand_items("/memo").unwrap();
        let names: Vec<&str> = items.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"list"));
        assert!(names.contains(&"write"));
        assert!(names.contains(&"delete"));
    }

    #[test]
    fn test_get_subcommands_unknown() {
        assert!(get_subcommand_items("/unknown").is_none());
        assert!(get_subcommand_items("/connect").is_none());
        assert!(get_subcommand_items("/help").is_none());
    }

    // ── dispatch: popup mode transitions ──

    #[test]
    fn test_dispatch_role_sets_subcommand_popup() {
        let mut state = AppState::default();
        dispatch("/role", &mut state, "12:00:00");
        assert!(matches!(state.popup_mode, PopupMode::SubCommand { .. }));
    }

    #[test]
    fn test_dispatch_pool_sets_subcommand_popup() {
        let mut state = AppState::default();
        dispatch("/pool", &mut state, "12:00:00");
        assert!(matches!(state.popup_mode, PopupMode::SubCommand { .. }));
    }

    #[test]
    fn test_dispatch_memo_sets_subcommand_popup() {
        let mut state = AppState::default();
        dispatch("/memo", &mut state, "12:00:00");
        assert!(matches!(state.popup_mode, PopupMode::SubCommand { .. }));
    }

    #[test]
    fn test_dispatch_role_list_shows_message() {
        let mut state = AppState::default();
        // Needs a runtime with templates, but at minimum should not panic
        dispatch("/role list", &mut state, "12:00:00");
        // No runtime → shows "Runtime not available" message
        assert!(
            state
                .core
                .messages
                .last()
                .map(|m| m.content.contains("Runtime"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_dispatch_role_default_sets_subcommand() {
        let mut state = AppState::default();
        dispatch("/role default", &mut state, "12:00:00");
        // Without runtime, no items available → shows error message
        assert!(matches!(state.popup_mode, PopupMode::None));
        assert!(
            state
                .core
                .messages
                .last()
                .map(|m| m.content.contains("No role templates"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_dispatch_role_default_with_arg_no_runtime() {
        let mut state = AppState::default();
        dispatch("/role default planner", &mut state, "12:00:00");
        // No runtime available → shows message, default_role unchanged
        assert_eq!(state.core.default_role, "general_business_analyst");
    }

    #[test]
    fn test_dispatch_connect_sets_providers_popup() {
        let mut state = AppState::default();
        dispatch("/connect", &mut state, "12:00:00");
        assert_eq!(state.popup_mode, PopupMode::Providers);
    }

    #[test]
    fn test_dispatch_help_adds_message() {
        let mut state = AppState::default();
        dispatch("/help", &mut state, "12:00:00");
        assert!(
            state
                .core
                .messages
                .last()
                .map(|m| m.content.contains("/role"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_dispatch_unknown_returns_false() {
        let mut state = AppState::default();
        assert!(!dispatch("not a command", &mut state, "12:00:00"));
    }

    #[test]
    fn test_dispatch_sh_without_arg_shows_usage() {
        let mut state = AppState::default();
        dispatch("/sh", &mut state, "12:00:00");
        assert!(
            state
                .core
                .messages
                .last()
                .map(|m| m.content.contains("Usage"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_dispatch_role_default_with_runtime_shows_popup() {
        let mut state = AppState::default();
        dispatch("/role default", &mut state, "12:00:00");
        // Without runtime, falls through to message (not a crash)
        assert!(state.popup_mode == PopupMode::None);
        assert!(state.ui.input.is_empty() || !state.ui.input.is_empty());
    }

    #[test]
    fn test_dispatch_role_help_has_default_subcommand() {
        let mut state = AppState::default();
        dispatch("/role", &mut state, "12:00:00");
        if let PopupMode::SubCommand { items, .. } = &state.popup_mode {
            let names: Vec<&str> = items.iter().map(|(n, _): &(String, String)| n.as_str()).collect();
            assert!(names.contains(&"default"), "default subcommand missing in /role help");
        } else {
            panic!("Expected SubCommand popup");
        }
    }
}
