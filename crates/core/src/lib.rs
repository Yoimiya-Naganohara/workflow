use std::{
    env::var,
    io::{Write, stdin, stdout},
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use rig::{
    client::CompletionClient, memory::InMemoryConversationMemory,
    providers::openai::CompletionsClient, tool::server::ToolServer,
};
use tokio::spawn;
use workflow_agent::{Agent, AgentId, Message, MessageType, agent_pool::AgentPool};
use workflow_role::{RoleId, RolePool};
use workflow_tool::{
    list_agents::ListAgents,
    orchestrate::{AgentFactory, Orchestrate},
    send_message::SendMessage,
};

pub struct Runtime {
    agent_pool: Arc<AgentPool>,
}
impl Runtime {
    pub fn new() -> Self {
        let agent_pool = Arc::new(AgentPool::new(NonZeroUsize::new(100).unwrap()));
        Self { agent_pool }
    }

    pub fn pool(&self) -> &Arc<AgentPool> {
        &self.agent_pool
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
        let role_pool = RolePool::default();
        let handle_cell: Arc<Mutex<Option<rig::tool::server::ToolServerHandle>>> =
            Arc::new(Mutex::new(None));
        let handle = {
            let factory = make_agent_factory(&client, model, &role_pool, Arc::clone(&handle_cell));
            ToolServer::new()
                .tool(SendMessage::new(Arc::clone(&self.agent_pool)))
                .tool(ListAgents::new(Arc::clone(&self.agent_pool)))
                .tool(Orchestrate::new(Arc::clone(&self.agent_pool), factory))
                .run()
        };
        *handle_cell.lock().unwrap() = Some(handle.clone());

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

fn make_agent_factory(
    client: &CompletionsClient,
    model: &str,
    role_pool: &RolePool,
    handle_cell: Arc<Mutex<Option<rig::tool::server::ToolServerHandle>>>,
) -> AgentFactory {
    let client = client.clone();
    let model = model.to_owned();
    let role_pool = role_pool.clone();
    Arc::new(move |id, role| {
        let handle = handle_cell
            .lock()
            .unwrap()
            .clone()
            .expect("ToolServerHandle not set — run() must complete first");
        let role_def = role_pool
            .get(&RoleId::from(role.clone()))
            .unwrap_or_else(|| role_pool.get(&RoleId::default()).unwrap());
        let preamble = role_def.definition();
        let role_name = role_def.name();
        let rig_agent = client
            .agent(&model)
            .tool_server_handle(handle)
            .memory(InMemoryConversationMemory::new())
            .conversation("")
            .preamble(preamble)
            .build();
        Arc::new(Agent::new(id, role_name.to_owned(), rig_agent))
    })
}
