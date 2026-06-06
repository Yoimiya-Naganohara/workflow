use super::*;
use anyhow::Result;
use async_stream::stream;
use futures::StreamExt;
use rig::OneOrMany;
use rig::agent::{MultiTurnStreamItem, StreamingResult};
use rig::completion::message::{AssistantContent, UserContent};
use rig::message::Text;
use rig::streaming::{StreamedAssistantContent, StreamingChat};

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
                        StreamedAssistantContent::Reasoning(_reasoning),
                    )) => {}
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
}
