//! Command handlers — all business logic for tree commands.

use crate::tui::command_tree::{AppState, CommandInvocation, CommandResult, UiEffect};
use crate::tui::effect::Effect;
use crate::tui::state::{ChatMessage, MessageRole, PopupMode};

// ── /help ──

pub fn help(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let help_text = crate::tui::commands::COMMANDS
        .iter()
        .map(|(cmd, desc)| format!("  {:20} {}", cmd, desc))
        .collect::<Vec<_>>()
        .join("\n");
    state.core.messages.push(ChatMessage::system(help_text));
    CommandResult::handled()
}

// ── /status ──

pub fn status(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let core = &state.core;
    let lines = [
        format!("Providers:       {}", core.configured_providers.len()),
        format!("Selected models: {}", core.selected_models.len()),
        format!("Active agents:   {}", core.agents.len()),
        format!("Default role:    {}", core.default_role),
        format!(
            "Reflection:      {}",
            if core.reflection.auto_reflect {
                "on"
            } else {
                "off"
            }
        ),
        format!(
            "Messages:        {}",
            core.messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count()
        ),
    ];
    state.core.messages.push(ChatMessage::system(format!(
        "System Status:\n{}",
        lines.join("\n")
    )));
    CommandResult::handled()
}

// ── /clear ──

pub fn clear(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.core.messages.clear();
    state.core.messages.push(ChatMessage::system(
        "Workflow Agent — conversation cleared. Describe your goal and I'll help.",
    ));
    state.core.responsible_agent_id = None;
    state.core.agents.clear();
    state.ui.input.clear();
    state.ui.input_cursor = 0;
    state.ui.chat_scroll = 0;
    state.ui.input_history.clear();
    state.ui.input_history_idx = None;
    state.ui.active_chat_abort = None;
    state.ui.active_chat_requests = 0;
    state.ui.cached_system_prompt = None;
    state.ui.cached_prompt_role.clear();
    CommandResult::handled()
}

// ── /sh ──

pub fn shell(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let cmd = inv.args.join(" ");
    if cmd.is_empty() {
        state.popup_mode = PopupMode::ShellInput {
            cmd: "/sh".to_string(),
            input: String::new(),
        };
        state.popup_selected = 0;
    } else {
        state
            .core
            .messages
            .push(ChatMessage::system(format!("$ {}", cmd)));
        state.effects.push(Effect::ExecuteShell { command: cmd });
    }
    CommandResult::handled()
}

// ── /models ──

pub fn models(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    if state.core.configured_providers.is_empty() {
        state.core.messages.push(ChatMessage::system(
            "No providers configured. Use `/connect` first.",
        ));
    } else {
        state.popup_mode = PopupMode::ModelPicker;
        state.popup_selected = 0;
        state
            .core
            .messages
            .push(ChatMessage::system("Select models to add to your pool"));
    }
    CommandResult::handled()
}

// ── /connect ──

pub fn connect(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.ui.input.clear();
    state.ui.input_cursor = 0;
    state.popup_mode = PopupMode::Providers;
    state.popup_selected = 0;
    state.effects.push(Effect::FetchModelRegistry);
    CommandResult::handled()
}

// ── /refresh ──

pub fn refresh(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.ui.cached_system_prompt = None;
    state.ui.cached_prompt_role.clear();
    state.core.messages.push(ChatMessage::system(
        "System prompt cache cleared. Next message will use current memos.",
    ));
    CommandResult::handled()
}

// ── /think ──

pub fn think_set(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let setting = inv
        .args
        .first()
        .or_else(|| inv.command_path.last())
        .map(|s| s.as_str());

    // Check reasoning effort keywords first.
    if let Some(s) = setting {
        let effort = match s {
            "low" | "min" => Some("low"),
            "medium" | "med" | "mid" => Some("medium"),
            "high" | "max" | "deep" => Some("high"),
            _ => None,
        };
        if let Some(e) = effort {
            state.ui.reasoning_effort = Some(e.to_string());
            state.ui.think_level = 2;
            // Look up the current model's reasoning_options from api.json
            // and store them in the pool for dynamic parameter building.
            let opts = state.core.selected_models.first().and_then(|sel| {
                state
                    .core
                    .models
                    .get_model(&sel.provider_id, &sel.model_id)
                    .map(|m| m.reasoning_options.clone())
            });
            if let Ok(mut pool) = state.core.agent_pool.try_write() {
                pool.reasoning_effort = Some(e.to_string());
                pool.reasoning_options = opts.unwrap_or_default();
            }
            state.core.messages.push(ChatMessage::system(format!(
                "Reasoning effort: {}, display: full",
                e
            )));
            return CommandResult::handled();
        }
    }

    let level = setting
        .and_then(|s| match s {
            "on" | "full" | "2" => Some(2u8),
            "brief" | "1" => Some(1),
            "off" | "hidden" | "0" => Some(0),
            _ => None,
        })
        .unwrap_or((state.ui.think_level + 1) % 3)
        .min(2);
    state.ui.think_level = level;
    if level == 0 {
        // Hiding reasoning also disables reasoning effort.
        state.ui.reasoning_effort = None;
        if let Ok(mut pool) = state.core.agent_pool.try_write() {
            pool.reasoning_effort = None;
            pool.reasoning_options = vec![];
        }
    }
    state.core.messages.push(ChatMessage::system(format!(
        "Reasoning display set to: {} ({})",
        ["hidden", "brief", "full"][level as usize],
        level
    )));
    CommandResult::handled()
}

