//! Parallel tool execution and event streaming.
//!
//! pi-agent-core design:
//! 1. Call LLM with tool definitions (via CompletionRequest, no Agent)
//! 2. Stream response, capture ToolCall events
//! 3. Execute tools in parallel via ToolServerHandle
//! 4. Emit ToolExecutionStart/End for each tool
//! 5. Feed results back to LLM, repeat

use futures::future::join_all;
use rig::completion::ToolDefinition;
use rig::tool::server::ToolServerHandle;

use crate::llm::ToolEvent;

/// Execute tool calls in parallel. Returns (name, result, is_error) for each.
pub async fn execute_tools_parallel(
    tool_calls: &[(String, String)],
    handle: &ToolServerHandle,
) -> Vec<ToolResult> {
    let futures: Vec<_> = tool_calls
        .iter()
        .map(|(name, args)| {
            let h = handle.clone();
            let n = name.clone();
            let a = args.clone();
            tokio::spawn(async move {
                let start = std::time::Instant::now();
                match h.call_tool(&n, &a).await {
                    Ok(result) => ToolResult {
                        name: n,
                        result,
                        is_error: false,
                        duration_ms: start.elapsed().as_millis() as u64,
                    },
                    Err(e) => ToolResult {
                        name: n,
                        result: format!("Error: {}", e),
                        is_error: true,
                        duration_ms: start.elapsed().as_millis() as u64,
                    },
                }
            })
        })
        .collect();

    join_all(futures)
        .await
        .into_iter()
        .map(|r| {
            r.unwrap_or_else(|e| ToolResult {
                name: "unknown".into(),
                result: format!("Join error: {}", e),
                is_error: true,
                duration_ms: 0,
            })
        })
        .collect()
}

/// Result of a single tool execution.
pub struct ToolResult {
    pub name: String,
    pub result: String,
    pub is_error: bool,
    pub duration_ms: u64,
}

/// Emit tool execution events (pi-agent-core: start → end sequence).
pub fn tool_execution_events(name: &str, result: &str, is_error: bool) -> Vec<ToolEvent> {
    let mut events = Vec::with_capacity(3);
    events.push(ToolEvent::Text(format!("[Executing {}...]\n", name)));
    if is_error {
        events.push(ToolEvent::Text(format!("  Error: {}\n", result)));
    }
    events
}

/// Get tool definitions from ToolServerHandle in rig format.
pub async fn get_tool_definitions(handle: &ToolServerHandle) -> Vec<ToolDefinition> {
    handle.get_tool_defs(None).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_empty() {
        let handle = crate::tools::create_tool_server();
        let r = execute_tools_parallel(&[], &handle).await;
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn test_get_tool_defs() {
        let handle = crate::tools::create_tool_server();
        let defs = get_tool_definitions(&handle).await;
        assert!(!defs.is_empty(), "should have tool definitions");
        assert!(defs.iter().any(|d| d.name == "read_file"));
    }
}
