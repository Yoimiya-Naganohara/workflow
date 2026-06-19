//! Slash command dispatch — node-based tree with macros.
//!
//! Commands are organized as a tree of structs implementing the
//! [`node::Node`] trait. The [`CommandStack`] manages navigation
//! through the tree as the user types.

pub mod dispatch;
pub mod node;

pub use dispatch::{display, get_current_commands, select};
pub use node::{CommandStack, Node};

// ═══════════════════════════════════════════════════════════════
//  Command macros
// ═══════════════════════════════════════════════════════════════

/// Define a leaf command node that executes directly.
///
/// # Example
/// ```ignore
/// leaf_node!(HelpNode, "help", "Show help", |args, state, now| {
///     push_msg(state, "Help text".to_string());
///     true
/// });
/// ```
macro_rules! leaf_node {
    ($name:ident, $path:expr, $desc:expr, $handler:expr) => {
        pub struct $name;

        impl $crate::tui::commands::node::Node for $name {
            fn name(&self) -> &str {
                $path
            }
            fn desc(&self) -> &str {
                $desc
            }
            fn execute(
                &self,
                args: &[String],
                state: &mut $crate::tui::state::AppState,
                now: &str,
            ) -> bool {
                let handler = $handler;
                handler(args, state, now)
            }
        }
    };
}

/// Define a branch command node with static children.
///
/// # Example
/// ```ignore
/// branch_node!(PoolGroup, "pool", "Pool management", [
///     PoolStatsNode,
///     PoolFlushNode,
///     PoolClearNode,
///     PoolQueryNode,
/// ]);
/// ```
macro_rules! branch_node {
    ($name:ident, $path:expr, $desc:expr, [$($child:ident),* $(,)?]) => {
        pub struct $name;

        impl $crate::tui::commands::node::Node for $name {
            fn name(&self) -> &str { $path }
            fn desc(&self) -> &str { $desc }
            fn children(&self) -> Vec<Box<dyn $crate::tui::commands::node::Node>> {
                vec![
                    $(Box::new($child),)*
                ]
            }
        }
    };
}

/// Define a branch command node with dynamic children.
///
/// # Example
/// ```ignore
/// dynamic_branch_node!(RoleShowNode, "show", "Show role details", load_role_names);
/// ```
macro_rules! dynamic_branch_node {
    ($name:ident, $path:expr, $desc:expr, $loader:expr) => {
        pub struct $name;

        impl $crate::tui::commands::node::Node for $name {
            fn name(&self) -> &str {
                $path
            }
            fn desc(&self) -> &str {
                $desc
            }
            fn children(&self) -> Vec<Box<dyn $crate::tui::commands::node::Node>> {
                ($loader)()
            }
        }
    };
}

// ═══════════════════════════════════════════════════════════════
//  Helper functions
// ═══════════════════════════════════════════════════════════════

fn push_msg(state: &mut crate::tui::state::AppState, text: String) {
    state
        .core
        .messages
        .push(crate::tui::state::ChatMessage::system(text));
}

fn try_read_runtime(
    core: &crate::tui::state::CoreState,
) -> Result<tokio::sync::RwLockReadGuard<'_, crate::runtime::AgentRuntime>, String> {
    let rt = core
        .runtime
        .as_ref()
        .ok_or_else(|| "Runtime not available".to_string())?;
    rt.try_read().map_err(|_| "Runtime locked".to_string())
}

// ═══════════════════════════════════════════════════════════════
//  Simple leaf nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    ConnectNode,
    "connect",
    "Configure a provider",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        state.popup_mode = crate::tui::state::PopupMode::Providers;
        state.popup_selected = 0;
        state
            .effects
            .push(crate::tui::effect::Effect::FetchModelRegistry);
        true
    }
);

leaf_node!(
    ModelsNode,
    "models",
    "Open model picker",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        if state.core.configured_providers.is_empty() {
            push_msg(
                state,
                "No providers configured. Use `/connect` first.".to_string(),
            );
        } else {
            state.popup_mode = crate::tui::state::PopupMode::ModelPicker;
            state.popup_selected = 0;
            push_msg(state, "Select models to add to your pool".to_string());
        }
        true
    }
);