pub fn think_status(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let effort = state
        .ui
        .reasoning_effort
        .as_ref()
        .map(|e| format!(", effort: {}", e))
        .unwrap_or_default();
    state.core.messages.push(ChatMessage::system(format!(
        "Reasoning display: {} {}",
        ["hidden (0)", "brief (1)", "full (2)"][state.ui.think_level as usize],
        effort
    )));
    CommandResult::handled()
}

// ── /pool ──

fn with_runtime<F, R>(state: &mut AppState, f: F) -> Result<R, String>
where
    F: FnOnce(&tokio::sync::RwLockReadGuard<crate::runtime::AgentRuntime>) -> Result<R, String>,
{
    let runtime = state
        .core
        .runtime
        .as_ref()
        .ok_or_else(|| "Runtime not available".to_string())?;
    f(&runtime
        .try_read()
        .map_err(|_| "Runtime locked".to_string())?)
}

pub fn pool_stats(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let msg = with_runtime(state, |rt| Ok(format!(
        "Experience Pool Statistics:\n\x20 Total entries:     {}\n\x20 Bedrock (A-track): {}\n\x20 Fluid  (B-track):  {}\n\x20 Pending suspend:   {}\n\x20 Remaining budget:  {}\n\x20 Available permits: {}",
        rt.experience_count(), rt.bedrock_count(), rt.fluid_count(), rt.pending_suspended(), rt.remaining_budget(), rt.available_permits()
    ))).unwrap_or_else(|e| e);
    state.core.messages.push(ChatMessage::system(msg));
    CommandResult::handled()
}

pub fn pool_flush(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let msg = match with_runtime(state, |rt| {
        rt.flush_experience_pool()
            .map_err(|e| format!("Flush failed: {}", e))
    }) {
        Ok(()) => "Experience pool flushed to disk".into(),
        Err(e) => e,
    };
    state.core.messages.push(ChatMessage::system(msg));
    CommandResult::handled()
}

pub fn pool_clear(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let msg = match with_runtime(state, |rt| {
        rt.clear_experience_pool()
            .map_err(|e| format!("Clear failed: {}", e))
    }) {
        Ok(()) => "Experience pool cleared".into(),
        Err(e) => e,
    };
    state.core.messages.push(ChatMessage::system(msg));
    CommandResult::handled()
}

pub fn pool_export(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.core.messages.push(ChatMessage::system(
        "Export not yet implemented. Pool file is at ~/.workflow/experience_a.bin",
    ));
    CommandResult::handled()
}

pub fn pool_import(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state
        .core
        .messages
        .push(ChatMessage::system("Import not yet implemented"));
    CommandResult::handled()
}

pub fn pool_query(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let query_text = inv.args.join(" ");
    if query_text.is_empty() {
        return CommandResult::handled().with_effect(UiEffect::OpenPopup(PopupMode::ShellInput {
            cmd: "/pool query".to_string(),
            input: String::new(),
        }));
    }
    if let Some(runtime) = state.core.runtime.clone() {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        state.effects.push(Effect::PoolQuery {
            query_text,
            runtime,
            now,
        });
    }
    CommandResult::handled()
}

// ── /sessions ──

