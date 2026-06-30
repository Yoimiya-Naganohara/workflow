//! NodeProvider implementations — map data sources to tree nodes.

use crate::tui::command_tree::{CommandContext, Handler, Node, NodeKind};
use std::borrow::Cow;

/// Build role nodes with a specific handler.
fn build_role_nodes(ctx: &CommandContext, handler: Handler) -> Vec<Node> {
    let Some(runtime) = ctx.core.runtime.as_ref() else {
        return vec![];
    };
    let Ok(guard) = runtime.try_read() else {
        return vec![];
    };
    let templates = guard.all_role_templates();
    if templates.is_empty() {
        return vec![];
    }
    let cur = &ctx.core.default_role;
    templates
        .iter()
        .map(|t| Node {
            id: Cow::Owned(t.role.clone()),
            display: Cow::Owned(if t.role == *cur {
                format!("{} (current)", t.label)
            } else {
                t.label.clone()
            }),
            help: Cow::Owned(format!("Role #{}", t.template_id)),
            kind: NodeKind::Execute { handler },
        })
        .collect()
}

pub fn role_names_show_provider(ctx: &CommandContext) -> Vec<Node> {
    build_role_nodes(ctx, super::handlers::role_show)
}
pub fn role_names_default_provider(ctx: &CommandContext) -> Vec<Node> {
    build_role_nodes(ctx, super::handlers::role_default)
}
pub fn role_names_delete_provider(ctx: &CommandContext) -> Vec<Node> {
    build_role_nodes(ctx, super::handlers::role_delete)
}

use crate::node;
pub fn role_provider(_: &CommandContext) -> Vec<Node> {
    vec![
        node!(
            "show",
            "show",
            "Show role template detail",
            Branch(role_names_show_provider)
        ),
        node!(
            "default",
            "default",
            "Set default bootstrap role",
            Branch(role_names_default_provider)
        ),
        node!(
            "create",
            "create",
            "Create a new role template",
            Execute(super::handlers::role_create)
        ),
        node!(
            "embed",
            "embed",
            "Compute embeddings for all roles",
            Execute(super::handlers::role_embed)
        ),
        node!(
            "delete",
            "delete",
            "Delete a role template",
            Branch(role_names_delete_provider)
        ),
    ]
}

pub fn think_provider(_: &CommandContext) -> Vec<Node> {
    vec![
        node!(
            "on",
            "on",
            "Show full reasoning",
            Execute(super::handlers::think_set)
        ),
        node!(
            "off",
            "off",
            "Hide reasoning",
            Execute(super::handlers::think_set)
        ),
        node!(
            "brief",
            "brief",
            "Show brief reasoning",
            Execute(super::handlers::think_set)
        ),
        node!(
            "low",
            "low",
            "Reasoning effort: low",
            Execute(super::handlers::think_set)
        ),
        node!(
            "medium",
            "medium",
            "Reasoning effort: medium",
            Execute(super::handlers::think_set)
        ),
        node!(
            "high",
            "high",
            "Reasoning effort: high",
            Execute(super::handlers::think_set)
        ),
        node!(
            "status",
            "status",
            "Show current level",
            Execute(super::handlers::think_status)
        ),
    ]
}

pub fn pool_provider(_: &CommandContext) -> Vec<Node> {
    vec![
        node!(
            "stats",
            "stats",
            "Show pool statistics",
            Execute(super::handlers::pool_stats)
        ),
        node!(
            "flush",
            "flush",
            "Flush bedrock to disk",
            Execute(super::handlers::pool_flush)
        ),
        node!(
            "clear",
            "clear",
            "Clear experience pool",
            Execute(super::handlers::pool_clear)
        ),
        node!(
            "export",
            "export",
            "Export pool to JSON",
            Execute(super::handlers::pool_export)
        ),
        node!(
            "import",
            "import",
            "Import pool from JSON",
            Execute(super::handlers::pool_import)
        ),
        node!(
            "query",
            "query",
            "Query experiences by text similarity",
            Execute(super::handlers::pool_query)
        ),
    ]
}