leaf_node!(
    HelpNode,
    "help",
    "Show help",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let help_text = crate::tui::commands::COMMANDS
            .iter()
            .map(|(cmd, desc)| format!("{:20} {}", cmd, desc))
            .collect::<Vec<_>>()
            .join("\n");
        push_msg(state, help_text);
        true
    }
);

leaf_node!(
    StatusNode,
    "status",
    "Show system status",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let msg = format!(
            "System Status:\n  Providers: {}\n  Models: {}",
            state.core.configured_providers.len(),
            state.core.selected_models.len()
        );
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    ClearNode,
    "clear",
    "Clear conversation",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        state.core.messages.clear();
        state
            .core
            .messages
            .push(crate::tui::state::ChatMessage::system(
                "Workflow Agent — conversation cleared.",
            ));
        state.core.responsible_agent_id = None;
        state.core.agents.clear();
        state.ui.input.clear();
        state.ui.input_cursor = 0;
        state.ui.chat_scroll = 0;
        state.ui.cached_system_prompt = None;
        state.ui.cached_prompt_role.clear();
        true
    }
);

leaf_node!(
    RefreshNode,
    "refresh",
    "Refresh prompt cache",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        state.ui.cached_system_prompt = None;
        state.ui.cached_prompt_role.clear();
        push_msg(state, "System prompt cache cleared.".to_string());
        true
    }
);

leaf_node!(
    KeymapNode,
    "keymap",
    "Show keyboard shortcuts",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let bindings = state.keymap.all_bindings();
        let mut lines = vec!["Keyboard Shortcuts:".to_string(), String::new()];
        for (key, action) in &bindings {
            lines.push(format!(
                "  {:20} {}",
                key,
                crate::tui::keymap::format_action(action)
            ));
        }
        push_msg(state, lines.join("\n"));
        true
    }
);

leaf_node!(
    ThinkNode,
    "think",
    "Set reasoning level",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let level = args
            .first()
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or((state.ui.think_level + 1) % 3);
        let lvl = level.min(2);
        state.ui.think_level = lvl;
        let labels = ["hidden", "brief", "full"];
        push_msg(
            state,
            format!(
                "Reasoning display set to: {} ({})",
                labels[lvl as usize], lvl
            ),
        );
        true
    }
);

leaf_node!(
    ShellNode,
    "sh",
    "Run shell command",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let cmd = args.join(" ");
        if cmd.is_empty() {
            state.popup_mode = crate::tui::state::PopupMode::ShellInput {
                cmd: "/sh".to_string(),
                input: String::new(),
            };
            state.popup_selected = 0;
        } else {
            state
                .effects
                .push(crate::tui::effect::Effect::ExecuteShell { command: cmd });
        }
        true
    }
);

// ═══════════════════════════════════════════════════════════════
//  Role nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    RoleListNode,
    "list",
    "List all role templates",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let msg = match try_read_runtime(&state.core) {
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
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    RoleCreateNode,
    "create",
    "Create a new role",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        push_msg(
            state,
            "Role creation — edit role templates in ~/.workflow/role_templates.json".to_string(),
        );
        true
    }
);

