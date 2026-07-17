use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_decoration::WebviewWindowExt;
use workflow_config::UserConfig;
use workflow_core::{Runtime, RuntimeConfig, RuntimeSnapshot, WorkflowEvent};
use workflow_providers::service::ProviderService;

struct AppState {
    runtime: Mutex<Option<Arc<Runtime>>>,
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
    AgentAdded { agent_id: u32 },
    AgentRemoved { agent_id: u32 },
    AgentOutput { agent_id: u32 },
    TranscriptChanged { agent_id: u32 },
    ResyncRequired,
    Error { message: String },
}

impl From<WorkflowEvent> for UiEvent {
    fn from(event: WorkflowEvent) -> Self {
        match event {
            WorkflowEvent::AgentAdded(agent) => Self::AgentAdded { agent_id: agent.id },
            WorkflowEvent::AgentRemoved(agent_id) => Self::AgentRemoved { agent_id },
            WorkflowEvent::AgentOutput { agent_id, .. } => Self::AgentOutput { agent_id },
            WorkflowEvent::TranscriptChanged(agent_id) => Self::TranscriptChanged { agent_id },
            WorkflowEvent::ResyncRequired => Self::ResyncRequired,
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
        let guard = state.service.lock().map_err(|e| e.to_string())?;
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
    let mut guard = state.service.lock().map_err(|e| e.to_string())?;
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
    let mut guard = state.service.lock().map_err(|e| e.to_string())?;
    *guard = service;
    Ok(entries)
}

#[tauri::command]
async fn configure_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    api_key: String,
    model: String,
) -> Result<(), String> {
    let base_url = {
        let guard = state.service.lock().map_err(|e| e.to_string())?;
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
    let runtime_config = RuntimeConfig {
        provider: provider_config,
        model,
        agent_capacity: std::num::NonZeroUsize::new(100).unwrap(),
    };
    let runtime = Arc::new(Runtime::try_new(runtime_config).map_err(|e| e.to_string())?);
    spawn_event_bridge(app, Arc::clone(&runtime));
    *state.runtime.lock().map_err(|e| e.to_string())? = Some(runtime);
    Ok(())
}

#[tauri::command]
async fn snapshot(
    state: State<'_, AppState>,
    selected: Option<u32>,
) -> Result<RuntimeSnapshot, String> {
    let runtime = state
        .runtime
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    Ok(runtime.snapshot(selected).await)
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
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "runtime not configured".to_string())?;
    runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    runtime
        .send_message(target, text)
        .await
        .map_err(|error| error.to_string())?;
    Ok(runtime.snapshot(Some(target)).await)
}

#[tauri::command]
async fn create_agent(
    state: State<'_, AppState>,
    role_name: String,
) -> Result<Vec<workflow_core::AgentInfo>, String> {
    let runtime = state
        .runtime
        .lock()
        .map_err(|e| e.to_string())?
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
        .map_err(|e| e.to_string())?
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
    let runtime = match state.runtime.lock().unwrap().clone() {
        Some(r) => r,
        None => return Vec::new(),
    };
    runtime.list_roles()
}

#[tauri::command]
fn add_role(
    state: State<'_, AppState>,
    name: String,
    definition: String,
) -> Vec<workflow_core::RoleInfo> {
    let runtime = match state.runtime.lock().unwrap().clone() {
        Some(r) => r,
        None => return Vec::new(),
    };
    runtime.add_role(name, definition)
}

#[tauri::command]
fn save_config(config: UserConfig) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn load_config() -> Result<Option<UserConfig>, String> {
    UserConfig::load().map_err(|e| e.to_string())
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
            let mut event = match events.recv().await {
                Ok(event) => UiEvent::from(event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => UiEvent::ResyncRequired,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            tokio::time::sleep(std::time::Duration::from_millis(32)).await;
            loop {
                match events.try_recv() {
                    Ok(next) => event = UiEvent::from(next),
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                        event = UiEvent::ResyncRequired;
                    }
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                        let _ = app.emit("workflow:event", event);
                        break 'events;
                    }
                }
            }
            let _ = app.emit("workflow:event", event);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_decoration::init())
        .setup(|app| {
            let service = Mutex::new(ProviderService::new());
            let runtime = Mutex::new(None);
            app.manage(AppState { runtime, service });

            #[cfg(desktop)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.create_overlay_titlebar();
                    window.show().unwrap();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_providers,
            fetch_providers,
            configure_runtime,
            snapshot,
            send,
            create_agent,
            remove_agent,
            get_roles,
            add_role,
            save_config,
            load_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