pub fn session_switch(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let name = inv
        .args
        .first()
        .or_else(|| inv.command_path.last())
        .map(|s| s.as_str());
    let Some(name) = name else {
        return CommandResult::error("Usage: sessions switch <name>");
    };
    let count = crate::persistence::load_session_as(name)
        .map(|m| m.len())
        .unwrap_or(0);
    if count == 0 {
        state.core.messages.push(ChatMessage::system(format!(
            "Session '{}' not found.",
            name
        )));
    } else {
        crate::tui::controller::switch_session(&mut state.core, &mut state.ui, name);
        state.ui.auto_scroll = true;
        state.core.messages.push(ChatMessage::system(format!(
            "Switched to session '{}' ({} messages).",
            name, count
        )));
    }
    CommandResult::handled()
}

// ── /memo ──

fn current_agent_role(state: &AppState) -> Option<String> {
    let agent_id = state.core.responsible_agent_id?;
    let pool = state.core.agent_pool.try_read().ok()?;
    Some(pool.get_agent(&agent_id)?.role.clone())
}

pub fn memo_show(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let key = inv
        .args
        .first()
        .or_else(|| inv.command_path.last())
        .map(|s| s.as_str());
    let Some(key) = key else {
        return CommandResult::error("Usage: memo show <key>");
    };
    let Some(role) = current_agent_role(state) else {
        state
            .core
            .messages
            .push(ChatMessage::system("No active agent"));
        return CommandResult::handled();
    };
    let pool = match state.core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => {
            state
                .core
                .messages
                .push(ChatMessage::system("Agent pool locked"));
            return CommandResult::handled();
        }
    };
    match pool.read_role_memo(&role, key) {
        Some(entry) => state.core.messages.push(ChatMessage::system(format!(
            "Memo '{}' ({} bytes):\n---\n{}\n---",
            entry.key,
            entry.value.len(),
            entry.value
        ))),
        None => state.core.messages.push(ChatMessage::system(format!(
            "Memo '{}' not found for role '{}'",
            key, role
        ))),
    }
    CommandResult::handled()
}

