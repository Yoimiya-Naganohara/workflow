use super::*;
use anyhow::Result;
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
    ($client:expr, $model:expr, $system:expr) => {
        $client
            .agent($model)
            .preamble($system)
            .temperature(crate::core::types::DEFAULT_TEMPERATURE)
            .max_tokens(crate::core::types::DEFAULT_MAX_TOKENS)
            .build()
    };
}

/// Per-provider match arm for `chat_with_tools_stream_mcp`.
///
/// Uses `ToolServerHandle` so tools can be added/removed at runtime
/// without recompiling.
macro_rules! mcp_stream_arm {
    ($client:expr, $model:expr, $system:expr, $handle:expr, $msg:expr, $history:expr) => {{
        let agent = $client
            .agent($model)
            .preamble($system)
            .temperature(crate::core::types::DEFAULT_TEMPERATURE)
            .max_tokens(crate::core::types::DEFAULT_MAX_TOKENS)
            .tool_server_handle($handle.clone())
            .build();
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

    pub async fn chat_stream(&self, model: &str, system: &str, message: &str) -> Result<ChatStream> {
        self.do_chat_stream(model, system, message, &[]).await
    }

    pub async fn chat_stream_with_history(
        &self,
        model: &str,
        system: &str,
        message: &str,
        history: &[(String, String)],
    ) -> Result<ChatStream> {
        self.do_chat_stream(model, system, message, history).await
    }

    /// Stream a chat response with MCP tool-calling capability.
    ///
    /// Uses a [`ToolServerHandle`] so tools can be added or removed at runtime
    /// without recompiling the agent. Tool calls are yielded as
    /// `ToolEvent::ToolCall` events alongside text chunks.
    pub async fn chat_with_tools_stream_mcp(
        &self,
        model: &str,
        system: &str,
        message: &str,
        history: &[(String, String)],
        tool_server: &ToolServerHandle,
    ) -> Result<ToolChatStream> {
        let history = Self::build_history(history);
        match self {
            Self::OpenAi(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Anthropic(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Cohere(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Gemini(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Mistral(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Ollama(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Llamafile(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Azure(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
            Self::Copilot(c) => mcp_stream_arm!(c, model, system, tool_server, message, history),
        }
    }

    fn build_history(history: &[(String, String)]) -> Vec<rig::completion::Message> {
        history
            .iter()
            .map(|(role, content)| match role.as_str() {
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
    ) -> Result<ChatStream> {
        let history = Self::build_history(history);
        match self {
            Self::OpenAi(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Anthropic(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Cohere(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Gemini(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Mistral(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Ollama(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Llamafile(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Azure(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
            )),
            Self::Copilot(c) => Ok(Self::wrap_chat_stream(
                build_chat_agent!(c, model, system).stream_chat(message, history).await,
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
                        let args = tool_call.function.arguments.clone();
                        yield ToolEvent::ToolCall {
                            name: tool_name,
                            args,
                            result: String::new(),
                        };
                    }
                    Ok(MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::Reasoning(reasoning),
                    )) => {
                        // Surface reasoning as text so user can see chain-of-thought
                        for block in reasoning.content {
                            if let rig::message::ReasoningContent::Text { text, .. } = block {
                                yield ToolEvent::Text(text);
                            }
                        }
                    }
                    Ok(MultiTurnStreamItem::CompletionCall(call)) => {
                        if let Some(usage) = call.usage {
                            yield ToolEvent::TokenUsage {
                                input: usage.input_tokens as u32,
                                output: usage.output_tokens as u32,
                            };
                        }
                    }
                    Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                        let usage = response.usage();
                        if usage.input_tokens > 0 || usage.output_tokens > 0 {
                            yield ToolEvent::TokenUsage {
                                input: usage.input_tokens as u32,
                                output: usage.output_tokens as u32,
                            };
                        }
                        yield ToolEvent::Done;
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        yield ToolEvent::Text(err.to_string());
                        yield ToolEvent::Done;
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
        let event = ToolEvent::TokenUsage { input: 150, output: 75 };
        match event {
            ToolEvent::TokenUsage { input, output } => {
                assert_eq!(input, 150);
                assert_eq!(output, 75);
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
    fn test_build_history_unknown_role_defaults_to_assistant() {
        let history = vec![("system".to_string(), "Be helpful".to_string())];
        let msgs = LlmProvider::build_history(&history);
        assert_eq!(msgs.len(), 1);
        // Unknown role should default to assistant
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
            result: String::new(),
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
        let event = ToolEvent::Done;
        assert!(matches!(event, ToolEvent::Done));
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
            result: "ok".to_string(),
        };
        let debug = format!("{:?}", event);
        assert!(debug.contains("ToolCall"));
        assert!(debug.contains("test"));
    }
}
