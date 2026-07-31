use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_decoration::WebviewWindowExt;
use tauri_plugin_dialog::DialogExt;
use workflow_config::{ProjectConfig, UserConfig};
use workflow_core::{
    Runtime, RuntimeConfig, RuntimeSnapshot, WorkflowEvent,
    sessions::{SessionMeta, Sessions},
};
use workflow_mcp::config::McpConfigSource;
use workflow_mcp::{McpConnectionInfo, McpServerConfig};
use workflow_providers::service::ProviderService;

const WORKFLOW_DIR: &str = ".workflow";

struct AppState {
    runtime: Mutex<Option<Arc<Runtime>>>,
    sessions: Mutex<Sessions>,
    active_session: Mutex<Option<u32>>,
    /// Last [`RuntimeConfig`] used, so sessions can be rebuilt with a
    /// different project binding.
    runtime_config: Mutex<Option<RuntimeConfig>>,
    service: Mutex<ProviderService>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderModel {
    pub id: String,
    pub name: String,
    pub supports_tools: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderEntry {
    pub id: String,
    pub name: String,
    pub api_url: Option<String>,
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UiEvent {
    AgentAdded {
        agent_id: u32,
    },
    AgentRemoved {
        agent_id: u32,
    },
    AgentStopped {
        agent_id: u32,
    },
    AgentOutput {
        agent_id: u32,
    },
    TranscriptChanged {
        agent_id: u32,
    },
    RolesChanged,
    ResyncRequired,
    Error {
        message: String,
    },
    McpConnected {
        server: String,
        tool_count: usize,
    },
    McpDisconnected {
        server: String,
    },
    McpToolNeedsApproval {
        request_id: String,
        server: String,
        tool: String,
        arguments: serde_json::Value,
    },
}

impl From<WorkflowEvent> for UiEvent {
    fn from(event: WorkflowEvent) -> Self {
        match event {
            WorkflowEvent::AgentAdded(agent) => Self::AgentAdded { agent_id: agent.id },
            WorkflowEvent::AgentRemoved(agent_id) => Self::AgentRemoved { agent_id },
            WorkflowEvent::AgentStopped(agent_id) => Self::AgentStopped { agent_id },
            WorkflowEvent::AgentOutput { agent_id, .. } => Self::AgentOutput { agent_id },
            WorkflowEvent::TranscriptChanged(agent_id) => Self::TranscriptChanged { agent_id },
            WorkflowEvent::RolesChanged => Self::RolesChanged,
            WorkflowEvent::ResyncRequired => Self::ResyncRequired,
            WorkflowEvent::McpConnected { server, tool_count } => {
                Self::McpConnected { server, tool_count }
            }
            WorkflowEvent::McpDisconnected { server } => Self::McpDisconnected { server },
            WorkflowEvent::McpToolNeedsApproval {
                request_id,
                server,
                tool,
                arguments,
            } => Self::McpToolNeedsApproval {
                request_id,
                server,
                tool,
                arguments,
            },
        }
    }
}

fn entry_from_provider(p: &workflow_providers::ProviderInfo) -> ProviderEntry {
    let models = p
        .models
        .values()
        .map(|m| ProviderModel {
            id: m.id.clone(),
            name: m.name.clone(),
            supports_tools: m.tool_call.unwrap_or(false),
        })
        .collect();
    ProviderEntry {
        id: p.id.clone(),
        name: p.name.clone(),
        api_url: p.api.clone(),
        models,
    }
}

#[tauri::command]
async fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderEntry>, String> {
    {
        let guard = state.service.lock().await;
        let entries: Vec<ProviderEntry> = guard
            .store()
            .providers()
            .iter()
            .map(entry_from_provider)
            .collect();
        if !entries.is_empty() {
            return Ok(entries);
        }
    }

    // Load from cache (lock released before async call)
    let mut service = ProviderService::new();
    service.initialize().await.map_err(|e| e.to_string())?;
    let entries: Vec<ProviderEntry> = service
        .store()
        .providers()
        .iter()
        .map(entry_from_provider)
        .collect();
    let mut guard = state.service.lock().await;
    *guard = service;
    Ok(entries)
}

#[tauri::command]
async fn fetch_providers(state: State<'_, AppState>) -> Result<Vec<ProviderEntry>, String> {
    let mut service = ProviderService::new();
    service.refresh().await.map_err(|e| e.to_string())?;
    let entries: Vec<ProviderEntry> = service
        .store()
        .providers()
        .iter()
        .map(entry_from_provider)
        .collect();
    let mut guard = state.service.lock().await;
    *guard = service;
    Ok(entries)
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let result = app.dialog().file().blocking_pick_folder();
    match result {
        Some(path) => {
            let path_str = path
                .as_path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

/// Resolve a user-provided project path to a canonical [`ProjectConfig`].
///
/// Accepts a directory or a file (the file's parent is used).
fn resolve_project(path: &str) -> Result<ProjectConfig, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("Project path '{path}' does not exist: {e}"))?;
    if canonical.is_dir() {
        Ok(ProjectConfig::new(canonical.to_string_lossy().to_string()))
    } else {
        let parent = canonical
            .parent()
            .ok_or_else(|| format!("Project path '{path}' has no parent directory"))?;
        Ok(ProjectConfig::new(parent.to_string_lossy().to_string()))
    }
}

#[tauri::command]
async fn configure_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    api_key: String,
    model: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let needs_init = state.service.lock().await.store().providers().is_empty();
    if needs_init {
        let mut svc = ProviderService::new();
        svc.initialize().await.map_err(|e| e.to_string())?;
        *state.service.lock().await = svc;
    }
    let base_url = {
        let guard = state.service.lock().await;
        guard
            .store()
            .providers()
            .iter()
            .find(|p| p.id == provider_id)
            .and_then(|p| p.api.clone())
            .unwrap_or_default()
    };

    let protocol = workflow_config::ProviderProtocol::from_id(&provider_id);
    let provider_config = workflow_config::ProviderConfig {
        id: provider_id,
        name: String::new(),
        protocol,
        base_url,
        api_key,
        models: vec![model.clone()],
        ..Default::default()
    };

    // Resolve project path if provided.
    let project = match project_path {
        Some(path) => Some(resolve_project(&path)?),
        None => None,
    };

    let runtime_config = RuntimeConfig {
        provider: provider_config,
        model,
        agent_capacity: std::num::NonZeroUsize::new(100).expect("100 must be non-zero"),
        project: project.clone(),
    };
    *state.runtime_config.lock().await = Some(runtime_config.clone());
    let runtime = Arc::new(Runtime::try_new(runtime_config).map_err(|e| e.to_string())?);
    spawn_event_bridge(app, Arc::clone(&runtime));
    *state.runtime.lock().await = Some(Arc::clone(&runtime));

    // Name the session after the opened project folder, falling back to
    // "Default" when no project is open.
    let session_name = project
        .as_ref()
        .map(|p| p.name())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Default".to_string());

    let mut sessions = state.sessions.lock().await;
    let active_id = *state.active_session.lock().await;

    // Reconfigured with a new project: rebind the active session to the new
    // runtime, rename it after the folder, and keep the project in sync.
    if let Some(id) = active_id {
        if let Some(session) = sessions.get(id) {
            session.meta.name = session_name;
            session.meta.project = project.as_ref().map(|p| p.path.clone());
            session.runtime = runtime;
            return Ok(());
        }
    }

    // No active session (or a stale id): wrap the runtime in a new session.
    let id = sessions.create_with_runtime(&session_name, runtime);
    *state.active_session.lock().await = Some(id);
    Ok(())
}

#[tauri::command]
async fn approve_mcp_tool(
    state: State<'_, AppState>,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime.mcp().resolve_approval(&request_id, approved).await;
    Ok(())
}

#[tauri::command]
async fn list_mcp_connections(
    state: State<'_, AppState>,
) -> Result<Vec<McpConnectionInfo>, String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    Ok(runtime.mcp().list_connections().await)
}

#[tauri::command]
async fn list_mcp_configs() -> Result<Vec<McpServerConfig>, String> {
    let source = McpConfigSource::new(McpConfigSource::default_path());
    source.load().map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_mcp_server(state: State<'_, AppState>, config: McpServerConfig) -> Result<(), String> {
    // Persist config first
    let source = McpConfigSource::new(McpConfigSource::default_path());
    source
        .add_server(config.clone())
        .map_err(|e| e.to_string())?;
    // Then connect
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .mcp()
        .connect_one(&config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn remove_mcp_server(state: State<'_, AppState>, name: String) -> Result<(), String> {
    // Disconnect first
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    let _ = runtime.mcp().disconnect(&name).await;
    // Then remove from config
    let source = McpConfigSource::new(McpConfigSource::default_path());
    source.remove_server(&name).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
}

#[tauri::command]
async fn get_project(state: State<'_, AppState>) -> Result<Option<ProjectInfo>, String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    Ok(runtime.project().map(|p| ProjectInfo {
        name: p.name(),
        path: p.path.clone(),
    }))
}

#[tauri::command]
async fn snapshot(
    state: State<'_, AppState>,
    selected: Option<u32>,
) -> Result<RuntimeSnapshot, String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    Ok(runtime.snapshot(selected).await)
}

/// Generate a session name from the first user message.
fn message_session_name(message: &str) -> String {
    let cleaned: String = message.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.len() > 55 {
        format!("{}…", &trimmed[..55])
    } else {
        trimmed.to_string()
    }
}

/// Auto-name the active session if it still has a generic name.
async fn auto_name_session(state: &State<'_, AppState>, message: &str) {
    let active_id = *state.active_session.lock().await;
    let Some(id) = active_id else { return };

    // Check if the session has a generic name (read-only lock).
    let needs_naming = {
        let sessions = state.sessions.lock().await;
        sessions.get_ref(id).map_or(false, |s| {
            let n = &s.meta.name;
            n == "New Session" || n == "Default" || n.starts_with("Session ")
        })
    };

    if needs_naming {
        let name = message_session_name(message);
        if !name.is_empty() {
            let mut sessions = state.sessions.lock().await;
            if let Some(s) = sessions.get(id) {
                s.meta.name = name;
            }
        }
    }
}

#[tauri::command]
async fn send(
    state: State<'_, AppState>,
    target: u32,
    text: String,
) -> Result<RuntimeSnapshot, String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    // Auto-name the session from the first user message (before consuming text).
    auto_name_session(&state, &text).await;

