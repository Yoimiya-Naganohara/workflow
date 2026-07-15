use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;
use workflow_agent::{
    Agent, AgentEvent, AgentId, Message, MessageType,
    agent_pool::{AgentInfo, AgentPool},
};
use workflow_core::Runtime;
use workflow_role::{Role, RoleId, RolePool};
use workflow_tool::{list_agents::ListAgents, send_message::SendMessage};

#[derive(Debug, Clone, Serialize)]
struct RoleInfo {
    id: String,
    name: String,
    definition: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum UiMessage {
    #[serde(rename = "user")]
    User { text: String },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { text: String },
    #[serde(rename = "tool")]
    Tool {
        text: String,
        result: Option<String>,
    },
    #[serde(rename = "error")]
    Error { text: String },
}

#[derive(Debug, Clone, Serialize)]
struct Snapshot {
    agents: Vec<AgentInfo>,
    selected: Option<AgentId>,
    messages: Vec<UiMessage>,
}

struct ChatLog {
    messages: HashMap<AgentId, Vec<UiMessage>>,
    buffer: HashMap<AgentId, String>,
}

struct AgentConfig {
    api_key: String,
    base_url: String,
    model: String,
    tool_handle: Option<rig::tool::server::ToolServerHandle>,
}

struct AppState {
    runtime: Runtime,
    chat: Arc<RwLock<ChatLog>>,
    role_pool: RolePool,
    agent_config: AgentConfig,
    next_id: AtomicU32,
}

async fn subscribe_agent(
    agent: Arc<Agent>,
    id: AgentId,
    app: AppHandle,
    chat: Arc<RwLock<ChatLog>>,
) {
    let mut rx = agent.receiver();
    while let Ok(ev) = rx.recv().await {
        match ev {
            AgentEvent::Text(t) => {
                let mut cs = chat.write().await;
                let buf = cs.buffer.entry(id).or_default();
                buf.push_str(&t);
                let text = buf.clone();
                drop(cs);
                let _ = app.emit("text", (id, text));
            }
            AgentEvent::Reasoning(t) => {
                let mut cs = chat.write().await;
                let msgs = cs.messages.entry(id).or_default();
                if msgs
                    .last()
                    .map_or(false, |m| matches!(m, UiMessage::Thinking { .. }))
                {
                    if let Some(UiMessage::Thinking { text: ref mut last }) = msgs.last_mut() {
                        last.push_str(&t);
                    }
                } else {
                    msgs.push(UiMessage::Thinking { text: t });
                }
                let _ = app.emit("tick", ());
            }
            AgentEvent::ToolCall { name } => {
                let mut cs = chat.write().await;
                let text = cs.buffer.remove(&id);
                let msgs = cs.messages.entry(id).or_default();
                if let Some(t) = text {
                    msgs.push(UiMessage::Text { text: t });
                }
                msgs.push(UiMessage::Tool {
                    text: name,
                    result: None,
                });
                let _ = app.emit("tick", ());
            }
            AgentEvent::ToolResult { name, result } => {
                let mut cs = chat.write().await;
                let msgs = cs.messages.entry(id).or_default();
                let idx = msgs.iter().rposition(
                    |m| matches!(m, UiMessage::Tool { text, result: None } if *text == name),
                );
                if let Some(i) = idx {
                    msgs[i] = UiMessage::Tool {
                        text: name,
                        result: Some(result),
                    };
                } else {
                    msgs.push(UiMessage::Tool {
                        text: name,
                        result: Some(result),
                    });
                }
                let _ = app.emit("tick", ());
            }
            AgentEvent::TurnComplete => {
                let mut cs = chat.write().await;
                if let Some(text) = cs.buffer.remove(&id) {
                    cs.messages
                        .entry(id)
                        .or_default()
                        .push(UiMessage::Text { text });
                }
                let _ = app.emit("tick", ());
            }
            AgentEvent::Error(e) => {
                chat.write()
                    .await
                    .messages
                    .entry(id)
                    .or_default()
                    .push(UiMessage::Error { text: e });
                let _ = app.emit("tick", ());
            }
        }
    }
}

async fn watch_agents(pool: Arc<AgentPool>, app: AppHandle, chat: Arc<RwLock<ChatLog>>) {
    let mut seen: Vec<AgentId> = Vec::new();
    loop {
        for info in pool.list_agents().await {
            if !seen.contains(&info.id) {
                seen.push(info.id);
                if let Some(agent) = pool.get_agent(&info.id).await {
                    tokio::spawn(subscribe_agent(agent, info.id, app.clone(), chat.clone()));
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tauri::command]
async fn ping() -> String {
    "pong".into()
}

#[tauri::command]
async fn snapshot(app: AppHandle, selected: Option<AgentId>) -> Result<Snapshot, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.read().await;
    let agents = s.runtime.pool().list_agents().await;
    let cs = s.chat.read().await;
    let sel = selected.or_else(|| agents.first().map(|a| a.id));
    let messages = sel
        .and_then(|id| cs.messages.get(&id).cloned())
        .unwrap_or_default();
    Ok(Snapshot {
        agents,
        selected: sel,
        messages,
    })
}

#[tauri::command]
async fn send(app: AppHandle, target: AgentId, text: String) -> Result<Snapshot, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.read().await;

    let agent = s
        .runtime
        .pool()
        .get_agent(&target)
        .await
        .ok_or_else(|| format!("agent {} not found", target))?;

    s.chat
        .write()
        .await
        .messages
        .entry(target)
        .or_default()
        .push(UiMessage::User { text: text.clone() });

    let _ = agent
        .sender()
        .send(MessageType::Data(Message::User(text)))
        .await;

    let agents = s.runtime.pool().list_agents().await;
    let cs = s.chat.read().await;
    let sel = Some(target);
    let messages = sel
        .and_then(|id| cs.messages.get(&id).cloned())
        .unwrap_or_default();
    Ok(Snapshot {
        agents,
        selected: sel,
        messages,
    })
}

#[tauri::command]
async fn create_agent(app: AppHandle, role_name: String) -> Result<Vec<AgentInfo>, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.write().await;

    let id = s.next_id.fetch_add(1, Ordering::SeqCst);
    let pool = s.runtime.pool();

    let role = s
        .role_pool
        .get(&RoleId::from(role_name.clone()))
        .cloned()
        .unwrap_or_else(|| s.role_pool.get(&RoleId::default()).cloned().unwrap());

    let cfg = &s.agent_config;

    if cfg.api_key.is_empty() {
        pool.add_agent(Arc::new(Agent::new_no_model(id, role.name().to_owned())))
            .await
            .map_err(|e| e.to_string())?;
    } else {
        use rig::{
            client::CompletionClient, memory::InMemoryConversationMemory,
            providers::openai::CompletionsClient,
        };

        let client = CompletionsClient::builder()
            .base_url(&cfg.base_url)
            .api_key(&cfg.api_key)
            .build()
            .map_err(|e| e.to_string())?;

        let handle = cfg
            .tool_handle
            .clone()
            .ok_or_else(|| "ToolServer not initialized".to_string())?;

        let rig_agent = client
            .agent(&cfg.model)
            .tool_server_handle(handle)
            .memory(InMemoryConversationMemory::new())
            .conversation("")
            .preamble(role.definition())
            .build();

        pool.add_agent(Arc::new(Agent::new(id, role.name().to_owned(), rig_agent)))
            .await
            .map_err(|e| e.to_string())?;
    }

    let agents = pool.list_agents().await;
    Ok(agents)
}

#[tauri::command]
async fn remove_agent(app: AppHandle, id: AgentId) -> Result<Vec<AgentInfo>, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.read().await;
    s.runtime.pool().remove_agent(&id).await;
    let agents = s.runtime.pool().list_agents().await;
    Ok(agents)
}

#[tauri::command]
async fn get_roles(app: AppHandle) -> Result<Vec<RoleInfo>, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.read().await;
    let roles = s.role_pool.list();
    Ok(roles
        .into_iter()
        .map(|r| RoleInfo {
            id: r.name().to_owned(),
            name: r.name().to_owned(),
            definition: r.definition().to_owned(),
        })
        .collect())
}