pub fn sessions_provider(_: &CommandContext) -> Vec<Node> {
    let sessions = crate::persistence::list_sessions();
    sessions
        .into_iter()
        .filter_map(|name| {
            let count = crate::persistence::load_session_as(&name)
                .map(|m| m.len())
                .unwrap_or(0);
            if count == 0 {
                return None;
            }
            Some(Node {
                id: Cow::Owned(name.clone()),
                display: Cow::Owned(format!("{} ({} msgs)", name, count)),
                help: Cow::Owned(format!("Saved with {} messages", count)),
                kind: NodeKind::Execute {
                    handler: super::handlers::session_switch,
                },
            })
        })
        .collect()
}

fn memo_keys_for_role(ctx: &CommandContext) -> Option<Vec<(String, u64)>> {
    let agent_id = ctx.core.responsible_agent_id?;
    let pool = ctx.core.agent_pool.try_read().ok()?;
    let role = pool.get_agent(&agent_id)?.role.clone();
    Some(
        pool.get_role_memos(&role)
            .iter()
            .map(|m| (m.key.clone(), m.timestamp))
            .collect(),
    )
}

fn memo_keys_provider(ctx: &CommandContext, handler: Handler) -> Vec<Node> {
    let keys = match memo_keys_for_role(ctx) {
        Some(k) => k,
        None => return vec![],
    };
    keys.into_iter()
        .map(|(key, ts)| Node {
            id: Cow::Owned(key.clone()),
            display: Cow::Owned(key),
            help: Cow::Owned(format!("written at {}", ts)),
            kind: NodeKind::Execute { handler },
        })
        .collect()
}

pub fn memo_keys_show_provider(ctx: &CommandContext) -> Vec<Node> {
    memo_keys_provider(ctx, super::handlers::memo_show)
}
pub fn memo_keys_delete_provider(ctx: &CommandContext) -> Vec<Node> {
    memo_keys_provider(ctx, super::handlers::memo_delete)
}

pub fn memo_provider(_: &CommandContext) -> Vec<Node> {
    vec![
        node!(
            "show",
            "show",
            "Show a memo by key",
            Branch(memo_keys_show_provider)
        ),
        node!(
            "write",
            "write",
            "Write a memo (key=value)",
            Execute(super::handlers::memo_write)
        ),
        node!(
            "delete",
            "delete",
            "Delete a memo",
            Branch(memo_keys_delete_provider)
        ),
        node!(
            "roles",
            "roles",
            "List roles with memos",
            Execute(super::handlers::memo_roles)
        ),
    ]
}

pub fn agent_provider(_: &CommandContext) -> Vec<Node> {
    vec![node!(
        "inspect",
        "inspect",
        "Show agent detail by ID",
        Branch(agent_inspect_provider)
    )]
}

pub fn agent_inspect_provider(ctx: &CommandContext) -> Vec<Node> {
    let pool = match ctx.core.agent_pool.try_read() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let agents = pool.agents();
    if agents.is_empty() {
        return vec![];
    }
    agents
        .iter()
        .map(|a| {
            let id_str = crate::agent::AgentPool::agent_id_str(&a.id);
            let short = id_str[..12.min(id_str.len())].to_string();
            Node {
                id: Cow::Owned(short.clone()),
                display: Cow::Owned(format!("{} - {}", short, a.name)),
                help: Cow::Owned(format!("{:?}, depth={}", a.status, a.depth)),
                kind: NodeKind::Execute {
                    handler: super::handlers::agent_inspect,
                },
            }
        })
        .collect()
}

pub fn reflect_provider(_: &CommandContext) -> Vec<Node> {
    vec![
        node!(
            "on",
            "on",
            "Enable reflection",
            Execute(super::handlers::reflect_on)
        ),
        node!(
            "off",
            "off",
            "Disable reflection",
            Execute(super::handlers::reflect_off)
        ),
        node!(
            "status",
            "status",
            "Show reflection status",
            Execute(super::handlers::reflect_status)
        ),
        node!(
            "max",
            "max <N>",
            "Set max retry attempts",
            Execute(super::handlers::reflect_max)
        ),
        node!(
            "rule",
            "rule",
            "Toggle a reflection rule",
            Execute(super::handlers::reflect_rule)
        ),
    ]
}