leaf_node!(
    RoleEmbedNode,
    "embed",
    "Compute embeddings",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let msg = match try_read_runtime(&state.core) {
            Ok(rt) => {
                let n = rt.all_role_templates().len();
                rt.compute_role_embeddings_async();
                format!("Computing embeddings for {} role template(s)...", n)
            }
            Err(e) => e,
        };
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    RoleShowLeaf,
    "show",
    "Show role details",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let name = args.first().map(|s| s.as_str()).unwrap_or("");
        let msg = match try_read_runtime(&state.core) {
            Ok(rt) => match rt.get_role_template(name) {
                Some(t) => {
                    let embedded = if t.embedding.is_some() { "yes" } else { "no" };
                    format!(
                        "Role: {}\n  Label:    {}\n  ID:       {}\n  Embedded: {}\n  Prompt ({}):\n{}\n{}\n{}",
                        t.role,
                        t.label,
                        t.template_id,
                        embedded,
                        t.system_prompt.len(),
                        "─".repeat(36),
                        t.system_prompt,
                        "─".repeat(36)
                    )
                }
                None => format!("Role '{}' not found.", name),
            },
            Err(e) => e,
        };
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    RoleEditLeaf,
    "edit",
    "Edit role",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let name = args.first().map(|s| s.as_str()).unwrap_or("");
        let msg = match try_read_runtime(&state.core) {
            Ok(rt) => match rt.get_role_template(name) {
                Some(t) => format!(
                    "Role '{}' found. Edit in ~/.workflow/role_templates.json",
                    t.role
                ),
                None => format!("Role '{}' not found.", name),
            },
            Err(e) => e,
        };
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    RoleDeleteLeaf,
    "delete",
    "Delete role",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let name = args.first().map(|s| s.as_str()).unwrap_or("");
        let msg = match try_read_runtime(&state.core) {
            Ok(rt) => match rt.get_role_template(name) {
                Some(t) => {
                    rt.delete_role_template(t.template_id);
                    format!("Role '{}' deleted.", name)
                }
                None => format!("Role '{}' not found.", name),
            },
            Err(e) => e,
        };
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    RoleOptimizeLeaf,
    "optimize",
    "Optimize role prompt",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let name = args.first().map(|s| s.as_str()).unwrap_or("");
        let msg = format!("Optimizing role '{}'...", name);
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    RoleDefaultLeaf,
    "default",
    "Set default role",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let name = args.first().map(|s| s.as_str()).unwrap_or("");
        state.core.default_role = name.to_string();
        state.core.responsible_agent_id = None;
        state.core.agents.clear();
        push_msg(
            state,
            format!(
                "Default bootstrap role set to `{}`. Next chat message will use this role.",
                name
            ),
        );
        true
    }
);

branch_node!(
    RoleGroup,
    "role",
    "Role templates",
    [
        RoleListNode,
        RoleShowBranch,
        RoleCreateNode,
        RoleEditBranch,
        RoleDeleteBranch,
        RoleEmbedNode,
        RoleOptimizeBranch,
        RoleDefaultBranch,
    ]
);

dynamic_branch_node!(RoleShowBranch, "show", "Show role details", load_role_names);
dynamic_branch_node!(RoleEditBranch, "edit", "Edit role", load_role_names);
dynamic_branch_node!(RoleDeleteBranch, "delete", "Delete role", load_role_names);
dynamic_branch_node!(
    RoleOptimizeBranch,
    "optimize",
    "Optimize role prompt",
    load_role_names
);
dynamic_branch_node!(
    RoleDefaultBranch,
    "default",
    "Set default role",
    load_role_names
);

// ═══════════════════════════════════════════════════════════════
//  Pool nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    PoolStatsNode,
    "stats",
    "Show pool statistics",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let msg = match try_read_runtime(&state.core) {
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
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    PoolFlushNode,
    "flush",
    "Flush bedrock to disk",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let msg = match try_read_runtime(&state.core) {
            Ok(rt) => match rt.flush_experience_pool() {
                Ok(()) => "Experience pool flushed to disk".to_string(),
                Err(e) => format!("Flush failed: {}", e),
            },
            Err(e) => e,
        };
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    PoolClearNode,
    "clear",
    "Clear experience pool",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let msg = match try_read_runtime(&state.core) {
            Ok(rt) => match rt.clear_experience_pool() {
                Ok(()) => "Experience pool cleared".to_string(),
                Err(e) => format!("Clear failed: {}", e),
            },
            Err(e) => e,
        };
        push_msg(state, msg);
        true
    }
);

leaf_node!(
    PoolQueryNode,
    "query",
    "Query experiences",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let query = args.join(" ");
        if query.is_empty() {
            push_msg(state, "Usage: /pool query <text>".to_string());
        } else {
            push_msg(state, format!("Querying pool for: {}", query));
        }
        true
    }
);

