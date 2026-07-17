use std::{
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use rig::{
    client::CompletionClient, memory::InMemoryConversationMemory,
    providers::openai::CompletionsClient, tool::server::ToolServer,
};
use serde::Serialize;
use tokio::sync::{OnceCell, RwLock as AsyncRwLock, broadcast};
pub use workflow_agent::agent_pool::AgentInfo;
use workflow_agent::{
    Agent, AgentEvent, AgentId, Message,
    agent_pool::{AgentPool, AgentPoolEvent},
};
use workflow_config::*;
use workflow_role::{Role, RoleId, RolePool};
use workflow_tool::{
    RoleChecker,
    list_agents::ListAgents,
    orchestrate::{AgentFactory, Orchestrate},
    send_message::SendMessage,
};

const DEFAULT_AGENT_CAPACITY: usize = 100;
const EVENT_CAPACITY: usize = 256;

type AgentObserver = Arc<dyn Fn(Arc<Agent>) + Send + Sync>;

struct RolePoolChecker(Arc<RwLock<RolePool>>);

impl RoleChecker for RolePoolChecker {
    fn exists(&self, role: &str) -> bool {
        self.0.read().unwrap().get(&role.to_string()).is_some()
    }

    fn list_roles(&self) -> Vec<String> {
        self.0
            .read()
            .unwrap()
            .list()
            .into_iter()
            .map(|r| r.name().to_owned())
            .collect()
    }
}

#[derive(serde::Deserialize)]
struct CreateRoleArgs {
    name: String,
    definition: String,
}

#[derive(Debug, thiserror::Error)]
enum CreateRoleError {
    #[error("failed to create role: {0}")]
    Failed(String),
}

struct CreateRole {
    roles: Arc<RwLock<RolePool>>,
}

impl CreateRole {
    fn new(roles: Arc<RwLock<RolePool>>) -> Self {
        Self { roles }
    }
}

impl rig::tool::Tool for CreateRole {
    const NAME: &'static str = "create_role";

    type Error = CreateRoleError;
    type Args = CreateRoleArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: "create_role".to_string(),
            description: "Create a new agent role with a name and definition. \
                The definition should describe the role's responsibilities and behavior."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique role identifier (e.g. 'coder', 'reviewer')"
                    },
                    "definition": {
                        "type": "string",
                        "description": "Description of the role's responsibilities and behavior"
                    }
                },
                "required": ["name", "definition"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let role_id = RoleId::from(args.name.clone());
        let role = Role::new(args.name, args.definition, vec![]);
        self.roles.write().unwrap().add(role_id, role);
        Ok("Role created successfully".to_string())
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub provider: ProviderConfig,
    pub model: String,
    pub agent_capacity: NonZeroUsize,
}

