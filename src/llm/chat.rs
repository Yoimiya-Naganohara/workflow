use super::*;
use anyhow::Result;

/// Callback after each tool round: (tool_names, text) → continue?
pub type AfterToolFn = Box<dyn Fn(&[String], &str) -> bool + Send + 'static>;

/// Callback before each round: (prev_tool_names) → allow?
/// Return false to block the round.
pub type BeforeToolFn = Box<dyn Fn(&[String]) -> bool + Send + 'static>;

/// Configuration for `chat_with_tools_loop`.
#[derive(Default)]
pub struct LoopConfig {
    /// Called after each round with (tool_names, text). Return false to stop.
    pub after_tool: Option<AfterToolFn>,
    /// Called before each round with previous round's tool names.
    /// Return false to block the round.
    pub before_tool: Option<BeforeToolFn>,
}
use async_stream::stream;
use futures::StreamExt;
use rig::OneOrMany;
use rig::agent::{MultiTurnStreamItem, StreamingResult};
use rig::completion::message::{AssistantContent, UserContent};
use rig::message::Text;
use rig::streaming::{StreamedAssistantContent, StreamingChat};
use rig::tool::server::ToolServerHandle;

/// Build a chat agent with standard temperature/max_tokens configuration.
///
/// All rig provider clients share the same builder pattern but return
/// different concrete types — a macro avoids 9× repetition.
macro_rules! build_chat_agent {
    ($client:expr, $model:expr, $system:expr, $params:expr) => {{
        let mut builder = $client
            .agent($model)
            .preamble($system)
            .temperature(crate::core::types::DEFAULT_TEMPERATURE)
            .max_tokens(crate::core::types::DEFAULT_MAX_TOKENS);
        if let Some(p) = $params {
            builder = builder.additional_params(p.clone());
        }
        builder.build()
    }};
}

/// Per-provider match arm for `chat_with_tools_stream_mcp`.
///
/// Uses `ToolServerHandle` so tools can be added/removed at runtime
/// without recompiling.
macro_rules! mcp_stream_arm {
    ($client:expr, $model:expr, $system:expr, $handle:expr, $msg:expr, $history:expr, $params:expr) => {{
        let mut builder = $client
            .agent($model)
            .preamble($system)
            .temperature(crate::core::types::DEFAULT_TEMPERATURE)
            .max_tokens(crate::core::types::DEFAULT_MAX_TOKENS)
            .tool_server_handle($handle.clone());
        if let Some(p) = $params {
            builder = builder.additional_params(p.clone());
        }
        let agent = builder.build();
        Ok(Self::wrap_tool_stream(
            agent
                .stream_chat($msg, $history)
                .multi_turn(crate::core::types::DEFAULT_MAX_TOOL_TURNS)
                .await,
        ))
    }};
}

impl LlmProvider {
    pub async fn chat(&self, model: &str, system: &str, message: &str) -> Result<String> {
        let mut stream = self.chat_stream(model, system, message).await?;
        let mut response = String::new();
        while let Some(chunk) = stream.next().await {
            response.push_str(&chunk?);
        }
        Ok(response)
    }

    pub async fn chat_stream(
        &self,
        model: &str,
        system: &str,
        message: &str,
    ) -> Result<ChatStream> {
        self.do_chat_stream(model, system, message, &[], None).await
    }

    pub async fn chat_stream_with_history(
        &self,
        model: &str,
        system: &str,
        message: &str,
        history: &[(String, String)],
    ) -> Result<ChatStream> {
        self.do_chat_stream(model, system, message, history, None)
            .await
    }