pub fn memo_write(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let args_str = inv.args.join(" ");
    if args_str.is_empty() {
        return CommandResult::handled().with_effect(UiEffect::OpenPopup(PopupMode::ShellInput {
            cmd: "/memo write".to_string(),
            input: String::new(),
        }));
    }
    let Some((key, value)) = args_str.split_once('=') else {
        state
            .core
            .messages
            .push(ChatMessage::system("Usage: memo write <key>=<value>"));
        return CommandResult::handled();
    };
    let (key, value) = (key.trim(), value.trim());
    if key.is_empty() {
        state
            .core
            .messages
            .push(ChatMessage::system("Usage: memo write <key>=<value>"));
        return CommandResult::handled();
    }
    let Some(role) = current_agent_role(state) else {
        state
            .core
            .messages
            .push(ChatMessage::system("No active agent"));
        return CommandResult::handled();
    };
    let mut pool = match state.core.agent_pool.try_write() {
        Ok(p) => p,
        Err(_) => {
            state
                .core
                .messages
                .push(ChatMessage::system("Agent pool locked"));
            return CommandResult::handled();
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    pool.write_role_memo(
        &role,
        crate::agent::MemoEntry {
            key: key.to_string(),
            value: value.to_string(),
            timestamp: now,
        },
    );
    state.core.messages.push(ChatMessage::system(format!(
        "Memo '{}' written ({} bytes) for role '{}'",
        key,
        value.len(),
        role
    )));
    CommandResult::handled()
}

pub fn memo_delete(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let key = inv
        .args
        .first()
        .or_else(|| inv.command_path.last())
        .map(|s| s.as_str());
    let Some(key) = key else {
        return CommandResult::error("Usage: memo delete <key>");
    };
    let Some(role) = current_agent_role(state) else {
        state
            .core
            .messages
            .push(ChatMessage::system("No active agent"));
        return CommandResult::handled();
    };
    let mut pool = match state.core.agent_pool.try_write() {
        Ok(p) => p,
        Err(_) => {
            state
                .core
                .messages
                .push(ChatMessage::system("Agent pool locked"));
            return CommandResult::handled();
        }
    };
    if pool.delete_role_memo(&role, key) {
        state.core.messages.push(ChatMessage::system(format!(
            "Memo '{}' deleted from role '{}'",
            key, role
        )));
    } else {
        state.core.messages.push(ChatMessage::system(format!(
            "Memo '{}' not found for role '{}'",
            key, role
        )));
    }
    CommandResult::handled()
}

pub fn memo_roles(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let pool = match state.core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => {
            state
                .core
                .messages
                .push(ChatMessage::system("Agent pool locked"));
            return CommandResult::handled();
        }
    };
    let role_memos = pool.role_memos();
    if role_memos.is_empty() {
        state
            .core
            .messages
            .push(ChatMessage::system("No roles with memos found"));
        return CommandResult::handled();
    }
    let lines = role_memos
        .iter()
        .map(|(role, memos)| {
            format!(
                "  '{}': {} memos, {} bytes",
                role,
                memos.len(),
                memos.iter().map(|m| m.value.len()).sum::<usize>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    state
        .core
        .messages
        .push(ChatMessage::system(format!("Role Memos:\n{}", lines)));
    CommandResult::handled()
}

// ── /role ──

fn role_arg(inv: &CommandInvocation) -> Option<&str> {
    inv.args
        .first()
        .map(|s| s.as_str())
        .or_else(|| inv.command_path.last().map(|s| s.as_str()))
}

pub fn role_create(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.core.messages.push(ChatMessage::system(
        "Role creation — edit role templates in ~/.workflow/role_templates.json",
    ));
    CommandResult::handled()
}

pub fn role_embed(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let n = (|| -> Option<usize> {
        let runtime = state.core.runtime.as_ref()?;
        let guard = runtime.try_read().ok()?;
        let n = guard.all_role_templates().len();
        guard.compute_role_embeddings_async();
        Some(n)
    })();
    match n {
        Some(count) => state.core.messages.push(ChatMessage::system(format!(
            "Computing embeddings for {} role template(s)...",
            count
        ))),
        None => state
            .core
            .messages
            .push(ChatMessage::system(if state.core.runtime.is_some() {
                "Runtime locked"
            } else {
                "Runtime not available"
            })),
    }
    CommandResult::handled()
}

pub fn role_show(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let Some(role_id) = role_arg(inv) else {
        return CommandResult::error("Usage: role show <name>");
    };
    let msg = (|| -> Option<String> {
        let runtime = state.core.runtime.as_ref()?;
        let guard = runtime.try_read().ok()?;
        let t = guard.get_role_template(role_id)?;
        Some(format!(
            "Role: {}\n  Label:        {}\n  ID:           {}\n  Embedded:     {}\n  Prompt ({}):\n{}\n{}\n{}",
            t.role,
            t.label,
            t.template_id,
            if t.embedding.is_some() { "yes" } else { "no" },
            t.system_prompt.len(),
            "─".repeat(36),
            t.system_prompt,
            "─".repeat(36)
        ))
    })();
    let msg = msg.unwrap_or_else(|| {
        if state.core.runtime.is_none() {
            "Runtime not available".to_string()
        } else if let Some(rt) = state.core.runtime.as_ref() {
            if rt.try_read().is_err() {
                "Runtime locked".to_string()
            } else {
                format!("Role '{}' not found.", role_id)
            }
        } else {
            "Runtime not available".to_string()
        }
    });
    state.core.messages.push(ChatMessage::system(msg));
    CommandResult::handled()
}

pub fn role_default(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let Some(role_id) = role_arg(inv) else {
        return CommandResult::error("Usage: role default <name>");
    };
    state.core.default_role = role_id.to_string();
    state.core.responsible_agent_id = None;
    state.core.agents.clear();
    state.core.messages.push(ChatMessage::system(format!(
        "Default bootstrap role set to `{}`. Next chat message will use this role.",
        role_id
    )));
    CommandResult::handled()
}

pub fn role_delete(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let Some(role_id) = role_arg(inv) else {
        return CommandResult::error("Usage: role delete <name>");
    };
    let result = (|| -> Result<(), String> {
        let runtime = state
            .core
            .runtime
            .as_ref()
            .ok_or_else(|| "Runtime not available".to_string())?;
        let guard = runtime
            .try_read()
            .map_err(|_| "Runtime locked".to_string())?;
        let t = guard
            .get_role_template(role_id)
            .ok_or_else(|| format!("Role '{}' not found.", role_id))?;
        guard.delete_role_template(t.template_id);
        Ok(())
    })();
    match result {
        Ok(()) => state
            .core
            .messages
            .push(ChatMessage::system(format!("Role '{}' deleted.", role_id))),
        Err(e) => state.core.messages.push(ChatMessage::system(e)),
    }
    CommandResult::handled()
}

// ── /agent ──

pub fn agent_inspect(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let id_str = inv
        .args
        .first()
        .or_else(|| inv.command_path.last())
        .map(|s| s.as_str())
        .unwrap_or("");
    if id_str.is_empty() {
        return CommandResult::error("Usage: agent inspect <id>");
    }
    let pool = match state.core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => {
            state
                .core
                .messages
                .push(ChatMessage::system("Agent pool locked"));
            return CommandResult::handled();
        }
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
            state.core.messages.push(ChatMessage::system(format!("Agent: {}\n  ID:       {}\n  Role:     {}\n  Status:   {:?}\n  Depth:    {}\n  Goal:     {}\n  Parent:   {}\n  Children: {}{}", a.name, id_full, a.role, a.status, a.depth, a.goal,
                a.parent_id.map(|id| crate::agent::AgentPool::agent_id_str(&id)).unwrap_or_else(|| "root".to_string()), a.children.len(), trace_block)));
        }
        None => state.core.messages.push(ChatMessage::system(format!(
            "Agent '{}' not found.",
            id_str
        ))),
    }
    CommandResult::handled()
}

// ── /reflect ──

pub fn reflect_on(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.core.reflection.auto_reflect = true;
    state.core.messages.push(ChatMessage::system(
        "Reflection enabled — agent responses will be self-checked after each turn.",
    ));
    CommandResult::handled()
}

pub fn reflect_off(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    state.core.reflection.auto_reflect = false;
    state
        .core
        .messages
        .push(ChatMessage::system("Reflection disabled."));
    CommandResult::handled()
}

pub fn reflect_status(_inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let enabled = state.core.reflection.auto_reflect;
    let rules: Vec<String> = [
        ("code", 0u8),
        ("error", 1),
        ("questions", 2),
        ("promise", 3),
        ("fileref", 4),
        ("minlen", 5),
    ]
    .iter()
    .map(|(name, i)| {
        format!(
            "  {} {}",
            if state.core.reflection.rules_enabled[*i as usize] {
                "✓"
            } else {
                "✗"
            },
            name
        )
    })
    .collect();
    state.core.messages.push(ChatMessage::system(format!(
        "Reflection: {}\nMax retries: {}\nRules:\n{}",
        if enabled { "on" } else { "off" },
        state.core.reflection.max_attempts,
        rules.join("\n")
    )));
    CommandResult::handled()
}

pub fn reflect_max(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    let Some(val_str) = inv.args.first() else {
        state
            .core
            .messages
            .push(ChatMessage::system("Usage: /reflect max <N>"));
        return CommandResult::handled();
    };
    match val_str.parse::<u8>() {
        Ok(n) => {
            state.core.reflection.max_attempts = n;
            state.core.messages.push(ChatMessage::system(format!(
                "Reflection max retries set to {}.",
                n
            )));
        }
        Err(_) => state.core.messages.push(ChatMessage::system(format!(
            "Invalid number: {}. Usage: /reflect max <N>",
            val_str
        ))),
    }
    CommandResult::handled()
}

pub fn reflect_rule(inv: &CommandInvocation, state: &mut AppState) -> CommandResult {
    if inv.args.len() < 2 {
        state
            .core
            .messages
            .push(ChatMessage::system("Usage: /reflect rule <name> <on|off>"));
        return CommandResult::handled();
    }
    let enable = match inv.args[1].as_str() {
        "on" | "1" | "true" => Some(true),
        "off" | "0" | "false" => Some(false),
        _ => None,
    };
    let Some(enable) = enable else {
        state.core.messages.push(ChatMessage::system(format!(
            "Invalid state: {}. Use 'on' or 'off'.",
            inv.args[1]
        )));
        return CommandResult::handled();
    };
    let idx = match inv.args[0].as_str() {
        "code" => Some(crate::reflection::RULE_CODE_COMPLETE),
        "error" => Some(crate::reflection::RULE_ERROR_AWARENESS),
        "questions" => Some(crate::reflection::RULE_MULTI_QUESTION),
        "promise" => Some(crate::reflection::RULE_EMPTY_PROMISE),
        "fileref" => Some(crate::reflection::RULE_FILE_REF_USED),
        "minlen" => Some(crate::reflection::RULE_MIN_OUTPUT),
        _ => None,
    };
    let Some(idx) = idx else {
        state.core.messages.push(ChatMessage::system(format!(
            "Unknown rule: {}. Rules: code, error, questions, promise, fileref, minlen",
            inv.args[0]
        )));
        return CommandResult::handled();
    };
    state.core.reflection.rules_enabled[idx] = enable;
    state.core.messages.push(ChatMessage::system(format!(
        "Rule '{}' {}.",
        inv.args[0],
        if enable {
            "enabled ✓"
        } else {
            "disabled ✗"
        }
    )));
    CommandResult::handled()
}