impl RuntimeConfig {
    pub fn resolve() -> Self {
        let file = FileConfigSource::new(FileConfigSource::default_path());
        let merged = match merge_configs(&[&DefaultConfigSource, &file, &EnvConfigSource]) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

        let provider = merged
            .iter()
            .find(|p| p.requires_api_key() && !p.api_key.is_empty())
            .or_else(|| merged.first());

        let default = Self::default();
        match provider {
            Some(p) => {
                let model = p.models.first().cloned().unwrap_or_else(|| default.model.clone());
                let mut provider = p.clone();
                if provider.models.is_empty() {
                    provider.models.push(model.clone());
                }
                Self {
                    provider,
                    model,
                    agent_capacity: default.agent_capacity,
                }
            }
            None => default,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            provider: ProviderConfig {
                id: "opencode".to_owned(),
                name: "OpenCode AI".to_owned(),
                protocol: ProviderProtocol::OpenAiCompatible,
                base_url: "https://opencode.ai/zen/v1".to_owned(),
                api_key: std::env::var("OPENCODE_API_KEY").unwrap_or_default(),
                models: vec!["big-pickle".to_owned()],
                ..Default::default()
            },
            model: "big-pickle".to_owned(),
            agent_capacity: NonZeroUsize::new(DEFAULT_AGENT_CAPACITY).unwrap(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to configure provider: {0}")]
    Provider(String),
    #[error("agent {0} not found")]
    AgentNotFound(AgentId),
    #[error("failed to create agent: {0}")]
    CreateAgent(String),
    #[error("failed to send message to agent {agent_id}: {message}")]
    SendMessage { agent_id: AgentId, message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleInfo {
    pub id: String,
    pub name: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ConversationMessage {
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
pub struct RuntimeSnapshot {
    pub agents: Vec<AgentInfo>,
    pub selected: Option<AgentId>,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    AgentAdded(AgentInfo),
    AgentRemoved(AgentId),
    AgentOutput {
        agent_id: AgentId,
        event: AgentEvent,
    },
    TranscriptChanged(AgentId),
    ResyncRequired,
}

impl WorkflowEvent {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::AgentOutput {
                event: AgentEvent::Text(text) | AgentEvent::Reasoning(text),
                ..
            } => Some(text),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::AgentOutput {
                event: AgentEvent::Error(error),
                ..
            } => Some(error),
            _ => None,
        }
    }
}

pub struct Runtime {
    agent_pool: Arc<AgentPool>,
    roles: Arc<RwLock<RolePool>>,
    messages: Arc<AsyncRwLock<HashMap<AgentId, Vec<ConversationMessage>>>>,
    events: broadcast::Sender<WorkflowEvent>,
    factory: AgentFactory,
    observer: Arc<RwLock<Option<AgentObserver>>>,
    observed_agents: Mutex<HashSet<AgentId>>,
    next_id: Arc<AtomicU32>,
    initialized: OnceCell<()>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self::try_new(RuntimeConfig::resolve())
            .expect("default runtime configuration must be valid")
    }

    pub fn try_new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let agent_pool = Arc::new(AgentPool::new(config.agent_capacity));
        let roles = Arc::new(RwLock::new(RolePool::default()));
        let next_id = Arc::new(AtomicU32::new(AgentId::default()));
        let observer: Arc<RwLock<Option<AgentObserver>>> = Arc::new(RwLock::new(None));

        let mut client_builder = CompletionsClient::builder()
            .api_key(&config.provider.api_key as &str);
        if !config.provider.base_url.is_empty() {
            client_builder = client_builder.base_url(&config.provider.base_url);
        }
        let client = client_builder
            .build()
            .map_err(|error| RuntimeError::Provider(error.to_string()))?;

        let handle_cell: Arc<Mutex<Option<rig::tool::server::ToolServerHandle>>> =
            Arc::new(Mutex::new(None));
        let factory = make_agent_factory(
            &client,
            &config.model,
            Arc::clone(&roles),
            Arc::clone(&handle_cell),
            Arc::clone(&observer),
        );
        let role_checker: Arc<dyn RoleChecker> = Arc::new(RolePoolChecker(Arc::clone(&roles)));
        let tool_handle = ToolServer::new()
            .tool(SendMessage::new(Arc::clone(&agent_pool)))
            .tool(ListAgents::new(Arc::clone(&agent_pool)))
            .tool(Orchestrate::with_id_allocator(
                Arc::clone(&agent_pool),
                Arc::clone(&factory),
                Arc::clone(&next_id),
                Arc::clone(&role_checker),
            ))
            .tool(CreateRole::new(Arc::clone(&roles)))
            .run();
        *handle_cell.lock().unwrap() = Some(tool_handle);

        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Ok(Self {
            agent_pool,
            roles,
            messages: Arc::new(AsyncRwLock::new(HashMap::new())),
            events,
            factory,
            observer,
            observed_agents: Mutex::new(HashSet::new()),
            next_id,
            initialized: OnceCell::new(),
        })
    }

    pub async fn initialize(self: &Arc<Self>) -> Result<(), RuntimeError> {
        self.initialized
            .get_or_try_init(|| async {
                let weak = Arc::downgrade(self);
                *self.observer.write().unwrap() = Some(Arc::new(move |agent| {
                    if let Some(runtime) = weak.upgrade() {
                        runtime.attach_agent(agent);
                    }
                }));

                self.spawn_pool_event_bridge();

                if self.agent_pool.list_agents().await.is_empty() {
                    self.create_agent(RoleId::default()).await?;
                }
                Ok(())
            })
            .await
            .map(|_| ())
    }

    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        self.agent_pool.list_agents().await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WorkflowEvent> {
        self.events.subscribe()
    }

    pub async fn create_agent(&self, role_id: RoleId) -> Result<AgentInfo, RuntimeError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let agent = (self.factory)(id, role_id);
        let info = AgentInfo {
            id,
            role: agent.role().to_owned(),
            current_task: None,
        };
        self.agent_pool
            .add_agent(agent)
            .await
            .map_err(|error| RuntimeError::CreateAgent(error.to_string()))?;
        Ok(info)
    }

    pub async fn remove_agent(&self, id: AgentId) {
        self.agent_pool.remove_agent(&id).await;
    }

    pub async fn send_message(&self, id: AgentId, text: String) -> Result<(), RuntimeError> {
        let agent = self
            .agent_pool
            .get_agent(&id)
            .await
            .ok_or(RuntimeError::AgentNotFound(id))?;

        self.messages
            .write()
            .await
            .entry(id)
            .or_default()
            .push(ConversationMessage::User { text: text.clone() });
        let _ = self.events.send(WorkflowEvent::TranscriptChanged(id));

        agent
            .send(Message::User(text))
            .await
            .map_err(|error| RuntimeError::SendMessage {
                agent_id: id,
                message: error.to_string(),
            })
    }

    pub async fn snapshot(&self, selected: Option<AgentId>) -> RuntimeSnapshot {
        let agents = self.agent_pool.list_agents().await;
        let selected = selected.or_else(|| agents.first().map(|agent| agent.id));
        let messages = match selected {
            Some(id) => self
                .messages
                .read()
                .await
                .get(&id)
                .cloned()
                .unwrap_or_default(),
            None => Vec::new(),
        };
        RuntimeSnapshot {
            agents,
            selected,
            messages,
        }
    }

    pub fn list_roles(&self) -> Vec<RoleInfo> {
        self.roles
            .read()
            .unwrap()
            .list()
            .into_iter()
            .map(role_info)
            .collect()
    }

    pub fn add_role(&self, name: String, definition: String) -> Vec<RoleInfo> {
        self.roles.write().unwrap().add(
            RoleId::from(name.clone()),
            Role::new(name, definition, Vec::new()),
        );
        self.list_roles()
    }

    fn attach_agent(self: &Arc<Self>, agent: Arc<Agent>) {
        let id = agent.id();
        if !self.observed_agents.lock().unwrap().insert(id) {
            return;
        }

        let mut receiver = agent.receiver();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let Some(runtime) = weak.upgrade() else {
                            break;
                        };
                        runtime.record_agent_event(id, &event).await;
                        let _ = runtime.events.send(WorkflowEvent::AgentOutput {
                            agent_id: id,
                            event,
                        });
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(runtime) = weak.upgrade() {
                            let _ = runtime.events.send(WorkflowEvent::ResyncRequired);
                        } else {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    fn spawn_pool_event_bridge(self: &Arc<Self>) {
        let mut receiver = self.agent_pool.subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let event = match receiver.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(runtime) = weak.upgrade() {
                            let _ = runtime.events.send(WorkflowEvent::ResyncRequired);
                            continue;
                        }
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };

                let Some(runtime) = weak.upgrade() else {
                    break;
                };
                match event {
                    AgentPoolEvent::Added(agent) => {
                        runtime.attach_agent(Arc::clone(&agent));
                        let info = AgentInfo {
                            id: agent.id(),
                            role: agent.role().to_owned(),
                            current_task: agent.current_task().read().await.clone(),
                        };
                        let _ = runtime.events.send(WorkflowEvent::AgentAdded(info));
                    }
                    AgentPoolEvent::Removed(id) => {
                        runtime.observed_agents.lock().unwrap().remove(&id);
                        runtime.messages.write().await.remove(&id);
                        let _ = runtime.events.send(WorkflowEvent::AgentRemoved(id));
                    }
                }
            }
        });
    }

