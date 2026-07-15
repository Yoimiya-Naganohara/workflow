use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use workflow_core::{Runtime, RuntimeSnapshot, WorkflowEvent};

struct AppState {
    runtime: Arc<Runtime>,
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

#[tauri::command]
async fn snapshot(
    state: State<'_, AppState>,
    selected: Option<u32>,
) -> Result<RuntimeSnapshot, String> {
    state
        .runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    Ok(state.runtime.snapshot(selected).await)
}

#[tauri::command]
async fn send(
    state: State<'_, AppState>,
    target: u32,
    text: String,
) -> Result<RuntimeSnapshot, String> {
    state
        .runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    state
        .runtime
        .send_message(target, text)
        .await
        .map_err(|error| error.to_string())?;
    Ok(state.runtime.snapshot(Some(target)).await)
}

#[tauri::command]
async fn create_agent(
    state: State<'_, AppState>,
    role_name: String,
) -> Result<Vec<workflow_core::AgentInfo>, String> {
    state
        .runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    state
        .runtime
        .create_agent(role_name)
        .await
        .map_err(|error| error.to_string())?;
    Ok(state.runtime.list_agents().await)
}

#[tauri::command]
async fn remove_agent(
    state: State<'_, AppState>,
    id: u32,
) -> Result<Vec<workflow_core::AgentInfo>, String> {
    state
        .runtime
        .initialize()
        .await
        .map_err(|error| error.to_string())?;
    state.runtime.remove_agent(id).await;
    Ok(state.runtime.list_agents().await)
}

#[tauri::command]
fn get_roles(state: State<'_, AppState>) -> Vec<workflow_core::RoleInfo> {
    state.runtime.list_roles()
}

#[tauri::command]
fn add_role(
    state: State<'_, AppState>,
    name: String,
    definition: String,
) -> Vec<workflow_core::RoleInfo> {
    state.runtime.add_role(name, definition)
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

            // Streaming models can emit many token events per second. Coalesce
            // them into one UI invalidation per frame instead of invoking a
            // snapshot command for every token.
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
        .setup(|app| {
            let runtime = Arc::new(Runtime::new());
            app.manage(AppState {
                runtime: Arc::clone(&runtime),
            });
            spawn_event_bridge(app.handle().clone(), runtime);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            send,
            create_agent,
            remove_agent,
            get_roles,
            add_role,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
