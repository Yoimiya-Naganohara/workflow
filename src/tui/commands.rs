//! Slash command dispatch table.
//!
//! Maps user-input commands (``/connect``, ``/models``, ``/sh``, etc.)
//! to their implementations.  Synchronous operations are handled
//! inline; async operations push an [`Effect`] onto the state's
//! effect queue.

use crate::tui::effect::Effect;
use crate::tui::state::{AppState, ChatMessage, MessageRole, MessageStatus, PopupMode};

// ═══════════════════════════════════════════════════════════════
//  Helper macros and functions
// ═══════════════════════════════════════════════════════════════

/// Clear input and return true (command processed).
macro_rules! done {
    ($state:expr) => {{
        $state.ui.input.clear();
        $state.ui.input_cursor = 0;
        true
    }};
}

/// Push a system message and return true.
macro_rules! msg {
    ($state:expr, $text:expr) => {{
        $state.core.messages.push(ChatMessage::system($text));
        done!($state)
    }};
}

/// Push a styled message (with status) and return true.
macro_rules! msg_styled {
    ($state:expr, $text:expr, $status:expr, $now:expr) => {{
        $state.core.messages.push(ChatMessage {
            role: MessageRole::System,
            content: $text.to_string(),
            reasoning: String::new(),
            timestamp: $now.to_string(),
            status: $status,
        });
        done!($state)
    }};
}

/// Get a read handle to the runtime, returning an error message on failure.
fn try_read_runtime(
    core: &crate::tui::state::CoreState,
) -> Result<
    tokio::sync::RwLockReadGuard<'_, crate::runtime::AgentRuntime>,
    String,
> {
    let rt = core
        .runtime
        .as_ref()
        .ok_or_else(|| "Runtime not available".to_string())?;
    rt.try_read()
        .map_err(|_| "Runtime locked".to_string())
}

// ═══════════════════════════════════════════════════════════════
//  Command dispatch
// ═══════════════════════════════════════════════════════════════