branch_node!(
    PoolGroup,
    "pool",
    "Pool management",
    [PoolStatsNode, PoolFlushNode, PoolClearNode, PoolQueryNode,]
);

// ═══════════════════════════════════════════════════════════════
//  Memo nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    MemoListNode,
    "list",
    "List role memos",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        push_msg(state, "Memo list — use /memo list".to_string());
        true
    }
);

leaf_node!(
    MemoShowNode,
    "show",
    "Show memo by key",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let key = args.first().map(|s| s.as_str()).unwrap_or("");
        push_msg(state, format!("Showing memo: {}", key));
        true
    }
);

leaf_node!(
    MemoWriteNode,
    "write",
    "Write memo (key=value)",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let rest = args.join(" ");
        if let Some((key, value)) = rest.split_once('=') {
            push_msg(
                state,
                format!("Writing memo: {} = {}", key.trim(), value.trim()),
            );
        } else {
            push_msg(state, "Usage: /memo write <key>=<value>".to_string());
        }
        true
    }
);

leaf_node!(
    MemoDeleteNode,
    "delete",
    "Delete memo",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let key = args.first().map(|s| s.as_str()).unwrap_or("");
        push_msg(state, format!("Deleting memo: {}", key));
        true
    }
);

leaf_node!(
    MemoRolesNode,
    "roles",
    "List roles with memos",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        push_msg(state, "Roles with memos".to_string());
        true
    }
);

branch_node!(
    MemoGroup,
    "memo",
    "Role memos",
    [
        MemoListNode,
        MemoShowNode,
        MemoWriteNode,
        MemoDeleteNode,
        MemoRolesNode,
    ]
);

// ═══════════════════════════════════════════════════════════════
//  Agent nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    AgentListNode,
    "list",
    "List all agents",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        push_msg(state, "Agent list".to_string());
        true
    }
);

leaf_node!(
    AgentInspectNode,
    "inspect",
    "Inspect agent",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let id = args.first().map(|s| s.as_str()).unwrap_or("");
        push_msg(state, format!("Inspecting agent: {}", id));
        true
    }
);

branch_node!(
    AgentGroup,
    "agent",
    "Agent management",
    [AgentListNode, AgentInspectNode,]
);

// ═══════════════════════════════════════════════════════════════
//  Reflect nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    ReflectStatusNode,
    "status",
    "Show reflection status",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let enabled = state.core.reflection.auto_reflect;
        let max_attempts = state.core.reflection.max_attempts;
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
            let icon = if state.core.reflection.rules_enabled[*idx] {
                "✓"
            } else {
                "✗"
            };
            format!("  {} {}", icon, name)
        })
        .collect();
        push_msg(
            state,
            format!(
                "Reflection: {}\nMax retries: {}\nRules:\n{}",
                if enabled { "🟢 on" } else { "🔴 off" },
                max_attempts,
                rules_state.join("\n")
            ),
        );
        true
    }
);

leaf_node!(
    ReflectOnNode,
    "on",
    "Enable reflection",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        state.core.reflection.auto_reflect = true;
        push_msg(state, "🟢 Reflection enabled.".to_string());
        true
    }
);

leaf_node!(
    ReflectOffNode,
    "off",
    "Disable reflection",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        state.core.reflection.auto_reflect = false;
        push_msg(state, "🔴 Reflection disabled.".to_string());
        true
    }
);

leaf_node!(
    ReflectMaxNode,
    "max",
    "Set max retries",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let val = args.first().map(|s| s.as_str()).unwrap_or("1");
        match val.parse::<u8>() {
            Ok(n) => {
                state.core.reflection.max_attempts = n;
                push_msg(state, format!("Reflection max retries set to {}.", n));
            }
            Err(_) => {
                push_msg(
                    state,
                    format!("Invalid number: {}. Usage: /reflect max <N>", val),
                );
            }
        }
        true
    }
);

