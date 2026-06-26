//! Agent stream processing — consume ToolEvent stream.
use super::AgentRuntime;
use crate::agent::AgentPool;
use crate::core::types::*;
use std::sync::Arc;
use tokio::sync::RwLock;

impl AgentRuntime {
    pub(crate) async fn process_tool_stream(
        stream: crate::llm::ToolChatStream,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> (String, u64) {
        use futures::StreamExt;
        futures::pin_mut!(stream);
        let mut text = String::new();
        let mut tools_used: u64 = 0;
        let mut tool_call_count = 0usize;
        let mut done_received = false;

        while let Some(event) = stream.next().await {
            match event {
                crate::llm::ToolEvent::Text(t) => text.push_str(&t),
                crate::llm::ToolEvent::Reasoning(t) => {
                    if let Ok(mut pool) = agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.reasoning.push_str(&t);
                        }
                    }
                }
                crate::llm::ToolEvent::ToolCall { name, args, .. } => {
                    tool_call_count += 1;
                    tools_used |= Self::tool_bit(&name);
                    let args_preview = serde_json::to_string(&args).unwrap_or_default();
                    let args_preview = if args_preview.len() > 80 {
                        format!("{}…", args_preview.chars().take(80).collect::<String>())
                    } else {
                        args_preview
                    };
                    if let Ok(mut pool) = agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.tool_trace.push_back(crate::agent::ToolCallRecord {
                                name,
                                args_preview,
                                status: crate::agent::ToolStatus::Success,
                                error_message: None,
                            });
                            if agent.tool_trace.len() > crate::agent::MAX_TOOL_TRACE {
                                agent.tool_trace.pop_front();
                            }
                        }
                    }
                }
                crate::llm::ToolEvent::TokenUsage {
                    input,
                    output,
                    cached_input,
                    cache_creation_input,
                } => {
                    // Accumulate token counts on the agent for UI display.
                    if input > 0 || output > 0 || cached_input > 0 || cache_creation_input > 0 {
                        if let Ok(mut pool) = agent_pool.try_write() {
                            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                                agent.tokens_input = agent.tokens_input.saturating_add(input);
                                agent.tokens_output = agent.tokens_output.saturating_add(output);
                            }
                        }
                    }
                }
                crate::llm::ToolEvent::Done { reason } => {
                    done_received = true;
                    if reason == crate::llm::DoneReason::LoopTerminated
                        || reason == crate::llm::DoneReason::StreamError
                    {
                        tracing::info!(
                            "Agent {:02x}.. completed with {} ({} bytes)",
                            agent_id[0],
                            if reason == crate::llm::DoneReason::LoopTerminated {
                                "loop termination"
                            } else {
                                "stream error"
                            },
                            text.len(),
                        );
                    }
                    break;
                }
            }
        }

        if !done_received {
            tracing::warn!(
                "Agent {:02x}.. stream ended without Done event ({} bytes)",
                agent_id[0],
                text.len(),
            );
        }

        // Empty text fallback
        if text.trim().is_empty() && tool_call_count > 0 {
            text = format!(
                "Completed after {} tool call{}.",
                tool_call_count,
                if tool_call_count == 1 { "" } else { "s" }
            );
        }

        // Heuristic tool error detection
        if !text.is_empty() {
            let error_keywords = [
                "error:",
                "Error:",
                "ERROR:",
                "failed:",
                "Failed:",
                "FAILED:",
                "Not Found",
                "permission denied",
                "Permission denied",
                "timed out",
                "Timed Out",
                "timeout",
                "Tool execution error",
                "connection refused",
                "Connection refused",
                "no such file",
                "No such file",
                "exit code:",
                "Tool execution failed",
                "cannot read",
                "Cannot read",
                "is a directory",
                "Is a directory",
            ];
            if let Some(error_line) = error_keywords.iter().find_map(|pat| {
                let start = text.find(pat)?;
                let end = (start + 120).min(text.len());
                Some(text[start..end].to_string())
            }) {
                if let Ok(mut pool) = agent_pool.try_write() {
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        for record in agent.tool_trace.iter_mut().rev().take(2) {
                            if record.status == crate::agent::ToolStatus::Success {
                                record.status = crate::agent::ToolStatus::Error;
                                record.error_message = Some(error_line.clone());
                                break;
                            }
                        }
                    }
                }
            }
        }

        (text, tools_used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{DoneReason, ToolChatStream, ToolEvent};
    use futures::stream;

    /// Helper: create a mock stream from a vec of ToolEvents.
    fn mock_stream(events: Vec<ToolEvent>) -> ToolChatStream {
        Box::pin(stream::iter(events))
    }

    /// Helper: create a pool with one agent, return (pool, agent_id).
    fn make_pool_and_agent() -> (Arc<RwLock<AgentPool>>, AgentId) {
        let mut pool = AgentPool::new();
        let agent = crate::agent::Agent {
            id: rand::random(),
            name: "test-agent".to_string(),
            role: "tester".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "test".to_string(),
            config: crate::agent::AgentConfig::default(),
            status: crate::agent::AgentStatus::Planning,
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
            reasoning: String::new(),
        };
        let id = agent.id;
        pool.add_agent(agent);
        (Arc::new(RwLock::new(pool)), id)
    }

    // ──────────────────────────────────────────────────────
    //  Tests
    // ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_text_only() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::Text("Hello ".to_string()),
            ToolEvent::Text("World".to_string()),
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        let (text, _tools) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert_eq!(text, "Hello World");
    }

    #[tokio::test]
    async fn test_tool_call_updates_trace() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::ToolCall {
                name: "read_file".to_string(),
                args: serde_json::json!({}),
                result: String::new(),
            },
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        let (_text, tools) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert_eq!(tools, 1 << 0, "read_file = bit 0");
        // Verify tool trace on agent
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        assert_eq!(agent.tool_trace.len(), 1);
        assert_eq!(agent.tool_trace[0].name, "read_file");
    }

    #[tokio::test]
    async fn test_reasoning_updates_agent() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::Reasoning("thinking...".to_string()),
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        assert_eq!(agent.reasoning, "thinking...");
    }

    #[tokio::test]
    async fn test_token_usage_accumulates() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::TokenUsage {
                input: 100,
                output: 50,
                cached_input: 0,
                cache_creation_input: 0,
            },
            ToolEvent::TokenUsage {
                input: 200,
                output: 75,
                cached_input: 0,
                cache_creation_input: 0,
            },
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        assert_eq!(agent.tokens_input, 300);
        assert_eq!(agent.tokens_output, 125);
    }

    #[tokio::test]
    async fn test_zero_token_usage_skipped() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::TokenUsage {
                input: 0,
                output: 0,
                cached_input: 0,
                cache_creation_input: 0,
            },
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        assert_eq!(agent.tokens_input, 0);
    }

    #[tokio::test]
    async fn test_done_loop_terminated() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::Text("some text".to_string()),
            ToolEvent::Done {
                reason: DoneReason::LoopTerminated,
            },
        ]);
        let (text, _) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert_eq!(text, "some text");
    }

    #[tokio::test]
    async fn test_done_stream_error() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![ToolEvent::Done {
            reason: DoneReason::StreamError,
        }]);
        let (text, _) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert!(text.is_empty());
    }

    #[tokio::test]
    async fn test_no_done_event_fallback_text() {
        let (pool, aid) = make_pool_and_agent();
        // Stream ends without Done — triggers warning path
        let stream = mock_stream(vec![
            ToolEvent::Text("hello".to_string()),
            // no Done
        ]);
        let (text, _) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn test_empty_text_with_tool_calls_uses_fallback() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::ToolCall {
                name: "sh".to_string(),
                args: serde_json::json!({"cmd": "ls"}),
                result: String::new(),
            },
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        let (text, _) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert!(text.contains("Completed after"));
        assert!(text.contains("1 tool call"));
    }

    #[tokio::test]
    async fn test_empty_text_plural_tool_calls() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::ToolCall {
                name: "sh".to_string(),
                args: serde_json::json!({"cmd": "ls"}),
                result: String::new(),
            },
            ToolEvent::ToolCall {
                name: "read_file".to_string(),
                args: serde_json::json!({"path": "x"}),
                result: String::new(),
            },
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        let (text, _) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert!(text.contains("2 tool calls"));
    }

    #[tokio::test]
    async fn test_tool_error_detection_marks_trace() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::ToolCall {
                name: "read_file".to_string(),
                args: serde_json::json!({"path": "/nonexistent"}),
                result: String::new(),
            },
            ToolEvent::Text("Error: file not found".to_string()),
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        // Last tool call should be marked Error
        assert!(
            agent
                .tool_trace
                .iter()
                .any(|t| t.status == crate::agent::ToolStatus::Error),
            "expected at least one tool marked Error"
        );
    }

    #[tokio::test]
    async fn test_no_false_positive_error_detection() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::ToolCall {
                name: "read_file".to_string(),
                args: serde_json::json!({"path": "/tmp/x"}),
                result: String::new(),
            },
            ToolEvent::Text("Success: file read OK".to_string()),
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        // No error keyword matched, tool should remain Success
        assert!(
            agent
                .tool_trace
                .iter()
                .all(|t| t.status == crate::agent::ToolStatus::Success),
            "no tool should be marked Error"
        );
    }

    #[tokio::test]
    async fn test_tool_call_trace_overflow() {
        let (pool, aid) = make_pool_and_agent();
        let max = crate::agent::MAX_TOOL_TRACE;
        let mut events: Vec<ToolEvent> = (0..max + 5)
            .map(|i| ToolEvent::ToolCall {
                name: format!("tool_{}", i),
                args: serde_json::json!({}),
                result: String::new(),
            })
            .collect();
        events.push(ToolEvent::Done {
            reason: DoneReason::Normal,
        });
        let stream = mock_stream(events);
        AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        assert_eq!(
            agent.tool_trace.len(),
            max,
            "trace should be capped at MAX_TOOL_TRACE"
        );
    }

    #[tokio::test]
    async fn test_multiple_tool_types_bits() {
        let (pool, aid) = make_pool_and_agent();
        let stream = mock_stream(vec![
            ToolEvent::ToolCall {
                name: "read_file".to_string(),
                args: serde_json::json!({}),
                result: String::new(),
            },
            ToolEvent::ToolCall {
                name: "sh".to_string(),
                args: serde_json::json!({}),
                result: String::new(),
            },
            ToolEvent::ToolCall {
                name: "write_file".to_string(),
                args: serde_json::json!({}),
                result: String::new(),
            },
            ToolEvent::Done {
                reason: DoneReason::Normal,
            },
        ]);
        let (_text, tools) = AgentRuntime::process_tool_stream(stream, aid, &pool).await;
        assert!(tools & (1 << 0) != 0, "read_file bit set"); // read_file = bit 0
        assert!(tools & (1 << 2) != 0, "sh bit set"); // sh = bit 2
        assert!(tools & (1 << 1) != 0, "write_file bit set"); // write_file = bit 1
    }
}
