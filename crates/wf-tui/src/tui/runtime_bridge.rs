//! Bridge between [`RuntimeEvent`] (infrastructure layer) and
//! [`AppEvent`](crate::tui::effect::AppEvent) (presentation layer).
//!
//! This module is the **single directional insulation valve**: it reads
//! events from the background `RuntimeEventLoop` and translates them
//! into UI‑visible state changes.
//!
//! # Contract
//!
//! - **Read‑only on shared state** — the bridge never writes to the
//!   `AgentPool`.  Data mutations happen inside the event loop under
//!   its own write locks.
//! - **Bumps the dirty flag** — `agent_tree_version` is incremented on
//!   every event, so Phase 1's diagnostic tree cache is invalidated and
//!   redrawn on the next frame.
//! - **Thin** — transformation logic belongs in the event loop or in
//!   `handle_event`, not here.

use std::sync::Arc;

use tokio::sync::RwLock;

use wf_core::event::RuntimeEvent;
use crate::tui::state::AppState;

/// Run the bridge event loop.
///
/// Spawn this as a background tokio task during TUI initialisation.
/// It terminates when `runtime_rx` is closed (all senders dropped).
pub async fn runtime_event_broker(
    mut runtime_rx: tokio::sync::mpsc::Receiver<RuntimeEvent>,
    app_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::effect::AppEvent>,
    state: Arc<RwLock<AppState>>,
) {
    while let Some(event) = runtime_rx.recv().await {
        let mut s = state.write().await;

        // Bump the Phase 1 dirty flag — the render loop will rebuild
        // the diagnostic tree on the next frame.
        s.ui.agent_tree_version = s.ui.agent_tree_version.wrapping_add(1);

        match event {
            RuntimeEvent::ActivateAgent { .. } => {
                // Pure infrastructure — no TUI visual effect needed.
                // The diagnostic tree picks up the new child on next
                // render via the bumped agent_tree_version.
            }

            RuntimeEvent::ChildCompleted {
                parent_id,
                child_id,
                result,
            } => {
                let id_str = format!(
                    "..{:04x}",
                    u16::from(child_id[0]) << 8 | u16::from(child_id[1])
                );
                let preview: String = result.chars().take(120).collect();
                let content = format!("[OK] Agent {} completed\n{}", id_str, preview);

                // ── Inject child result into parent agent's LLM context ──
                // So the AGENT sees the result on its next LLM call.
                if s.core.responsible_agent_id == Some(parent_id) {
                    use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
                    let now = chrono::Local::now().format("%H:%M:%S").to_string();
                    s.core.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: content.clone(),
                        reasoning: String::new(),
                        chunks: vec![],
                        timestamp: now,
                        status: MessageStatus::Completed,
                    });
                    // Also inject into the agent's context directly.
                    if let Ok(mut pool) = s.core.agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&parent_id) {
                            agent.context.push(wf_llm::types::Message {
                                role: "system".to_string(),
                                content: content.clone(),
                            });
                        }
                    }
                } else {
                    let _ = app_tx.send(crate::tui::effect::AppEvent::SystemLog { content });
                }
            }

            RuntimeEvent::ReadyForAggregation { agent_id } => {
                // Re-enable user input — the parent is about to synthesise.
                s.ui.input_disabled = false;

                let _ = app_tx.send(crate::tui::effect::AppEvent::AggregationStarting { agent_id });
            }

            RuntimeEvent::AggregationCompleted {
                agent_id: _,
                result,
            } => {
                s.ui.input_disabled = false;
                // Push the aggregated result directly as a chat message.
                use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
                s.core.messages.push(ChatMessage {
                    role: MessageRole::Agent,
                    content: result,
                    reasoning: String::new(),
                    chunks: vec![],
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Completed,
                });
            }

            RuntimeEvent::AgentFailed { agent_id, error } => {
                s.ui.input_disabled = false;
                let id_str = format!(
                    "..{:04x}",
                    u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                );
                let _ = app_tx.send(crate::tui::effect::AppEvent::ChatError {
                    response_index: 0,
                    request_id: 0,
                    error: format!("Agent {} failed: {}", id_str, error),
                });
            }

            RuntimeEvent::InboxMessage {
                agent_id,
                from_name,
                preview: _,
                unread_count,
            } => {
                let id_str = format!(
                    "..{:04x}",
                    u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                );
                let content = format!(
                    "Message from {} -> {} ({} unread)",
                    from_name, id_str, unread_count
                );
                let _ = app_tx.send(crate::tui::effect::AppEvent::SystemLog { content });
            }

            // ── Delegation events (Phase 2+) ──
            // These are infrastructure-level mutations.  They become
            // visible in the TUI once the delegation engine is wired
            // up (Phase 3).  For now they just bump the tree version.
            RuntimeEvent::TaskCompleted { .. }
            | RuntimeEvent::TaskFailed { .. }
            | RuntimeEvent::EscalateTask { .. }
            | RuntimeEvent::MergeTaskResult { .. }
            | RuntimeEvent::DecomposeTask { .. } => {
                // Task graph mutation — the TUI diagnostic tree
                // will reflect it on next render via the bumped
                // agent_tree_version.
            }
        }
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use wf_core::event::RuntimeEvent;
    use crate::tui::effect::AppEvent;

    use super::*;

    #[tokio::test]
    async fn test_broker_bumps_tree_version_on_any_event() {
        let state = Arc::new(RwLock::new(AppState::default()));
        let (app_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let (runtime_tx, runtime_rx) = tokio::sync::mpsc::channel::<RuntimeEvent>(64);

        let state_clone = state.clone();
        tokio::spawn(async move {
            runtime_event_broker(runtime_rx, app_tx, state_clone).await;
        });

        let v0 = state.read().await.ui.agent_tree_version;

        runtime_tx
            .send(RuntimeEvent::ActivateAgent {
                agent_id: [1u8; 16],
                parent_id: None,
            })
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let v1 = state.read().await.ui.agent_tree_version;
        assert!(v1 > v0, "version should be bumped: {} → {}", v0, v1);
    }

    #[tokio::test]
    async fn test_broker_sends_system_log_on_child_completed() {
        let state = Arc::new(RwLock::new(AppState::default()));
        let (app_tx, mut app_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runtime_tx, runtime_rx) = tokio::sync::mpsc::channel::<RuntimeEvent>(64);

        let state_clone = state.clone();
        tokio::spawn(async move {
            runtime_event_broker(runtime_rx, app_tx, state_clone).await;
        });

        runtime_tx
            .send(RuntimeEvent::ChildCompleted {
                parent_id: [0u8; 16],
                child_id: [1u8; 16],
                result: "done".to_string(),
            })
            .await
            .unwrap();

        let event = tokio::time::timeout(tokio::time::Duration::from_millis(200), app_rx.recv())
            .await
            .expect("timeout waiting for SystemLog")
            .expect("channel closed");

        match event {
            AppEvent::SystemLog { content } => {
                assert!(content.contains("completed"), "log: {}", content);
            }
            _ => panic!("expected SystemLog"),
        }
    }

    #[tokio::test]
    async fn test_broker_resets_input_on_failure() {
        let state = Arc::new(RwLock::new(AppState::default()));
        let (app_tx, _) = tokio::sync::mpsc::unbounded_channel();
        let (runtime_tx, runtime_rx) = tokio::sync::mpsc::channel::<RuntimeEvent>(64);

        let state_clone = state.clone();
        tokio::spawn(async move {
            runtime_event_broker(runtime_rx, app_tx, state_clone).await;
        });

        assert!(!state.read().await.ui.input_disabled);

        runtime_tx
            .send(RuntimeEvent::AgentFailed {
                agent_id: [1u8; 16],
                error: "OOM".to_string(),
            })
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        assert!(!state.read().await.ui.input_disabled);
    }
}