    async fn record_agent_event(&self, id: AgentId, event: &AgentEvent) {
        let mut messages = self.messages.write().await;
        let messages = messages.entry(id).or_default();
        match event {
            AgentEvent::Text(text) => append_text(messages, text, false),
            AgentEvent::Reasoning(text) => append_text(messages, text, true),
            AgentEvent::ToolCall { name, params } => messages.push(ConversationMessage::Tool {
                text: format!("{name}: {params}"),
                result: None,
            }),
            AgentEvent::ToolResult { name, result } => {
                if let Some(ConversationMessage::Tool {
                    result: tool_result,
                    ..
                }) = messages.iter_mut().rev().find(|message| {
                    matches!(message, ConversationMessage::Tool { text, result: None } if text.starts_with(name))
                }) {
                    *tool_result = Some(result.clone());
                } else {
                    messages.push(ConversationMessage::Tool {
                        text: name.clone(),
                        result: Some(result.clone()),
                    });
                }
            }
            AgentEvent::Error(error) => {
                messages.push(ConversationMessage::Error {
                    text: error.clone(),
                });
            }
            AgentEvent::TurnComplete => {}
        }
    }
}

fn append_text(messages: &mut Vec<ConversationMessage>, text: &str, reasoning: bool) {
    match (messages.last_mut(), reasoning) {
        (Some(ConversationMessage::Text { text: current }), false)
        | (Some(ConversationMessage::Thinking { text: current }), true) => current.push_str(text),
        (_, false) => messages.push(ConversationMessage::Text {
            text: text.to_owned(),
        }),
        (_, true) => messages.push(ConversationMessage::Thinking {
            text: text.to_owned(),
        }),
    }
}