    runtime
        .send_message(target, text)
        .await
        .map_err(|error| error.to_string())?;

    Ok(runtime.snapshot(Some(target)).await)
}

#[tauri::command]
async fn stop_agent(state: State<'_, AppState>, target: u32) -> Result<(), String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .stop_agent(target)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn create_agent(
    state: State<'_, AppState>,
    role_name: String,
) -> Result<Vec<workflow_core::AgentInfo>, String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    runtime
        .create_agent(role_name)
        .await
        .map_err(|error| error.to_string())?;
    Ok(runtime.list_agents().await)
}

#[tauri::command]
async fn remove_agent(
    state: State<'_, AppState>,
    id: u32,
) -> Result<Vec<workflow_core::AgentInfo>, String> {
    let runtime = state
        .runtime
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    runtime.remove_agent(id).await;
    Ok(runtime.list_agents().await)
}

#[tauri::command]
fn get_roles(state: State<'_, AppState>) -> Vec<workflow_core::RoleInfo> {
    let runtime = match state.runtime.blocking_lock().clone() {
        Some(r) => r,
        None => return Vec::new(),
    };
    runtime.list_roles()
}

fn roles_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(WORKFLOW_DIR).join("roles.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedRole {
    name: String,
    definition: String,
}

fn default_roles() -> Vec<workflow_core::RoleInfo> {
    vec![
        workflow_core::RoleInfo {
            id: "planner".into(),
            name: "planner".into(),
            definition: "I should help user to plan".into(),
        },
        workflow_core::RoleInfo {
            id: "executor".into(),
            name: "executor".into(),
            definition: "I should execute the plan".into(),
        },
    ]
}

