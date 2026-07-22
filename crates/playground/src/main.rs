use rig::{
    client::CompletionClient,
    message::{Reasoning, ReasoningContent},
    providers::openai::CompletionsClient,
};
#[tokio::main]
async fn main() {
    let client = CompletionsClient::builder()
        .base_url("https://opencode.ai/zed/v1")
        .api_key("")
        .build()
        .expect("failed to build OpenAI-compatible client");
    let agent = client.agent("big-pickle").preamble("preamble").build();
    let response = agent.runner("hi").run().await.expect("LLM call failed");
    dbg!(response.output);
    let Some(messages) = response.messages else { return; };
    messages.iter().for_each(|msg| match msg {
        rig::message::Message::Assistant { id, content } => {
            content.iter().for_each(|c| match c {
                rig::message::AssistantContent::Reasoning(Reasoning { id, content, .. }) => {
                    for content in content.iter() {
                        match content {
                            ReasoningContent::Text { text, .. } => {
                                println!("{}", text)
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            });
        }
        _ => {}
    });
}
