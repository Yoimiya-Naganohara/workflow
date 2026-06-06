use super::*;
use anyhow::Result;
use async_stream::stream;
use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, StreamingResult};
use rig::completion::message::{AssistantContent, UserContent};
use rig::message::Text;
use rig::streaming::{StreamedAssistantContent, StreamingChat};
use rig::OneOrMany;

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
            Self::OpenAi(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Anthropic(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Cohere(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Gemini(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Mistral(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Ollama(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Llamafile(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Azure(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
            Self::Copilot(c) => {
                let agent = c
                    .agent(model)
                    .preamble(system)
                    .temperature(0.7)
                    .max_tokens(4000)
                    .build();
                Ok(Self::wrap_chat_stream(
                    agent.stream_chat(message, history).await,
                ))
            }
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
