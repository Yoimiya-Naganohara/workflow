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
                            });
                            if agent.tool_trace.len() > crate::agent::MAX_TOOL_TRACE {
                                agent.tool_trace.pop_front();
                            }
                        }
                    }
                }
                crate::llm::ToolEvent::TokenUsage { input, output, .. } => {
                    // Accumulate token counts on the agent for UI display.
                    if input > 0 || output > 0 {
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
            let has_error = error_keywords.iter().any(|pat| text.contains(pat));
            if has_error {
                if let Ok(mut pool) = agent_pool.try_write() {
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        for record in agent.tool_trace.iter_mut().rev().take(2) {
                            if record.status == crate::agent::ToolStatus::Success {
                                record.status = crate::agent::ToolStatus::Error;
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