fn role_info(role: &Role) -> RoleInfo {
    RoleInfo {
        id: role.name().to_owned(),
        name: role.name().to_owned(),
        definition: role.definition().to_owned(),
    }
}

fn make_agent_factory(
    client: &CompletionsClient,
    model: &str,
    roles: Arc<RwLock<RolePool>>,
    handle_cell: Arc<Mutex<Option<rig::tool::server::ToolServerHandle>>>,
    observer: Arc<RwLock<Option<AgentObserver>>>,
) -> AgentFactory {
    let client = client.clone();
    let model = model.to_owned();
    Arc::new(move |id, requested_role| {
        let handle = handle_cell
            .lock()
            .unwrap()
            .clone()
            .expect("tool server is initialized before agents are created");
        let role = {
            let roles = roles.read().unwrap();
            roles
                .get(&RoleId::from(requested_role.clone()))
                .or_else(|| roles.get(&RoleId::default()))
                .cloned()
                .expect("the default role must exist")
        };
        let agent_role = if requested_role.is_empty() {
            role.name().to_owned()
        } else {
            requested_role
        };
        let rig_agent = client
            .agent(&model)
            .tool_server_handle(handle)
            .memory(InMemoryConversationMemory::new())
            .conversation(id.to_string())
            .preamble(&format!(
                "{}\n{}",
                role.definition(),
                workflow_agent::protocol::A2A_SYSTEM_PROMPT
            ))
            .build();
        let agent = Arc::new(Agent::new(id, agent_role, rig_agent));
        if let Some(observer) = observer.read().unwrap().clone() {
            observer(Arc::clone(&agent));
        }
        agent
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialization_is_idempotent() {
        let runtime = Arc::new(Runtime::new());

        runtime.initialize().await.unwrap();
        runtime.initialize().await.unwrap();

        let agents = runtime.list_agents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, 0);
        assert_eq!(agents[0].role, "planner");
    }

    #[tokio::test]
    async fn manual_agents_share_the_runtime_id_allocator() {
        let runtime = Arc::new(Runtime::new());
        runtime.initialize().await.unwrap();

        let executor = runtime.create_agent("executor".to_owned()).await.unwrap();
        let planner = runtime.create_agent("planner".to_owned()).await.unwrap();

        assert_eq!(executor.id, 1);
        assert_eq!(executor.role, "executor");
        assert_eq!(planner.id, 2);
        assert_eq!(planner.role, "planner");
    }
}