    /// Stream a chat response with multi-round loop and after_tool_call hook.
    ///
    /// Calls rig's multi_turn(1) in a loop. After each round, `after_tool`
    /// is called with (tool_names, text). Return `true` to continue, `false` to stop.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat_with_tools_loop(
        &self,
        model: &str,
        system: &str,
        message: &str,
        history: &[(String, String)],
        tool_server: &ToolServerHandle,
        additional_params: Option<&serde_json::Value>,
        config: LoopConfig,
    ) -> Result<ToolChatStream> {
        let first_msg = message.to_string();
        let sys = system.to_string();
        let mdl = model.to_string();
        let ts = tool_server.clone();
        let provider_ref = self.clone();
        let cfg = config;
        let initial_history: Vec<(String, String)> = history.to_vec();
        let addl = additional_params.cloned();

        Ok(Box::pin(async_stream::stream! {
            let mut current_msg = first_msg;
            let mut hist = initial_history;
            let mut prev_tools: Vec<String> = Vec::new();

            for _ in 0..10 {
                // before_tool: check if we should proceed
                if let Some(ref bf) = cfg.before_tool {
                    if !bf(&prev_tools) {
                        yield ToolEvent::Text("Tool blocked by policy.".into());
                        break;
                    }
                }

                let round = match provider_ref
                    .chat_with_tools_stream_mcp(&mdl, &sys, &current_msg, &hist, &ts, addl.as_ref())
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        yield ToolEvent::Text(format!("Error: {}", e));
                        break;
                    }
                };

                let mut text = String::new();
                let mut tools: Vec<(String, serde_json::Value)> = Vec::new();
                let mut done = false;

                futures::pin_mut!(round);
                use futures::StreamExt;

                while let Some(event) = round.next().await {
                    match &event {
                        ToolEvent::Text(t) => text.push_str(t),
                        ToolEvent::ToolCall { name, args } => tools.push((name.clone(), args.clone())),
                        ToolEvent::Done { .. } => done = true,
                        _ => {}
                    }
                    yield event;
                    if done { break; }
                }

                if tools.is_empty() { break; }

                // Emit ToolExecutionStart events
                let tool_call_pairs: Vec<(String, String)> = tools.iter()
                    .map(|(n, a)| (n.clone(), a.to_string()))
                    .collect();

                for (name, args) in &tools {
                    yield ToolEvent::ToolExecutionStart {
                        name: name.clone(),
                        args: args.clone(),
                    };
                }

                // Execute tools in parallel
                let tool_results = crate::runtime::agent_loop::execute_tools_parallel(
                    &tool_call_pairs, &ts,
                ).await;

                // Emit ToolExecutionEnd events
                for r in &tool_results {
                    yield ToolEvent::ToolExecutionEnd {
                        name: r.name.clone(),
                        result: r.result.clone(),
                        is_error: r.is_error,
                    };
                }

                // after_tool with results
                let tool_names: Vec<String> = tools.iter().map(|(n, _)| n.clone()).collect();
                if let Some(ref h) = cfg.after_tool {
                    if !h(&tool_names, &text) { break; }
                }

                prev_tools = tool_names;
                hist.push(("assistant".into(), text));
                hist.push(("user".into(), "Continue.".into()));
                current_msg = String::new();
            }
        }))
    }

    /// Stream a chat response with MCP tool-calling capability.
    ///
    /// Uses a [`ToolServerHandle`] so tools can be added or removed at runtime
    /// without recompiling. Tool calls are yielded as
    /// `ToolEvent::ToolCall` events alongside text chunks.
    pub async fn chat_with_tools_stream_mcp(
        &self,
        model: &str,
        system: &str,
        message: &str,
        history: &[(String, String)],
        tool_server: &ToolServerHandle,
        additional_params: Option<&serde_json::Value>,
    ) -> Result<ToolChatStream> {
        let history = Self::build_history(history);
        match self {
            Self::OpenAi(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Anthropic(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Cohere(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Gemini(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Mistral(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Ollama(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Llamafile(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Azure(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
            Self::Copilot(c) => mcp_stream_arm!(
                c,
                model,
                system,
                tool_server,
                message,
                history,
                additional_params
            ),
        }
    }

    fn build_history(history: &[(String, String)]) -> Vec<rig::completion::Message> {
        history
            .iter()
            .map(|(role, content)| match role.as_str() {
                "system" => rig::completion::Message::System {
                    content: content.clone(),
                },
                "user" => rig::completion::Message::User {
                    content: OneOrMany::one(UserContent::text(content.clone())),
                },
                _ => rig::completion::Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::text(content.clone())),
                },
            })
            .collect()
    }

    async fn do_chat_stream(
        &self,
        model: &str,
        system: &str,
        message: &str,
        history: &[(String, String)],
        additional_params: Option<&serde_json::Value>,
    ) -> Result<ChatStream> {
        let history = Self::build_history(history);
        match self {
            Self::OpenAi(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Anthropic(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Cohere(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Gemini(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Mistral(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Ollama(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Llamafile(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Azure(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
            Self::Copilot(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system, additional_params)
                    .stream_chat(message, history)
                    .await,
            )),
        }
    }

    fn wrap_chat_stream<R>(stream: StreamingResult<R>) -> ChatStream
    where
        R: Clone + Unpin + rig::completion::GetTokenUsage + Send + 'static,
    {
        let stream = stream! {
            let mut stream = stream;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::Text(Text { text, .. }),
                    )) => {
                        yield Ok(text);
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::Reasoning(reasoning),
                    )) => {
                        // Surface reasoning as text so user can see chain-of-thought
                        for block in reasoning.content {
                            if let rig::message::ReasoningContent::Text { text, .. } = block {
                                yield Ok(text);
                            }
                        }
                    }
                    Ok(MultiTurnStreamItem::FinalResponse(_)) => break,
                    Ok(_) => {}
                    Err(err) => {
                        yield Err(anyhow::anyhow!(err.to_string()));
                        break;
                    }
                }
            }
        };
        Box::pin(stream)
    }

    fn wrap_tool_stream<R>(stream: StreamingResult<R>) -> ToolChatStream
    where
        R: Clone + Unpin + rig::completion::GetTokenUsage + Send + 'static,
    {
        let stream = stream! {
            let mut stream = stream;
            // ── Duplicate detection: same tool + same args ──
            // Key = "tool_name:args_json", value = count of repeats.
            // Detects when the LLM calls the exact same tool with the exact
            // same arguments 3+ times — a reliable indicator of a tool loop.
            let mut dup_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            // ── Per-tool call count (regardless of args) ──
            // Key = tool_name, value = total calls to this tool.
            let mut per_tool_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            // ── Total tool calls in this stream ──
            let mut total_tool_calls: usize = 0;

            // Build a human-readable tool call summary from the counters.
            let tool_summary = |total: usize, per_tool: &std::collections::HashMap<String, usize>| -> String {
                if total == 0 {
                    return "(none)".to_string();
                }
                let mut tools: Vec<(&String, &usize)> = per_tool.iter().collect();
                tools.sort_by(|a, b| b.1.cmp(a.1));
                let detail: Vec<String> = tools.iter().map(|(name, count)| {
                    format!("{}×{}", name, count)
                }).collect();
                format!("{} call(s) — {}", total, detail.join(", "))
            };

            while let Some(item) = stream.next().await {

                match item {
                    Ok(MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::Text(Text { text, .. }),
                    )) => {
                        yield ToolEvent::Text(text);
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::ToolCall {
                            tool_call,
                            internal_call_id: _,
                        },
                    )) => {
                        let tool_name = tool_call.function.name.clone();
                        // Move args instead of clone — avoids one Value copy per call.
                        let args = tool_call.function.arguments;

                        // ── Bounding check 1: absolute total tool calls ──
                        total_tool_calls += 1;
                        if total_tool_calls > crate::core::constants::MAX_TOOL_CALLS_PER_STREAM {
                            let summary = tool_summary(total_tool_calls, &per_tool_count);
                            tracing::warn!(
                                "Tool loop closed: {} total calls (limit {}): {}",
                                total_tool_calls,
                                crate::core::constants::MAX_TOOL_CALLS_PER_STREAM,
                                summary,
                            );
                            yield ToolEvent::Text(format!(
                                "\n\n<system>Tool call limit reached: {}. The assistant cannot make further tool calls. Summarize what you have found so far.</system>\n",
                                summary,
                            ));
                            yield ToolEvent::Done {
                                reason: DoneReason::LoopTerminated,
                            };
                            break;
                        }

                        // ── Bounding check 2: same tool too many times ──
                        // Compute summary BEFORE mutable borrow of per_tool_count.
                        let (next_count, summary) = {
                            let next = per_tool_count.get(&tool_name).copied().unwrap_or(0) + 1;
                            let s = tool_summary(total_tool_calls, &per_tool_count);
                            (next, s)
                        };
                        *per_tool_count.entry(tool_name.clone()).or_insert(0) += 1;
                        if next_count > crate::core::constants::MAX_CALLS_PER_TOOL {
                            tracing::warn!(
                                "Tool loop closed: '{}' called {} times (limit {}): {}",
                                tool_name,
                                next_count,
                                crate::core::constants::MAX_CALLS_PER_TOOL,
                                summary,
                            );
                            yield ToolEvent::Text(format!(
                                "\n\n<system>Tool loop detected: '{}' called {} times (max {}). Tool calls stopped. Tool summary — {}. Summarize what you have found so far.</system>\n",
                                tool_name, next_count, crate::core::constants::MAX_CALLS_PER_TOOL, summary,
                            ));
                            yield ToolEvent::Done {
                                reason: DoneReason::LoopTerminated,
                            };
                            break;
                        }

                        // ── Duplicate detection: same tool + same args ──
                        let call_key = format!("{}:{}", tool_name, args);
                        let count = dup_count.entry(call_key).or_insert(0);
                        *count += 1;

                        if *count >= 3 {
                            let summary = tool_summary(total_tool_calls, &per_tool_count);
                            tracing::warn!(
                                "Tool loop closed: '{}' called {} times with same args: {}",
                                tool_name,
                                count,
                                summary,
                            );
                            yield ToolEvent::Text(format!(
                                "\n\n<system>Tool loop detected: '{}' called {} times with identical arguments. Tool calls stopped. Tool summary — {}. Summarize what you have found so far.</system>\n",
                                tool_name, count, summary,
                            ));
                            yield ToolEvent::Done {
                                reason: DoneReason::LoopTerminated,
                            };
                            break;
                        }

                        yield ToolEvent::ToolCall {
                            name: tool_name,
                            args,
                        };
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::Reasoning(reasoning),
                    )) => {
                        for block in reasoning.content {
                            if let rig::message::ReasoningContent::Text { text, .. } = block {
                                yield ToolEvent::Reasoning(text);
                            }
                        }
                    }
                    Ok(MultiTurnStreamItem::CompletionCall(call)) => {
                        // Emit per-turn token usage for intermediate tool-calling
                        // completions, not just the final response.
                        if let Some(usage) = call.usage {
                            if usage.input_tokens > 0 || usage.output_tokens > 0 {
                                yield ToolEvent::TokenUsage {
                                    input: usage.input_tokens as u32,
                                    output: usage.output_tokens as u32,
                                    cached_input: usage.cached_input_tokens as u32,
                                    cache_creation_input: usage.cache_creation_input_tokens as u32,
                                };
                            }
                        }
                    }
                    Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                        let usage = response.usage();
                        if usage.input_tokens > 0 || usage.output_tokens > 0 {
                            yield ToolEvent::TokenUsage {
                                input: usage.input_tokens as u32,
                                output: usage.output_tokens as u32,
                                cached_input: usage.cached_input_tokens as u32,
                                cache_creation_input: usage.cache_creation_input_tokens as u32,
                            };
                        }
                        yield ToolEvent::Done {
                            reason: DoneReason::Normal,
                        };
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        yield ToolEvent::Text(err.to_string());
                        yield ToolEvent::Done {
                            reason: DoneReason::StreamError,
                        };
                        break;
                    }
                }
            }
        };
        Box::pin(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_history_empty() {
        let history: Vec<(String, String)> = vec![];
        let msgs = LlmProvider::build_history(&history);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_tool_event_token_usage() {
        let event = ToolEvent::TokenUsage {
            input: 150,
            output: 75,
            cached_input: 10,
            cache_creation_input: 20,
        };
        match event {
            ToolEvent::TokenUsage {
                input,
                output,
                cached_input,
                cache_creation_input,
            } => {
                assert_eq!(input, 150);
                assert_eq!(output, 75);
                assert_eq!(cached_input, 10);
                assert_eq!(cache_creation_input, 20);
            }
            _ => panic!("expected TokenUsage"),
        }
    }

    #[test]
    fn test_build_history_user_message() {
        let history = vec![("user".to_string(), "Hello".to_string())];
        let msgs = LlmProvider::build_history(&history);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_build_history_assistant_message() {
        let history = vec![("assistant".to_string(), "Hi there".to_string())];
        let msgs = LlmProvider::build_history(&history);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_build_history_alternating() {
        let history = vec![
            ("user".to_string(), "Hello".to_string()),
            ("assistant".to_string(), "Hi".to_string()),
            ("user".to_string(), "How are you?".to_string()),
        ];
        let msgs = LlmProvider::build_history(&history);
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn test_build_history_system_message() {
        let history = vec![("system".to_string(), "Be helpful".to_string())];
        let msgs = LlmProvider::build_history(&history);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            rig::completion::Message::System { content } => assert_eq!(content, "Be helpful"),
            _ => panic!("expected system message"),
        }
    }

    // ── ToolEvent ──

    #[test]
    fn test_tool_event_text() {
        let event = ToolEvent::Text("response chunk".to_string());
        match event {
            ToolEvent::Text(t) => assert_eq!(t, "response chunk"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_tool_event_tool_call() {
        let event = ToolEvent::ToolCall {
            name: "read_file".to_string(),
            args: serde_json::json!({"path": "/tmp/test.txt"}),
        };
        match event {
            ToolEvent::ToolCall { name, args, .. } => {
                assert_eq!(name, "read_file");
                assert_eq!(args["path"], "/tmp/test.txt");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_tool_event_done() {
        let event = ToolEvent::Done {
            reason: DoneReason::Normal,
        };
        assert!(matches!(event, ToolEvent::Done { .. }));
    }

    #[test]
    fn test_tool_event_done_reason_variants() {
        for reason in &[
            DoneReason::Normal,
            DoneReason::LoopTerminated,
            DoneReason::StreamError,
        ] {
            let event = ToolEvent::Done { reason: *reason };
            if let ToolEvent::Done { reason: r } = event {
                assert_eq!(r, *reason);
            } else {
                panic!("expected Done");
            }
        }
    }

    #[test]
    fn test_tool_event_clone() {
        let event = ToolEvent::Text("hello".to_string());
        let cloned = event.clone();
        assert!(matches!(cloned, ToolEvent::Text(t) if t == "hello"));
    }

    #[test]
    fn test_tool_event_debug() {
        let event = ToolEvent::ToolCall {
            name: "test".to_string(),
            args: serde_json::json!({}),
        };
        let debug = format!("{:?}", event);
        assert!(debug.contains("ToolCall"));
        assert!(debug.contains("test"));
    }
}