fn read_saved_roles() -> Vec<SavedRole> {
    let path = roles_path();
    if !path.exists() {
        return Vec::new();
    }
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_saved_roles(roles: &[SavedRole]) {
    if let Ok(data) = serde_json::to_string_pretty(roles) {
        let path = roles_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, data);
    }
}

fn merge_roles(
    defaults: Vec<workflow_core::RoleInfo>,
    saved: Vec<SavedRole>,
) -> Vec<workflow_core::RoleInfo> {
    let mut names: HashSet<String> = defaults.iter().map(|r| r.name.clone()).collect();
    let mut merged = defaults;
    for s in saved {
        if names.insert(s.name.clone()) {
            merged.push(workflow_core::RoleInfo {
                id: s.name.clone(),
                name: s.name,
                definition: s.definition,
            });
        }
    }
    merged
}

#[tauri::command]
fn add_role(
    state: State<'_, AppState>,
    name: String,
    definition: String,
) -> Vec<workflow_core::RoleInfo> {
    if let Some(runtime) = state.runtime.blocking_lock().clone() {
        let roles = runtime.add_role(name.clone(), definition.clone());
        let saved: Vec<SavedRole> = roles
            .iter()
            .map(|r| SavedRole {
                name: r.name.clone(),
                definition: r.definition.clone(),
            })
            .collect();
        write_saved_roles(&saved);
        return roles;
    }

    let mut saved = read_saved_roles();
    if !saved.iter().any(|r| r.name == name) {
        saved.push(SavedRole { name, definition });
    }
    write_saved_roles(&saved);
    merge_roles(default_roles(), saved)
}