/// Try to dispatch a command.  Returns ``true`` if the input was a
/// recognised command (even if it failed), ``false`` if it should
/// be treated as a normal chat message.
pub fn dispatch(trimmed: &str, state: &mut AppState, now: &str) -> bool {
    let core = &mut state.core;
    let ui = &mut state.ui;

    match trimmed {
        // ── Provider / Model commands ──────────────────────────────

        "/connect" => {
            ui.input.clear();
            ui.input_cursor = 0;
            state.popup_mode = PopupMode::Providers;
            state.popup_selected = 0;
            state.effects.push(Effect::FetchModelRegistry);
            true
        }

        "/models" | "/model" | "/m" => {
            if core.configured_providers.is_empty() {
                return msg!(
                    state,
                    "No providers configured. Use `/connect` first."
                );
            }
            state.popup_mode = PopupMode::ModelPicker;
            state.popup_selected = 0;
            msg!(state, "Select models to add to your pool")
        }

        // ── Shell commands ─────────────────────────────────────────

        "/sh" => {
            state.popup_mode = PopupMode::ShellInput {
                cmd: "/sh".to_string(),
                input: String::new(),
            };
            state.popup_selected = 0;
            true
        }

        _ if trimmed.starts_with("/sh ") => {
            let arg = trimmed.strip_prefix("/sh ").unwrap_or("").trim();
            if arg.is_empty() {
                return msg!(state, "Usage: /sh <command>");
            }
            core.messages.push(ChatMessage::system(format!("$ {}", arg)));
            state.effects.push(Effect::ExecuteShell {
                command: arg.to_string(),
            });
            ui.input.clear();
            ui.input_cursor = 0;
            ui.chat_scroll = 0;
            true
        }

        // ── Pool commands ──────────────────────────────────────────

        "/pool" | "/pool help" => {
            state.popup_mode = PopupMode::SubCommand {
                parent: "/pool".to_string(),
                items: get_subcommand_items("/pool")
                    .unwrap_or(&[])
                    .iter()
                    .map(|(n, d)| (n.to_string(), d.to_string()))
                    .collect(),
            };
            state.popup_selected = 0;
            done!(state)
        }

        "/pool stats" => {
            let msg = match try_read_runtime(core) {
                Ok(rt) => format!(
                    "Experience Pool Statistics:\n\
                     \x20 Total entries:     {}\n\
                     \x20 Bedrock (A-track): {}\n\
                     \x20 Fluid  (B-track):  {}\n\
                     \x20 Pending suspend:   {}\n\
                     \x20 Remaining budget:  {}\n\
                     \x20 Available permits: {}",
                    rt.experience_count(),
                    rt.bedrock_count(),
                    rt.fluid_count(),
                    rt.pending_suspended(),
                    rt.remaining_budget(),
                    rt.available_permits()
                ),
                Err(e) => e,
            };
            msg!(state, msg)
        }

        "/pool flush" => {
            let (msg, is_err) = match try_read_runtime(core) {
                Ok(rt) => match rt.flush_experience_pool() {
                    Ok(()) => ("Experience pool flushed to disk".to_string(), false),
                    Err(e) => (format!("Flush failed: {}", e), true),
                },
                Err(e) => (e, true),
            };
            let status = if is_err {
                MessageStatus::Error
            } else {
                MessageStatus::Completed
            };
            msg_styled!(state, msg, status, now)
        }

        "/pool clear" => {
            let (msg, is_err) = match try_read_runtime(core) {
                Ok(rt) => match rt.clear_experience_pool() {
                    Ok(()) => ("Experience pool cleared".to_string(), false),
                    Err(e) => (format!("Clear failed: {}", e), true),
                },
                Err(e) => (e, true),
            };
            let status = if is_err {
                MessageStatus::Error
            } else {
                MessageStatus::Completed
            };
            msg_styled!(state, msg, status, now)
        }

        "/pool export" => {
            msg!(
                state,
                "Export not yet implemented. Pool file is at ~/.workflow/experience_a.bin"
            )
        }

        "/pool import" => {
            msg!(state, "Import not yet implemented")
        }

        _ if trimmed.starts_with("/pool query ") || trimmed.starts_with("/pool q ") => {
            let query_text = trimmed
                .split_once(' ')
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
            if query_text.is_empty() {
                state.popup_mode = PopupMode::ShellInput {
                    cmd: "/pool query".to_string(),
                    input: String::new(),
                };
                state.popup_selected = 0;
                return true;
            }
            if let Some(runtime) = core.runtime.clone() {
                state.effects.push(Effect::PoolQuery {
                    query_text,
                    runtime,
                    now: now.to_string(),
                });
            } else {
                return msg!(state, "Runtime not available for query");
            }
            done!(state)
        }

        _ if trimmed.starts_with("/pool ") => {
            let rest = trimmed.strip_prefix("/pool ").unwrap_or("").trim();
            msg!(state, format!("Unknown pool command: {}. Use /pool for help.", rest))
        }

        // ── Session commands ───────────────────────────────────────

        "/sessions" => {
            let items = resolve_dynamic_items("/sessions switch", core);
            if items.is_empty() {
                return msg!(
                    state,
                    "No saved sessions. Save a session via the agent or wait for auto-save on exit."
                );
            }
            state.popup_mode = PopupMode::SubCommand {
                parent: "/sessions switch".to_string(),
                items,
            };
            state.popup_selected = 0;
            done!(state)
        }

        _ if trimmed.starts_with("/sessions switch ") => {
            let name = trimmed.strip_prefix("/sessions switch ").unwrap_or("").trim();
            if name.is_empty() {
                return msg!(state, "Usage: /sessions switch <name>");
            }
            let count = crate::persistence::load_session_as(name)
                .map(|m| m.len())
                .unwrap_or(0);
            if count == 0 {
                return msg!(state, format!("Session '{}' not found.", name));
            }
            crate::tui::controller::switch_session(core, ui, name);
            ui.auto_scroll = true;
            msg!(state, format!("🔄 Switched to session '{}' ({} messages).", name, count))
        }

        // ── Role commands ──────────────────────────────────────────

        "/role" | "/role help" => {
            state.popup_mode = PopupMode::SubCommand {
                parent: "/role".to_string(),
                items: get_subcommand_items("/role")
                    .unwrap_or(&[])
                    .iter()
                    .map(|(n, d)| (n.to_string(), d.to_string()))
                    .collect(),
            };
            state.popup_selected = 0;
            done!(state)
        }

        "/role list" => {
            let msg = match try_read_runtime(core) {
                Ok(rt) => {
                    let templates = rt.all_role_templates();
                    if templates.is_empty() {
                        "No role templates found.".to_string()
                    } else {
                        let mut lines = vec!["Role Templates:".to_string()];
                        for t in &templates {
                            let embedded = if t.embedding.is_some() { "✓" } else { "✗" };
                            lines.push(format!(
                                "  id={:<3}  {:<30}  label={:<20}  embedded={}",
                                t.template_id, t.role, t.label, embedded
                            ));
                        }
                        lines.join("\n")
                    }
                }
                Err(e) => e,
            };
            msg!(state, msg)
        }

        "/role show" => {
            let items = resolve_dynamic_items("/role show", core);
            if items.is_empty() {
                return msg!(state, "No role templates available.");
            }
            state.popup_mode = PopupMode::SubCommand {
                parent: "/role show".to_string(),
                items,
            };
            state.popup_selected = 0;
            done!(state)
        }

        _ if trimmed.starts_with("/role show ") => {
            let role_name = trimmed.strip_prefix("/role show ").unwrap_or("").trim();
            if role_name.is_empty() {
                return msg!(state, "Usage: /role show <name>");
            }
            let msg = match try_read_runtime(core) {
                Ok(rt) => match rt.get_role_template(role_name) {
                    Some(t) => {
                        let embedded = if t.embedding.is_some() { "yes" } else { "no" };
                        format!(
                            "Role: {}\n  Label:        {}\n  ID:           {}\n  Embedded:     {}\n  Prompt ({}):\n{}\n{}\n{}",
                            t.role, t.label, t.template_id, embedded, t.system_prompt.len(),
                            "─".repeat(36), t.system_prompt, "─".repeat(36)
                        )
                    }
                    None => format!("Role '{}' not found. Use /role list to see available roles.", role_name),
                },
                Err(e) => e,
            };
            msg!(state, msg)
        }

        "/role create" => {
            msg!(
                state,
                "Role creation — edit role templates in ~/.workflow/role_templates.json"
            )
        }

        "/role edit" => {
            let items = resolve_dynamic_items("/role edit", core);
            if items.is_empty() {
                return msg!(state, "No role templates available.");
            }
            state.popup_mode = PopupMode::SubCommand {
                parent: "/role edit".to_string(),
                items,
            };
            state.popup_selected = 0;
            done!(state)
        }

        _ if trimmed.starts_with("/role edit ") => {
            let role_name = trimmed.strip_prefix("/role edit ").unwrap_or("").trim();
            if role_name.is_empty() {
                return msg!(state, "Usage: /role edit <name>");
            }
            let msg = match try_read_runtime(core) {
                Ok(rt) => match rt.get_role_template(role_name) {
                    Some(t) => format!("Role '{}' found. Edit in ~/.workflow/role_templates.json", t.role),
                    None => format!("Role '{}' not found. Use /role list to see available roles.", role_name),
                },
                Err(e) => e,
            };
            msg!(state, msg)
        }

        "/role embed" => {
            let msg = match try_read_runtime(core) {
                Ok(rt) => {
                    let n = rt.all_role_templates().len();
                    rt.compute_role_embeddings_async();
                    format!("Computing embeddings for {} role template(s)...", n)
                }
                Err(e) => e,
            };
            msg!(state, msg)
        }

        "/role optimize" => {
            let items = resolve_dynamic_items("/role optimize", core);
            if items.is_empty() {
                return msg!(state, "No role templates available.");
            }
            state.popup_mode = PopupMode::SubCommand {
                parent: "/role optimize".to_string(),
                items,
            };
            state.popup_selected = 0;
            done!(state)
        }

        _ if trimmed.starts_with("/role optimize ") => {
            let role_name = trimmed.strip_prefix("/role optimize ").unwrap_or("").trim();
            if role_name.is_empty() {
                return msg!(state, "Usage: /role optimize <name>");
            }

            // Validate first, then dispatch - avoids holding runtime guard across msg! macro
            let optimize_result: Result<usize, String> = (|| {
                let rt_guard = try_read_runtime(core)?;
                let role = rt_guard.get_role_template(role_name)
                    .ok_or_else(|| format!("Role '{}' not found.", role_name))?;
                let experiences = rt_guard.get_experiences_by_role(role.template_id);
                if experiences.len() < crate::runtime::optimizer::MIN_EXPERIENCES {
                    return Err(format!(
                        "Need at least {} experiences for '{}', have {}. Keep using the role to gather more data.",
                        crate::runtime::optimizer::MIN_EXPERIENCES, role_name, experiences.len()
                    ));
                }
                let tracker = rt_guard.optimization_tracker.lock()
                    .map_err(|_| "Tracker lock failed".to_string())?;
                if let Some(reason) = tracker.can_optimize(role.template_id, experiences.len()) {
                    return Err(format!("Cannot optimize '{}': {}", role_name, reason));
                }
                if rt_guard.provider.is_none() {
                    return Err("No LLM provider configured. Connect a provider first via /connect.".to_string());
                }
                Ok(experiences.len())
            })(); // rt_guard dropped here

            match optimize_result {
                Ok(exp_count) => {
                    if let Some(runtime) = core.runtime.clone() {
                        state.effects.push(Effect::OptimizeRole {
                            role_name: role_name.to_string(),
                            runtime,
                        });
                    }
                    msg!(state, format!("Optimizing role '{}' from {} experiences...", role_name, exp_count))
                }
                Err(e) => msg!(state, e),
            }
        }

        "/role delete" => {
            let items = resolve_dynamic_items("/role delete", core);
            if items.is_empty() {
                return msg!(state, "No role templates available.");
            }
            state.popup_mode = PopupMode::SubCommand {
                parent: "/role delete".to_string(),
                items,
            };
            state.popup_selected = 0;
            done!(state)
        }

        _ if trimmed.starts_with("/role delete ") => {
            let role_name = trimmed.strip_prefix("/role delete ").unwrap_or("").trim();
            if role_name.is_empty() {
                return msg!(state, "Usage: /role delete <name>");
            }
            let msg = match try_read_runtime(core) {
                Ok(rt) => match rt.get_role_template(role_name) {
                    Some(t) => {
                        rt.delete_role_template(t.template_id);
                        format!("Role '{}' deleted.", role_name)
                    }
                    None => format!("Role '{}' not found.", role_name),
                },
                Err(e) => e,
            };
            msg!(state, msg)
        }

        "/role default" => {
            let items = resolve_dynamic_items("/role default", core);
            if items.is_empty() {
                return msg!(state, "No role templates available or runtime not ready.");
            }
            state.popup_mode = PopupMode::SubCommand {
                parent: "/role default".to_string(),
                items,
            };
            state.popup_selected = 0;
            // Don't clear input — user can type to filter
            true
        }

        _ if trimmed.starts_with("/role default ") => {
            let role_name = trimmed.strip_prefix("/role default ").unwrap_or("").trim();
            if role_name.is_empty() {
                return msg!(state, "Usage: /role default <name>");
            }
            let found = try_read_runtime(core)
                .map(|rt| rt.get_role_template(role_name).is_some())
                .unwrap_or(false);
            if found {
                core.default_role = role_name.to_string();
                core.responsible_agent_id = None;
                core.agents.clear();
                msg!(state, format!(
                    "Default bootstrap role set to `{}`. Next chat message will use this role.",
                    role_name
                ))
            } else {
                msg!(state, format!(
                    "Role '{}' not found. Use `/role list` to see available roles.",
                    role_name
                ))
            }
        }

        // ── Agent commands ─────────────────────────────────────────

        "/agent" | "/agent help" => {
            state.popup_mode = PopupMode::SubCommand {
                parent: "/agent".to_string(),
                items: get_subcommand_items("/agent")
                    .unwrap_or(&[])
                    .iter()
                    .map(|(n, d)| (n.to_string(), d.to_string()))
                    .collect(),
            };
            state.popup_selected = 0;
            done!(state)
        }

        "/agent list" => {
            let msg = match_agent_list(core);
            msg!(state, msg)
        }

        _ if trimmed.starts_with("/agent inspect ") => {
            let id_str = trimmed.strip_prefix("/agent inspect ").unwrap_or("").trim();
            if id_str.is_empty() {
                return msg!(state, "Usage: /agent inspect <agent_id>");
            }
            let msg = match_agent_inspect(core, id_str);
            msg!(state, msg)
        }

        // ── Memo commands ──────────────────────────────────────────

        "/memo" | "/memo help" => {
            state.popup_mode = PopupMode::SubCommand {
                parent: "/memo".to_string(),
                items: get_subcommand_items("/memo")
                    .unwrap_or(&[])
                    .iter()
                    .map(|(n, d)| (n.to_string(), d.to_string()))
                    .collect(),
            };
            state.popup_selected = 0;
            done!(state)
        }

        "/memo list" => {
            let msg = match_list_memos(core);
            msg!(state, msg)
        }

        _ if trimmed.starts_with("/memo show ") => {
            let key = trimmed.strip_prefix("/memo show ").unwrap_or("").trim();
            if key.is_empty() {
                return msg!(state, "Usage: /memo show <key>");
            }
            let msg = match_show_memo(core, key);
            msg!(state, msg)
        }

        _ if trimmed.starts_with("/memo write ") => {
            let rest = trimmed.strip_prefix("/memo write ").unwrap_or("").trim();
            if let Some((key, value)) = rest.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                if key.is_empty() {
                    return msg!(state, "Usage: /memo write <key>=<value>");
                }
                let msg = match_write_memo(core, key, value);
                msg!(state, msg)
            } else {
                msg!(state, "Usage: /memo write <key>=<value>")
            }
        }

        _ if trimmed.starts_with("/memo delete ") => {
            let key = trimmed.strip_prefix("/memo delete ").unwrap_or("").trim();
            if key.is_empty() {
                return msg!(state, "Usage: /memo delete <key>");
            }
            let msg = match_delete_memo(core, key);
            msg!(state, msg)
        }

        "/memo roles" | "/memo list roles" => {
            let msg = match_list_role_memos(core);
            msg!(state, msg)
        }

        // ── Reflection commands ────────────────────────────────────

        "/reflect" | "/reflect status" => {
            let enabled = core.reflection.auto_reflect;
            let max_attempts = core.reflection.max_attempts;
            let rules_state: Vec<String> = [
                ("code", crate::reflection::RULE_CODE_COMPLETE),
                ("error", crate::reflection::RULE_ERROR_AWARENESS),
                ("questions", crate::reflection::RULE_MULTI_QUESTION),
                ("promise", crate::reflection::RULE_EMPTY_PROMISE),
                ("fileref", crate::reflection::RULE_FILE_REF_USED),
                ("minlen", crate::reflection::RULE_MIN_OUTPUT),
            ]
            .iter()
            .map(|(name, idx)| {
                let icon = if core.reflection.rules_enabled[*idx] {
                    "✓"
                } else {
                    "✗"
                };
                format!("  {} {}", icon, name)
            })
            .collect();
            msg!(state, format!(
                "Reflection: {}\nMax retries: {}\nRules:\n{}",
                if enabled { "🟢 on" } else { "🔴 off" },
                max_attempts,
                rules_state.join("\n")
            ))
        }

        "/reflect on" => {
            core.reflection.auto_reflect = true;
            msg!(
                state,
                "🟢 Reflection enabled — agent responses will be self-checked after each turn."
            )
        }

        "/reflect off" => {
            core.reflection.auto_reflect = false;
            msg!(state, "🔴 Reflection disabled.")
        }

        _ if trimmed.starts_with("/reflect max ") => {
            let val = trimmed.strip_prefix("/reflect max ").unwrap_or("").trim();
            match val.parse::<u8>() {
                Ok(n) => {
                    core.reflection.max_attempts = n;
                    msg!(state, format!("Reflection max retries set to {}.", n))
                }
                Err(_) => msg!(state, format!("Invalid number: {}. Usage: /reflect max <N>", val)),
            }
        }

        _ if trimmed.starts_with("/reflect rule ") => {
            let rest = trimmed.strip_prefix("/reflect rule ").unwrap_or("").trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() != 2 {
                return msg!(state, "Usage: /reflect rule <name> <on|off>");
            }
            let (rule_name, state_str) = (parts[0], parts[1]);
            let enable = match state_str {
                "on" | "1" | "true" => Some(true),
                "off" | "0" | "false" => Some(false),
                _ => None,
            };
            let enable = match enable {
                Some(e) => e,
                None => return msg!(state, format!("Invalid state: {}. Use 'on' or 'off'.", state_str)),
            };
            let idx = match rule_name {
                "code" => Some(crate::reflection::RULE_CODE_COMPLETE),
                "error" => Some(crate::reflection::RULE_ERROR_AWARENESS),
                "questions" => Some(crate::reflection::RULE_MULTI_QUESTION),
                "promise" => Some(crate::reflection::RULE_EMPTY_PROMISE),
                "fileref" => Some(crate::reflection::RULE_FILE_REF_USED),
                "minlen" => Some(crate::reflection::RULE_MIN_OUTPUT),
                _ => None,
            };
            let idx = match idx {
                Some(i) => i,
                None => {
                    return msg!(
                        state,
                        format!("Unknown rule: {}. Rules: code, error, questions, promise, fileref, minlen", rule_name)
                    );
                }
            };
            core.reflection.rules_enabled[idx] = enable;
            msg!(state, format!(
                "Rule '{}' {}.",
                rule_name,
                if enable { "enabled ✓" } else { "disabled ✗" }
            ))
        }

        // ── System commands ────────────────────────────────────────

        "/keymap" => {
            let bindings = state.keymap.all_bindings();
            let mut lines = vec!["Keyboard Shortcuts:".to_string(), String::new()];
            for (key, action) in &bindings {
                lines.push(format!(
                    "  {:20} {}",
                    key,
                    crate::tui::keymap::format_action(action)
                ));
            }
            msg!(state, lines.join("\n"))
        }

        "/help" | "/?" => {
            let help_text = COMMANDS
                .iter()
                .map(|(cmd, desc)| format!("{:20} {}", cmd, desc))
                .collect::<Vec<_>>()
                .join("\n");
            msg!(state, help_text)
        }

        "/think" => {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let level = parts
                .get(1)
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or((ui.think_level + 1) % 3);
            let lvl = level.min(2);
            ui.think_level = lvl;
            let labels = ["hidden", "brief", "full"];
            msg!(state, format!(
                "Reasoning display set to: {} ({})",
                labels[lvl as usize], lvl
            ))
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
            // Clear cached system prompt so next session rebuilds with current memos
            ui.cached_system_prompt = None;
            ui.cached_prompt_role.clear();
            true
        }

        // ── Status / Info ──────────────────────────────────────────

        "/status" | "/info" => {
            let provider_count = core.configured_providers.len();
            let model_count = core.selected_models.len();
            let agent_count = core.agents.len();
            let reflection = if core.reflection.auto_reflect {
                "🟢 on"
            } else {
                "🔴 off"
            };
            let role = &core.default_role;
            let lines = [
                format!("Providers:       {}", provider_count),
                format!("Selected models: {}", model_count),
                format!("Active agents:   {}", agent_count),
                format!("Default role:    {}", role),
                format!("Reflection:      {}", reflection),
                format!(
                    "Messages:        {}",
                    core.messages.iter().filter(|m| m.role == MessageRole::User).count()
                ),
            ];
            msg!(state, format!("System Status:\n{}", lines.join("\n")))
        }

        // ── Cache management ───────────────────────────────────────

        "/refresh" => {
            // Clear cached system prompt so next message rebuilds with current memos
            ui.cached_system_prompt = None;
            ui.cached_prompt_role.clear();
            msg!(state, "System prompt cache cleared. Next message will use current memos.")
        }

        // ── Not a recognised command ───────────────────────────────
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════════
//  Subcommand definitions
// ═══════════════════════════════════════════════════════════════

/// Look up subcommand items for a parent command.
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
        "/role default" | "/role show" | "/role edit" | "/role delete" | "/role optimize" => {
            Some(&[("", "")])
        }
        "/pool" => Some(&[
            ("stats", "Show experience pool statistics"),
            ("flush", "Flush bedrock to disk"),
            ("clear", "Clear the experience pool"),
            ("export", "Export pool to JSON"),
            ("import", "Import pool from JSON"),
            ("query", "Query experiences by text similarity"),
        ]),
        "/agent" => Some(&[
            ("list", "List all agents with status"),
            ("inspect", "Show agent detail by ID"),
        ]),
        "/sessions switch" => Some(&[("", "")]),
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

/// Generic dynamic item resolver for popup completion.
pub fn resolve_dynamic_items(
    parent: &str,
    core: &crate::tui::state::CoreState,
) -> Vec<(String, String)> {
    match parent {
        "/role default" | "/role show" | "/role edit" | "/role delete" | "/role optimize" => {
            let default = &core.default_role;
            match try_read_runtime(core) {
                Ok(rt) => rt
                    .all_role_templates()
                    .iter()
                    .map(|t| {
                        let label = if t.role == *default {
                            format!("{} (current)", t.label)
                        } else {
                            t.label.clone()
                        };
                        (t.role.clone(), label)
                    })
                    .collect(),
                Err(_) => vec![],
            }
        }
        "/sessions switch" => {
            let sessions = crate::persistence::list_sessions();
            sessions
                .into_iter()
                .map(|name| {
                    let count = crate::persistence::load_session_as(&name)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    (name.clone(), format!("{} messages ({})", count, name))
                })
                .collect()
        }
        "/agent inspect" => core
            .agent_pool
            .try_read()
            .map(|pool| {
                pool.agents()
                    .iter()
                    .map(|a| {
                        let id_short = crate::agent::AgentPool::agent_id_str(&a.id);
                        (id_short[..12].to_string(), format!("{} — {:?}", a.name, a.status))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

// ═══════════════════════════════════════════════════════════════
//  Command registry (for popup and help)
// ═══════════════════════════════════════════════════════════════

/// All registered commands for the command popup and auto-generated help.
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
    ("/refresh", "Refresh system prompt cache (apply memo changes)"),
    ("/sessions", "Switch to a saved session"),
    ("/memo", "Role memo management (list/show/write/delete/roles)"),
    ("/think", "Set reasoning display level (0/1/2)"),
    ("/help", "Show help"),
];

// ═══════════════════════════════════════════════════════════════
//  Helper functions
// ═══════════════════════════════════════════════════════════════

/// Format a timestamp as a human-readable age string.
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
            let end = m.value.char_indices().nth(80).map(|(i, _)| i).unwrap_or(m.value.len());
            format!("{}...", &m.value[..end])
        } else {
            m.value.clone()
        };
        lines.push_str(&format!(
            "  {}  ({} bytes, {})  {:?}\n",
            m.key, m.value.len(), format_age(m.timestamp), preview
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
            entry.key, entry.value.len(), format_age(entry.timestamp), entry.value
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
    format!(
        "Memo '{}' written ({} bytes) for role '{}'",
        key, value.len(), role
    )
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

// ── Agent helpers ─────────────────────────────────────────────

fn match_agent_list(core: &crate::tui::state::CoreState) -> String {
    let pool = match core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    let agents = pool.agents();
    if agents.is_empty() {
        return "No agents in pool.".to_string();
    }
    let mut lines = vec![format!("Agents ({}):", agents.len())];
    for agent in agents {
        let id_str = crate::agent::AgentPool::agent_id_str(&agent.id);
        let short = &id_str[..12];
        lines.push(format!(
            "  {:<12} {:<18} depth={}  {:?}",
            short, agent.name, agent.depth, agent.status
        ));
    }
    lines.join("\n")
}

fn match_agent_inspect(core: &crate::tui::state::CoreState, id_str: &str) -> String {
    let pool = match core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => return "Agent pool locked".to_string(),
    };
    let agent = pool.agents().iter().find(|a| {
        let full = crate::agent::AgentPool::agent_id_str(&a.id);
        full.starts_with(id_str) || full == id_str
    });
    match agent {
        Some(a) => {
            let id_full = crate::agent::AgentPool::agent_id_str(&a.id);
            let traces: Vec<String> = a
                .tool_trace
                .iter()
                .rev()
                .take(3)
                .map(|t| format!("      {} — {}", t.name, t.args_preview))
                .collect();
            let trace_block = if traces.is_empty() {
                String::new()
            } else {
                format!("\n  Tool trace (last 3):\n{}", traces.join("\n"))
            };
            format!(
                "Agent: {}\n  ID:       {}\n  Role:     {}\n  Status:   {:?}\n  Depth:    {}\n  Goal:     {}\n  Parent:   {}\n  Children: {}{}",
                a.name, id_full, a.role, a.status, a.depth, a.goal,
                a.parent_id
                    .map(|id| crate::agent::AgentPool::agent_id_str(&id))
                    .unwrap_or_else(|| "root".to_string()),
                a.children.len(), trace_block,
            )
        }
        None => format!("Agent '{}' not found.", id_str),
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

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
        dispatch("/role list", &mut state, "12:00:00");
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
    fn test_dispatch_sh_without_arg_opens_shell_input() {
        let mut state = AppState::default();
        dispatch("/sh", &mut state, "12:00:00");
        assert!(
            matches!(
                state.popup_mode,
                PopupMode::ShellInput { .. }
            ),
            "/sh without args should open ShellInput popup, got {:?}",
            state.popup_mode
        );
    }

    #[test]
    fn test_dispatch_status_shows_info() {
        let mut state = AppState::default();
        dispatch("/status", &mut state, "12:00:00");
        let msg = state.core.messages.last().unwrap();
        assert!(msg.content.contains("System Status"));
        assert!(msg.content.contains("Providers:"));
    }

    #[test]
    fn test_dispatch_model_alias() {
        let mut state = AppState::default();
        // No providers configured, should show error
        dispatch("/model", &mut state, "12:00:00");
        assert!(
            state
                .core
                .messages
                .last()
                .map(|m| m.content.contains("No providers"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_dispatch_help_auto_generated() {
        let mut state = AppState::default();
        dispatch("/help", &mut state, "12:00:00");
        let msg = state.core.messages.last().unwrap();
        // Help should contain all commands from COMMANDS array
        assert!(msg.content.contains("/connect"));
        assert!(msg.content.contains("/models"));
        assert!(msg.content.contains("/pool"));
        assert!(msg.content.contains("/role"));
        assert!(msg.content.contains("/status"));
    }
}
