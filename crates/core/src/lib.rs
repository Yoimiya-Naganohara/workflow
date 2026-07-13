use std::{
    env::var,
    io::{Write, stdin, stdout},
    num::NonZeroUsize,
    sync::Arc,
};

use rig::{
    client::CompletionClient, memory::InMemoryConversationMemory,
    providers::openai::CompletionsClient, tool::server::ToolServer,
};
use tokio::spawn;
use workflow_agent::{Agent, AgentId, Message, MessageType, agent_pool::AgentPool};
use workflow_role::{RoleId, RolePool};
use workflow_tool::SendMessage;

pub struct Runtime {
    agent_pool: Arc<AgentPool>,
}
impl Runtime {
    pub fn new() -> Self {
        // Create the shared agent pool first so the tool can reference it.
        let agent_pool = Arc::new(AgentPool::new(NonZeroUsize::new(100).unwrap()));

        Self { agent_pool }
    }
    pub async fn run(&self) {
        // todo: use configuration or persistence
        let base_url = "https://opencode.ai/zen/v1";
        let api_key = var("OPENCODE_API_KEY").unwrap_or_default();
        let model = "big-pickle";
        let client = CompletionsClient::builder()
            .base_url(base_url)
            .api_key(api_key)
            .build()
            .unwrap();
        // Register tools on the ToolServer before running it.
        // Any tool that needs the pool gets a clone of the Arc.
        let handle = ToolServer::new()
            .tool(SendMessage::new(Arc::clone(&self.agent_pool)))
            .run();
        let role_pool = RolePool::default();
        let role = role_pool.get(&RoleId::default());
        let preamble = role.unwrap().definition();
        let role = role.unwrap().name();
        let id = "";

        let agent = client
            .agent(model)
            .tool_server_handle(handle)
            .memory(InMemoryConversationMemory::new())
            .conversation(id)
            .preamble(&preamble)
            .build();
        let agent = Arc::new(Agent::new(AgentId::default(), role.to_owned(), agent));
        self.agent_pool.add_agent(agent).await.unwrap();
        loop {
            let mut buf = String::new();
            stdin().read_line(&mut buf).unwrap();
            let mut agent = self
                .agent_pool
                .get_agent(&AgentId::default())
                .await
                .unwrap();
            agent
                .sender()
                .send(Message::Data(MessageType::User(buf)))
                .await;
            let mut receiver = agent.receiver();
            spawn(async move {
                while let Ok(response) = receiver.recv().await {
                    match response {
                        workflow_agent::AgentEvent::Reasoning(text) => {
                            print!("{}", text);
                        }
                        workflow_agent::AgentEvent::Text(text) => {
                            print!("{}", text);
                        }
                        _ => {}
                    }
                    stdout().flush().unwrap();
                }
            });
        }
    }
}