#[tauri::command]
async fn add_role(
    app: AppHandle,
    name: String,
    definition: String,
) -> Result<Vec<RoleInfo>, String> {
    let state = app.state::<RwLock<AppState>>();
    let mut s = state.write().await;
    s.role_pool.add(
        RoleId::from(name.clone()),
        Role::new(name, definition, vec![]),
    );
    let roles = s.role_pool.list();
    Ok(roles
        .into_iter()
        .map(|r| RoleInfo {
            id: r.name().to_owned(),
            name: r.name().to_owned(),
            definition: r.definition().to_owned(),
        })
        .collect())
}

#[tauri::command]
async fn get_agent_messages(app: AppHandle, id: AgentId) -> Result<Vec<UiMessage>, String> {
    let state = app.state::<RwLock<AppState>>();
    let s = state.read().await;
    let cs = s.chat.read().await;
    Ok(cs.messages.get(&id).cloned().unwrap_or_default())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let rt = tauri::async_runtime::handle();
            let runtime = Runtime::new();
            let pool = Arc::clone(runtime.pool());
            let role_pool = RolePool::default();

            let (tool_handle, api_key, base_url, model_name) = {
                let api_key = std::env::var("OPENCODE_API_KEY").unwrap_or_default();
                let base_url = if api_key.is_empty() {
                    String::new()
                } else {
                    "https://opencode.ai/zen/v1".to_string()
                };
                let model_name = "big-pickle".to_string();

                let handle = if !api_key.is_empty() {
                    use rig::tool::server::ToolServer;
                    Some(
                        ToolServer::new()
                            .tool(SendMessage::new(Arc::clone(&pool)))
                            .tool(ListAgents::new(Arc::clone(&pool)))
                            .run(),
                    )
                } else {
                    None
                };

                (handle, api_key, base_url, model_name)
            };

            let role_name = role_pool
                .get(&RoleId::default())
                .map(|r| r.name().to_owned())
                .unwrap_or_else(|| "planner".to_string());
            let role_def = role_pool
                .get(&RoleId::default())
                .map(|r| r.definition().to_owned())
                .unwrap_or_default();

            let p = Arc::clone(&pool);
            let tk = tool_handle.clone();
            let ak = api_key.clone();
            let bu = base_url.clone();
            let mn = model_name.clone();
            rt.spawn(async move {
                if ak.is_empty() {
                    eprintln!("[setup] OPENCODE_API_KEY not set — no-model agent");
                    p.add_agent(Arc::new(Agent::new_no_model(0, role_name)))
                        .await
                        .unwrap();
                } else {
                    use rig::{
                        client::CompletionClient, memory::InMemoryConversationMemory,
                        providers::openai::CompletionsClient,
                    };

                    let client = CompletionsClient::builder()
                        .base_url(&bu)
                        .api_key(&ak)
                        .build()
                        .unwrap();

                    let rig_agent = client
                        .agent(&mn)
                        .tool_server_handle(tk.unwrap())
                        .memory(InMemoryConversationMemory::new())
                        .conversation("")
                        .preamble(&role_def)
                        .build();

                    p.add_agent(Arc::new(Agent::new(0, role_name, rig_agent)))
                        .await
                        .unwrap();
                }
            });

            let chat = Arc::new(RwLock::new(ChatLog {
                messages: HashMap::new(),
                buffer: HashMap::new(),
            }));

            app.manage(RwLock::new(AppState {
                runtime,
                chat: chat.clone(),
                role_pool: role_pool.clone(),
                agent_config: AgentConfig {
                    api_key,
                    base_url,
                    model: model_name,
                    tool_handle,
                },
                next_id: AtomicU32::new(1),
            }));

            let handle = app.handle().clone();
            rt.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                watch_agents(pool, handle, chat).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            snapshot,
            send,
            create_agent,
            remove_agent,
            get_roles,
            add_role,
            get_agent_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