leaf_node!(
    ReflectRuleNode,
    "rule",
    "Toggle a rule",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let rule_name = args.first().map(|s| s.as_str()).unwrap_or("");
        let enabled = args
            .get(1)
            .map(|s| matches!(s.as_str(), "on" | "1" | "true"))
            .unwrap_or(true);

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
            push_msg(
                state,
                format!(
                    "Rule '{}' {}.",
                    rule_name,
                    if enabled {
                        "enabled ✓"
                    } else {
                        "disabled ✗"
                    }
                ),
            );
        } else {
            push_msg(
                state,
                format!(
                    "Unknown rule: {}. Rules: code, error, questions, promise, fileref, minlen",
                    rule_name
                ),
            );
        }
        true
    }
);

branch_node!(
    ReflectGroup,
    "reflect",
    "Reflection control",
    [
        ReflectStatusNode,
        ReflectOnNode,
        ReflectOffNode,
        ReflectMaxNode,
        ReflectRuleNode,
    ]
);

// ═══════════════════════════════════════════════════════════════
//  Session nodes
// ═══════════════════════════════════════════════════════════════

leaf_node!(
    SessionListNode,
    "list",
    "List saved sessions",
    |_args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let sessions = crate::persistence::list_sessions();
        if sessions.is_empty() {
            push_msg(state, "No saved sessions.".to_string());
        } else {
            let mut lines = vec!["Sessions:".to_string()];
            for name in &sessions {
                lines.push(format!("  {}", name));
            }
            push_msg(state, lines.join("\n"));
        }
        true
    }
);

leaf_node!(
    SessionSwitchLeaf,
    "switch",
    "Switch session",
    |args: &[String], state: &mut crate::tui::state::AppState, _now: &str| {
        let name = args.first().map(|s| s.as_str()).unwrap_or("");
        let count = crate::persistence::load_session_as(name)
            .map(|m| m.len())
            .unwrap_or(0);
        if count == 0 {
            push_msg(state, format!("Session '{}' not found.", name));
        } else {
            crate::tui::controller::switch_session(&mut state.core, &mut state.ui, name);
            state.ui.auto_scroll = true;
            push_msg(
                state,
                format!("🔄 Switched to session '{}' ({} messages).", name, count),
            );
        }
        true
    }
);

branch_node!(
    SessionGroup,
    "sessions",
    "Session management",
    [SessionListNode, SessionSwitchBranch,]
);

dynamic_branch_node!(
    SessionSwitchBranch,
    "switch",
    "Switch session",
    load_session_names
);

// ═══════════════════════════════════════════════════════════════
//  Dynamic role node
// ═══════════════════════════════════════════════════════════════

pub struct DynamicRoleNode {
    pub role_name: String,
    pub context: String,
}

impl Node for DynamicRoleNode {
    fn name(&self) -> &str {
        &self.role_name
    }
    fn desc(&self) -> &str {
        "Role"
    }
    fn execute(
        &self,
        _args: &[String],
        state: &mut crate::tui::state::AppState,
        _now: &str,
    ) -> bool {
        match self.context.as_str() {
            "show" => {
                let msg = match try_read_runtime(&state.core) {
                    Ok(rt) => match rt.get_role_template(&self.role_name) {
                        Some(t) => {
                            let embedded = if t.embedding.is_some() { "yes" } else { "no" };
                            format!(
                                "Role: {}\n  Label:    {}\n  ID:       {}\n  Embedded: {}\n  Prompt ({}):\n{}\n{}\n{}",
                                t.role,
                                t.label,
                                t.template_id,
                                embedded,
                                t.system_prompt.len(),
                                "─".repeat(36),
                                t.system_prompt,
                                "─".repeat(36)
                            )
                        }
                        None => format!("Role '{}' not found.", self.role_name),
                    },
                    Err(e) => e,
                };
                push_msg(state, msg);
            }
            "edit" => {
                let msg = match try_read_runtime(&state.core) {
                    Ok(rt) => match rt.get_role_template(&self.role_name) {
                        Some(t) => format!(
                            "Role '{}' found. Edit in ~/.workflow/role_templates.json",
                            t.role
                        ),
                        None => format!("Role '{}' not found.", self.role_name),
                    },
                    Err(e) => e,
                };
                push_msg(state, msg);
            }
            "delete" => {
                let msg = match try_read_runtime(&state.core) {
                    Ok(rt) => match rt.get_role_template(&self.role_name) {
                        Some(t) => {
                            rt.delete_role_template(t.template_id);
                            format!("Role '{}' deleted.", self.role_name)
                        }
                        None => format!("Role '{}' not found.", self.role_name),
                    },
                    Err(e) => e,
                };
                push_msg(state, msg);
            }
            "optimize" => {
                push_msg(state, format!("Optimizing role '{}'...", self.role_name));
            }
            "default" => {
                state.core.default_role = self.role_name.clone();
                state.core.responsible_agent_id = None;
                state.core.agents.clear();
                push_msg(
                    state,
                    format!(
                        "Default bootstrap role set to `{}`. Next chat message will use this role.",
                        self.role_name
                    ),
                );
            }
            _ => {}
        }
        true
    }
}