#[tauri::command]
fn remove_role(state: State<'_, AppState>, name: String) -> Vec<workflow_core::RoleInfo> {
    // Remove from saved roles
    let mut saved = read_saved_roles();
    saved.retain(|r| r.name != name);
    write_saved_roles(&saved);

    // Remove from runtime if available
    if let Some(runtime) = state.runtime.blocking_lock().clone() {
        return runtime.remove_role(&name);
    }

    merge_roles(default_roles(), saved)
}

#[tauri::command]
fn load_roles(state: State<'_, AppState>) -> Vec<workflow_core::RoleInfo> {
    if let Some(runtime) = state.runtime.blocking_lock().clone() {
        let saved = read_saved_roles();
        for role in &saved {
            let existing = runtime.list_roles();
            if !existing.iter().any(|r| r.name == role.name) {
                runtime.add_role(role.name.clone(), role.definition.clone());
            }
        }
        let all = runtime.list_roles();
        let saved: Vec<SavedRole> = all
            .iter()
            .map(|r| SavedRole {
                name: r.name.clone(),
                definition: r.definition.clone(),
            })
            .collect();
        write_saved_roles(&saved);
        return all;
    }

    let saved = read_saved_roles();
    merge_roles(default_roles(), saved)
}

#[tauri::command]
fn save_config(config: UserConfig) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn load_config() -> Result<Option<UserConfig>, String> {
    UserConfig::load().map_err(|e| e.to_string())
}

// ============================================================================
//  Session commands
// ============================================================================

