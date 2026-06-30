//! Agent diagnostic tree — read-only projection from `AgentPool`.
//!
//! This module builds a flat, indented line representation of the agent
//! delegation hierarchy.  It is designed to be called from the TUI render
//! loop with a `try_read` guard so it never blocks the UI thread.
//!
//! # Phase 1 contract
//!
//! - Pure data → text transformation.  No I/O, no async.
//! - Uses only existing `AgentPool` / `Agent` fields.
//! - Testable without Tokio or LLM API keys.

use wf_agent::{Agent, AgentPool, AgentStatus, ToolStatus};
use wf_core::AgentId;

/// A single line in the diagnostic tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeLine {
    pub agent_id: AgentId,
    pub display_text: String,
    pub status: AgentStatus,
}

/// Status symbol and colour hint for a given agent status.
fn status_tag(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "[Idle]",
        AgentStatus::Planning => "[Planning]",
        AgentStatus::AwaitingChildren => "[Awaiting]",
        AgentStatus::Aggregating => "[Aggregating]",
        AgentStatus::Completed => "[Completed]",
        AgentStatus::Failed => "[Failed]",
    }
}

/// Recursive depth-first builder.
///
/// `agents` must contain every agent referenced in the tree reachable from
/// `root_id`.  Missing agents are silently skipped (defensive — avoids
/// panic if the pool has dangling `children` references during mutation).
fn build_diagnostic_tree(
    agents: &std::collections::HashMap<AgentId, &Agent>,
    root_id: &AgentId,
    depth: usize,
    is_last: bool,
    lines: &mut Vec<TreeLine>,
) {
    let Some(agent) = agents.get(root_id) else {
        return;
    };

    // Prefix construction
    let prefix = if depth == 0 {
        "* ".to_string()
    } else {
        let indent = "  ".repeat(depth.saturating_sub(1));
        let branch = if is_last { "`-* " } else { "|-* " };
        format!("{}{}", indent, branch)
    };

    // Truncate long names for compact display
    let name = if agent.name.len() > 20 {
        format!("{}…", &agent.name[..19])
    } else {
        agent.name.clone()
    };

    let display_text = format!("{:<6}{:<22} {}", prefix, name, status_tag(&agent.status),);

    lines.push(TreeLine {
        agent_id: agent.id,
        display_text,
        status: agent.status.clone(),
    });

    for (idx, child_id) in agent.children.iter().enumerate() {
        build_diagnostic_tree(
            agents,
            child_id,
            depth + 1,
            idx + 1 == agent.children.len(),
            lines,
        );
    }
}

/// Build the diagnostic tree lines for a pool, rooted at `root_id`.
///
/// Uses `try_read`(std::sync::TryLockError) internally — if the pool
/// lock is contended the function returns an empty vec and the caller
/// should retry on the next frame.
pub fn build_agent_tree_lines(pool: &AgentPool, root_id: &AgentId) -> Vec<TreeLine> {
    let agents_snapshot: std::collections::HashMap<AgentId, &Agent> =
        pool.agents().iter().map(|a| (a.id, a)).collect();

    if !agents_snapshot.contains_key(root_id) {
        return Vec::new();
    }

    let mut lines = Vec::new();
    build_diagnostic_tree(&agents_snapshot, root_id, 0, true, &mut lines);
    lines
}

/// Build a short preview of the last few tool trace entries for an agent.
/// Returns up to 3 formatted lines (truncated for compact display).
pub fn build_tool_trace_preview(pool: &AgentPool, agent_id: &AgentId) -> Vec<String> {
    let Some(agent) = pool.agents().iter().find(|a| a.id == *agent_id) else {
        return Vec::new();
    };
    agent
        .tool_trace
        .iter()
        .rev()
        .take(3)
        .map(|record| {
            let mark = match record.status {
                ToolStatus::Running => "⏳",
                ToolStatus::Success => "✓",
                ToolStatus::Error => "✗",
            };
            format!("  {} {}({})", mark, record.name, record.args_preview)
        })
        .collect()
}