// ═══════════════════════════════════════════════════════════════
//  Dynamic loaders
// ═══════════════════════════════════════════════════════════════

fn load_role_names() -> Vec<Box<dyn Node>> {
    vec![] // 实际加载在 dispatch 中处理
}

fn load_session_names() -> Vec<Box<dyn Node>> {
    vec![] // 实际加载在 dispatch 中处理
}

// ═══════════════════════════════════════════════════════════════
//  Root node
// ═══════════════════════════════════════════════════════════════

pub struct RootNode;

impl Node for RootNode {
    fn name(&self) -> &str {
        "/"
    }
    fn desc(&self) -> &str {
        "root"
    }
    fn children(&self) -> Vec<Box<dyn Node>> {
        vec![
            Box::new(ConnectNode),
            Box::new(ModelsNode),
            Box::new(RoleGroup),
            Box::new(PoolGroup),
            Box::new(MemoGroup),
            Box::new(AgentGroup),
            Box::new(ReflectGroup),
            Box::new(SessionGroup),
            Box::new(ShellNode),
            Box::new(HelpNode),
            Box::new(StatusNode),
            Box::new(ClearNode),
            Box::new(RefreshNode),
            Box::new(KeymapNode),
            Box::new(ThinkNode),
        ]
    }
}

// ═══════════════════════════════════════════════════════════════
//  Autocompletion support
// ═══════════════════════════════════════════════════════════════

/// Static command list for autocompletion (name, description).
pub const COMMANDS: &[(&str, &str)] = &[
    ("/connect", "Configure a provider"),
    ("/models", "Open model picker"),
    ("/pool", "Pool management"),
    ("/role", "Role templates"),
    ("/agent", "Agent management"),
    ("/memo", "Role memos"),
    ("/reflect", "Reflection control"),
    ("/sessions", "Session management"),
    ("/sh", "Run shell command"),
    ("/status", "Show system status"),
    ("/clear", "Clear conversation"),
    ("/refresh", "Refresh prompt cache"),
    ("/keymap", "Show keyboard shortcuts"),
    ("/think", "Set reasoning level"),
    ("/help", "Show help"),
];

/// Legacy dispatch function.
pub use dispatch::dispatch;

/// Resolve dynamic items for popup completion.
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
        _ => vec![],
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_has_children() {
        let root = RootNode;
        let children = root.children();
        assert!(!children.is_empty());
        assert!(children.iter().any(|n| n.name() == "role"));
        assert!(children.iter().any(|n| n.name() == "pool"));
    }

    #[test]
    fn test_role_group_has_children() {
        let role = RoleGroup;
        let children = role.children();
        assert!(!children.is_empty());
        assert!(children.iter().any(|n| n.name() == "list"));
        assert!(children.iter().any(|n| n.name() == "show"));
    }

    #[test]
    fn test_leaf_has_no_children() {
        let list = RoleListNode;
        assert!(list.children().is_empty());
    }

    #[test]
    fn test_dynamic_role_show() {
        let show = RoleShowBranch;
        // Dynamic children (empty in test)
        assert!(show.children().is_empty());
    }
}