/// Return the currently active runtime, or an error if none is configured.
#[allow(dead_code)]
async fn active_runtime(state: &State<'_, AppState>) -> Result<Arc<Runtime>, String> {
    if let Some(runtime) = state.runtime.lock().await.clone() {
        return Ok(runtime);
    }
    Err("runtime not configured".to_string())
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, String> {
    let sessions = state.sessions.lock().await;
    let metas: Vec<SessionMeta> = sessions.list().into_iter().cloned().collect();
    Ok(metas)
}

#[tauri::command]
async fn create_session(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<SessionMeta, String> {
    // Default the name to the active project's folder name.
    let folder_name = state
        .runtime
        .lock()
        .await
        .clone()
        .and_then(|r| r.project().map(|p| p.name()));
    let session_name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or(folder_name)
        .unwrap_or_else(|| "Default".to_string());

    let mut sessions = state.sessions.lock().await;
    let id = sessions
        .create(&session_name)
        .await
        .map_err(|e| e.to_string())?;
    let meta = sessions.get_ref(id).map(|s| s.meta.clone()).unwrap();

    // Update the active runtime to point to this session.
    if let Some(session) = sessions.get_ref(id) {
        *state.runtime.lock().await = Some(session.runtime.clone());
        *state.active_session.lock().await = Some(id);
    }
    Ok(meta)
}

#[tauri::command]
async fn switch_session(state: State<'_, AppState>, id: u32) -> Result<SessionMeta, String> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get_ref(id)
        .ok_or_else(|| format!("session {id} not found"))?;
    let meta = session.meta.clone();
    *state.runtime.lock().await = Some(session.runtime.clone());
    *state.active_session.lock().await = Some(id);
    Ok(meta)
}

#[tauri::command]
async fn delete_session(state: State<'_, AppState>, id: u32) -> Result<(), String> {
    let mut sessions = state.sessions.lock().await;
    sessions.remove(id);

    // If the deleted session was active, clear the runtime.
    if *state.active_session.lock().await == Some(id) {
        *state.runtime.lock().await = None;
        *state.active_session.lock().await = None;
    }

    // Remove the persisted file.
    let path = sessions.dir().join(format!("session_{id}.json"));
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[tauri::command]
async fn rename_session(
    state: State<'_, AppState>,
    id: u32,
    name: String,
) -> Result<SessionMeta, String> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get(id)
        .ok_or_else(|| format!("session {id} not found"))?;
    session.meta.name = name;
    let meta = session.meta.clone();
    Ok(meta)
}

/// Bind a session to a project folder (or unbind with `None`).
///
/// Rebuilds the session's [`Runtime`] with the project so agents see the
/// project context, renames the session after the folder, and updates the
/// active runtime if the session is currently active.
#[tauri::command]
async fn bind_session_project(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: u32,
    project_path: Option<String>,
) -> Result<SessionMeta, String> {
    let runtime_config = state
        .runtime_config
        .lock()
        .await
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;

    // Resolve and apply the new project binding.
    let project = match project_path {
        Some(path) => Some(resolve_project(&path)?),
        None => None,
    };
    let mut runtime_config = runtime_config;
    runtime_config.project = project.clone();

    let runtime = Arc::new(Runtime::try_new(runtime_config).map_err(|e| e.to_string())?);
    spawn_event_bridge(app, Arc::clone(&runtime));

    let session_name = project
        .as_ref()
        .map(|p| p.name())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Default".to_string());

    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| format!("session {session_id} not found"))?;
    session.meta.name = session_name;
    session.meta.project = project.as_ref().map(|p| p.path.clone());
    session.runtime = runtime;
    let meta = session.meta.clone();

    // If the rebound session is active, point the runtime cache at it.
    if *state.active_session.lock().await == Some(session_id) {
        *state.runtime.lock().await = Some(session.runtime.clone());
    }
    Ok(meta)
}

#[tauri::command]
async fn save_sessions(state: State<'_, AppState>) -> Result<u32, String> {
    let sessions = state.sessions.lock().await;
    sessions.save().await.map_err(|e| e.to_string())
}

fn spawn_event_bridge(app: AppHandle, runtime: Arc<Runtime>) {
    tauri::async_runtime::spawn(async move {
        let mut events = runtime.subscribe();
        if let Err(error) = runtime.initialize().await {
            let _ = app.emit(
                "workflow:event",
                UiEvent::Error {
                    message: error.to_string(),
                },
            );
            return;
        }

        'events: loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = app.emit("workflow:event", UiEvent::ResyncRequired);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            // Forward critical events immediately without coalescing.
            let is_critical = matches!(&event, WorkflowEvent::McpToolNeedsApproval { .. });
            if is_critical {
                let _ = app.emit("workflow:event", UiEvent::from(event));
                continue;
            }

            // Non-critical events: coalesce to avoid flooding the UI.
            let mut ui_event = UiEvent::from(event);
            tokio::time::sleep(std::time::Duration::from_millis(32)).await;
            loop {
                match events.try_recv() {
                    Ok(next) => ui_event = UiEvent::from(next),
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                        ui_event = UiEvent::ResyncRequired;
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                        let _ = app.emit("workflow:event", ui_event);
                        break 'events;
                    }
                }
            }
            let _ = app.emit("workflow:event", ui_event);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_decoration::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let service = Mutex::new(ProviderService::new());
            let runtime = Mutex::new(None);
            let sessions = Mutex::new(Sessions::new());
            let active_session = Mutex::new(None);
            let runtime_config = Mutex::new(None);
            app.manage(AppState {
                runtime,
                sessions,
                active_session,
                runtime_config,
                service,
            });

            #[cfg(desktop)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.create_overlay_titlebar();
                    let _ = window.show();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_providers,
            fetch_providers,
            configure_runtime,
            get_project,
            pick_folder,
            snapshot,
            send,
            stop_agent,
            create_agent,
            remove_agent,
            get_roles,
            add_role,
            remove_role,
            save_config,
            load_config,
            load_roles,
            approve_mcp_tool,
            list_mcp_connections,
            list_mcp_configs,
            add_mcp_server,
            remove_mcp_server,
            list_sessions,
            create_session,
            switch_session,
            delete_session,
            rename_session,
            save_sessions,
            bind_session_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