/// Returns `true` if the root agent has at least one child that is still
/// active (not yet Completed or Failed).
pub fn has_active_delegations(pool: &AgentPool, root_id: &AgentId) -> bool {
    let Some(root) = pool.agents().iter().find(|a| a.id == *root_id) else {
        return false;
    };
    root.children.iter().any(|child_id| {
        pool.agents()
            .iter()
            .find(|a| a.id == *child_id)
            .map(|child| !matches!(child.status, AgentStatus::Completed | AgentStatus::Failed))
            .unwrap_or(false)
    })
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use wf_agent::{AgentConfig, ToolCallRecord, ToolStatus};

    fn stub_agent() -> Agent {
        Agent {
            id: [0u8; 16],
            name: String::new(),
            role: String::new(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: String::new(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: 0,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: None,
            sandbox: None,
            retry_count: 0,
            loop_terminated: false,
            reasoning: String::new(),
        }
    }

    /// Build a three-agent hierarchy:
    ///
    /// ```text
    /// ● planner (AwaitingChildren)
    ///   ├─◆ developer (Planning) ⠋
    ///   └─◆ reviewer  (Completed)
    /// ```
    fn build_test_pool_with_hierarchy() -> (AgentPool, AgentId) {
        let mut pool = AgentPool::new();
        let root_id: AgentId = [0u8; 16];
        let dev_id: AgentId = [1u8; 16];
        let rev_id: AgentId = [2u8; 16];

        pool.add_agent(Agent {
            id: root_id,
            name: "planner".into(),
            role: "planner".into(),
            status: AgentStatus::AwaitingChildren,
            children: vec![dev_id, rev_id],
            ..stub_agent()
        });

        pool.add_agent(Agent {
            id: dev_id,
            name: "developer".into(),
            role: "developer".into(),
            parent_id: Some(root_id),
            status: AgentStatus::Planning,
            ..stub_agent()
        });

        pool.add_agent(Agent {
            id: rev_id,
            name: "reviewer".into(),
            role: "reviewer".into(),
            parent_id: Some(root_id),
            status: AgentStatus::Completed,
            result: Some("LGTM".into()),
            ..stub_agent()
        });

        (pool, root_id)
    }

    #[test]
    fn test_tree_single_root_no_children() {
        let mut pool = AgentPool::new();
        let id = [0u8; 16];
        pool.add_agent(Agent {
            id,
            name: "solo".into(),
            status: AgentStatus::Completed,
            ..stub_agent()
        });

        let lines = build_agent_tree_lines(&pool, &id);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].display_text.starts_with("* "));
        assert!(lines[0].display_text.contains("Completed"));
        assert_eq!(lines[0].agent_id, id);
    }

    #[test]
    fn test_tree_hierarchy_indentation() {
        let (pool, root_id) = build_test_pool_with_hierarchy();
        let lines = build_agent_tree_lines(&pool, &root_id);

        assert_eq!(lines.len(), 3, "should have root + 2 children");

        // Root: * symbol
        assert!(
            lines[0].display_text.starts_with("* "),
            "root prefix: {:?}",
            lines[0].display_text
        );
        assert!(lines[0].display_text.contains("Awaiting"));

        // First child: |-*
        assert!(
            lines[1].display_text.starts_with("|-* "),
            "first child prefix: {:?}",
            lines[1].display_text
        );
        assert!(lines[1].display_text.contains("Planning"));

        // Last child: `-*
        assert!(
            lines[2].display_text.starts_with("`-* "),
            "last child prefix: {:?}",
            lines[2].display_text
        );
        assert!(lines[2].display_text.contains("Completed"));
    }

    #[test]
    fn test_tree_root_not_found_returns_empty() {
        let pool = AgentPool::new();
        let lines = build_agent_tree_lines(&pool, &[42u8; 16]);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_has_active_delegations_true_when_child_active() {
        let (pool, root_id) = build_test_pool_with_hierarchy();
        // developer is Planning → active delegation exists
        assert!(has_active_delegations(&pool, &root_id));
    }

    #[test]
    fn test_has_active_delegations_false_when_all_done() {
        let mut pool = AgentPool::new();
        let root_id = [0u8; 16];
        let child_id = [1u8; 16];

        pool.add_agent(Agent {
            id: root_id,
            name: "root".into(),
            status: AgentStatus::Completed,
            children: vec![child_id],
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: child_id,
            name: "child".into(),
            status: AgentStatus::Completed,
            parent_id: Some(root_id),
            ..stub_agent()
        });

        assert!(!has_active_delegations(&pool, &root_id));
    }

    #[test]
    fn test_has_active_delegations_false_no_children() {
        let mut pool = AgentPool::new();
        let root_id = [0u8; 16];
        pool.add_agent(Agent {
            id: root_id,
            name: "lonely".into(),
            status: AgentStatus::Planning,
            ..stub_agent()
        });

        assert!(!has_active_delegations(&pool, &root_id));
    }

    #[test]
    fn test_deep_nested_tree_uses_correct_prefixes() {
        // Build a chain: root → middle → leaf
        let mut pool = AgentPool::new();
        let root_id = [0u8; 16];
        let mid_id = [1u8; 16];
        let leaf_id = [2u8; 16];

        let other_id = [3u8; 16];
        pool.add_agent(Agent {
            id: other_id,
            name: "sibling".into(),
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: root_id,
            name: "root".into(),
            status: AgentStatus::AwaitingChildren,
            children: vec![mid_id, other_id],
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: mid_id,
            name: "middle".into(),
            status: AgentStatus::AwaitingChildren,
            parent_id: Some(root_id),
            children: vec![leaf_id],
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: leaf_id,
            name: "leaf".into(),
            status: AgentStatus::Planning,
            parent_id: Some(mid_id),
            ..stub_agent()
        });

        let lines = build_agent_tree_lines(&pool, &root_id);
        assert_eq!(lines.len(), 4, "root + middle + leaf + sibling");

        // Root: *
        assert!(
            lines[0].display_text.starts_with("* "),
            "root: {:?}",
            lines[0].display_text
        );
        // Middle (first child of root, not last): |-*
        assert!(
            lines[1].display_text.starts_with("|-* "),
            "middle: {:?}",
            lines[1].display_text
        );
        // Leaf (first child of middle, only child, thus last):   `-*
        // Depth=2, indent is "  "
        assert!(
            lines[2].display_text.starts_with("  `-* "),
            "leaf prefix: {:?}",
            lines[2].display_text
        );
        // Sibling (second child of root, last child, thus last): `-*
        assert!(
            lines[3].display_text.starts_with("`-* "),
            "sibling prefix: {:?}",
            lines[3].display_text
        );
    }

    // ── build_tool_trace_preview tests (Phase 3) ──

    #[test]
    fn test_tool_trace_preview_empty_when_no_trace() {
        let mut pool = AgentPool::new();
        let id = [0u8; 16];
        pool.add_agent(Agent {
            id,
            name: "quiet".into(),
            status: AgentStatus::Completed,
            ..stub_agent()
        });

        let preview = build_tool_trace_preview(&pool, &id);
        assert!(preview.is_empty(), "no tools called → empty preview");
    }

    #[test]
    fn test_tool_trace_preview_returns_up_to_three() {
        let mut pool = AgentPool::new();
        let id = [0u8; 16];
        pool.add_agent(Agent {
            id,
            name: "busy".into(),
            status: AgentStatus::Completed,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::from(vec![
                ToolCallRecord {
                    name: "read_file".into(),
                    args_preview: "\"path\": \"src/main.rs\"".into(),
                    status: ToolStatus::Success,
                    error_message: None,
                },
                ToolCallRecord {
                    name: "sh".into(),
                    args_preview: "\"command\": \"grep fn main\"".into(),
                    status: ToolStatus::Success,
                    error_message: None,
                },
                ToolCallRecord {
                    name: "write_file".into(),
                    args_preview: "\"path\": \"src/lib.rs\"".into(),
                    status: ToolStatus::Success,
                    error_message: None,
                },
            ]),
            ..stub_agent()
        });

        let preview = build_tool_trace_preview(&pool, &id);
        assert_eq!(preview.len(), 3, "all 3 entries returned");
        assert!(
            preview[0].contains("write_file"),
            "most recent first: {}",
            preview[0]
        );
        assert!(
            preview[2].contains("read_file"),
            "oldest last: {}",
            preview[2]
        );
    }

    #[test]
    fn test_tool_trace_preview_truncates_beyond_three() {
        let mut pool = AgentPool::new();
        let id = [0u8; 16];
        let mut trace: std::collections::VecDeque<ToolCallRecord> =
            std::collections::VecDeque::new();
        for i in 0..5 {
            trace.push_back(ToolCallRecord {
                name: format!("tool_{}", i),
                args_preview: String::new(),
                status: ToolStatus::Success,
                error_message: None,
            });
        }
        pool.add_agent(Agent {
            id,
            name: "overdrive".into(),
            status: AgentStatus::Completed,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: trace,
            ..stub_agent()
        });

        let preview = build_tool_trace_preview(&pool, &id);
        assert_eq!(preview.len(), 3, "capped at 3");
        assert!(
            preview[0].contains("tool_4"),
            "most recent tool_4: {}",
            preview[0]
        );
        assert!(
            preview[2].contains("tool_2"),
            "oldest shown tool_2: {}",
            preview[2]
        );
    }

    #[test]
    fn test_tool_trace_preview_handles_error_status() {
        let mut pool = AgentPool::new();
        let id = [0u8; 16];
        pool.add_agent(Agent {
            id,
            name: "faulty".into(),
            status: AgentStatus::Failed,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::from(vec![ToolCallRecord {
                name: "sh".into(),
                args_preview: "\"command\": \"rm -rf /\"".into(),
                status: ToolStatus::Error,
                error_message: None,
            }]),
            ..stub_agent()
        });

        let preview = build_tool_trace_preview(&pool, &id);
        assert_eq!(preview.len(), 1);
        assert!(preview[0].contains('✗'), "error marker present");
    }
}
