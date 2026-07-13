use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;
use workflow_agent::{
    Agent, AgentEvent, AgentId, Message, MessageType,
    agent_pool::AgentPool,
};
use workflow_core::Runtime;
use workflow_role::{RoleId, RolePool};
use workflow_tool::{list_agents::ListAgents, send_message::SendMessage};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "role")]
enum UiMessage {
    #[serde(rename = "user")]
    User { text: String },
    #[serde(rename = "assistant")]
    Assistant { text: String },
    #[serde(rename = "thinking")]
    Thinking { text: String },
    #[serde(rename = "tool")]
    Tool { text: String },
    #[serde(rename = "error")]
    Error { text: String },
}

#[derive(Debug, Clone, Serialize)]
struct Snapshot {
    agents: Vec<workflow_agent::agent_pool::AgentInfo>,
    selected: Option<AgentId>,
    messages: Vec<UiMessage>,
}

struct ChatLog {
    messages: HashMap<AgentId, Vec<UiMessage>>,
    buffer: HashMap<AgentId, String>,
}

struct AppState {
    runtime: Runtime,
    chat: Arc<RwLock<ChatLog>>,
}

async fn subscribe_agent(
    agent: Arc<workflow_agent::Agent>,
    id: AgentId,
    app: AppHandle,
    chat: Arc<RwLock<ChatLog>>,
) {
    let mut rx = agent.receiver();
    while let Ok(ev) = rx.recv().await {
        match ev {
            AgentEvent::Text(t) => {
                chat.write().await.buffer.entry(id).or_default().push_str(&t);
            }
            AgentEvent::Reasoning(t) => {
                chat.write().await.messages.entry(id).or_default()
                    .push(UiMessage::Thinking { text: t });
            }
            AgentEvent::ToolCall { name } => {
                chat.write().await.messages.entry(id).or_default()
                    .push(UiMessage::Tool { text: name });
            }
            AgentEvent::TurnComplete => {
                let mut cs = chat.write().await;
                if let Some(text) = cs.buffer.remove(&id) {
                    cs.messages.entry(id).or_default()
                        .push(UiMessage::Assistant { text });
                }
            }
            AgentEvent::Error(e) => {
                chat.write().await.messages.entry(id).or_default()
                    .push(UiMessage::Error { text: e });
            }
            _ => {}
        }
        let _ = app.emit("tick", ());
    }
}

async fn watch_agents(pool: Arc<AgentPool>, app: AppHandle, chat: Arc<RwLock<ChatLog>>) {
    let mut seen = 0u32;
    loop {
        for info in pool.list_agents().await {
            if info.id > seen {
                seen = info.id;
                if let Some(agent) = pool.get_agent(&info.id).await {
                    tokio::spawn(subscribe_agent(agent, info.id, app.clone(), chat.clone()));
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tauri::command]
async fn snapshot(app: AppHandle) -> Result<Snapshot, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.read().await;
    let agents = s.runtime.pool().list_agents().await;
    let cs = s.chat.read().await;
    let selected = agents.first().map(|a| a.id);
    let messages = selected
        .and_then(|id| cs.messages.get(&id).cloned())
        .unwrap_or_default();
    Ok(Snapshot { agents, selected, messages })
}

#[tauri::command]
async fn send(app: AppHandle, target: AgentId, text: String) -> Result<Snapshot, String> {
    let agent = {
        let state = app.state::<RwLock<AppState>>();
        let s = state.read().await;
        s.runtime.pool().get_agent(&target).await.ok_or("agent not found".to_string())?
    };
    {
        let state = app.state::<RwLock<AppState>>();
        let s = state.read().await;
        s.chat
            .write()
            .await
            .messages
            .entry(target)
            .or_default()
            .push(UiMessage::User { text: text.clone() });
    }
    let _ = agent
        .sender()
        .send(Message::Data(MessageType::User(text)))
        .await;
    snapshot(app).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let rt = tauri::async_runtime::handle();
            let runtime = Runtime::new();
            let pool = Arc::clone(runtime.pool());

            let p = Arc::clone(&pool);
            rt.spawn(async move {
                let role_pool = RolePool::default();
                let role = role_pool.get(&RoleId::default()).unwrap();
                let preamble = role.definition();
                let role_name = role.name().to_owned();

                let base_url = std::env::var("OPENCODE_API_KEY")
                    .map(|_| "https://opencode.ai/zen/v1".to_string())
                    .unwrap_or_default();
                let api_key = std::env::var("OPENCODE_API_KEY").unwrap_or_default();

                use rig::{
                    client::CompletionClient, memory::InMemoryConversationMemory,
                    providers::openai::CompletionsClient, tool::server::ToolServer,
                };

                let client = CompletionsClient::builder()
                    .base_url(&base_url)
                    .api_key(&api_key)
                    .build()
                    .unwrap();

                let handle = ToolServer::new()
                    .tool(SendMessage::new(Arc::clone(&p)))
                    .tool(ListAgents::new(Arc::clone(&p)))
                    .run();

                let agent = client
                    .agent("big-pickle")
                    .tool_server_handle(handle)
                    .memory(InMemoryConversationMemory::new())
                    .conversation("")
                    .preamble(&preamble)
                    .build();

                p.add_agent(Arc::new(Agent::new(AgentId::default(), role_name, agent)))
                    .await
                    .unwrap();
            });

            let chat = Arc::new(RwLock::new(ChatLog {
                messages: HashMap::new(),
                buffer: HashMap::new(),
            }));
            app.manage(RwLock::new(AppState {
                runtime,
                chat: chat.clone(),
            }));

            let handle = app.handle().clone();
            rt.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                watch_agents(pool, handle, chat).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![snapshot, send])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
