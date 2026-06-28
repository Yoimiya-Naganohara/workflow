//! LLM-driven task decomposition tool.
//!
//! Instead of heuristic-based decomposition, the agent calls this tool
//! to let the LLM produce a structured JSON decomposition plan.

use std::collections::HashMap;
use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use tokio::sync::{RwLock, mpsc, oneshot};

use super::builtin::ToolCallError;
use crate::core::types::AgentId;
use crate::llm::{LlmRequest, Message};
use crate::runtime::AgentRuntime;
use crate::runtime::event::RuntimeEvent;

// ── DecomposeTool ──

/// Decompose a goal into structured subtasks via LLM.
///
/// The LLM produces a JSON array of subtasks with roles, goals,
/// dependency relationships, and confirmation flags.
#[derive(Clone)]
pub struct DecomposeTool {
    pub runtime: Arc<RwLock<AgentRuntime>>,
    pub runtime_event_tx: Option<mpsc::Sender<RuntimeEvent>>,
    pub responsible_agent_id: Option<AgentId>,
}

#[derive(Debug, Deserialize)]
pub struct DecomposeArgs {
    pub goal: String,
}

#[derive(Debug, Deserialize)]
struct DecomposeSubtask {
    id: String,
    role: String,
    goal: String,
    #[serde(default)]
    depend_on: Vec<String>,
    #[serde(default = "default_auto_confirm")]
    auto_confirm: bool,
}

fn default_auto_confirm() -> bool {
    true
}

const DECOMPOSE_SYSTEM_PROMPT: &str = "\
You are a task decomposition engine. Given a goal, break it into concrete, \
actionable subtasks that can be executed by specialized agents.

Output a JSON array of subtasks. Each element must have:
- \"id\": a short unique identifier (e.g. \"t1\", \"t2\", \"backend\", \"frontend\")
- \"role\": the agent role (e.g. \"developer\", \"tester\", \"reviewer\", \"planner\")
- \"goal\": a concrete, self-contained description of what to do
- \"depend_on\": array of task IDs that must complete before this one (empty if none)
- \"auto_confirm\": boolean — if true, execute immediately; if false, wait for parent approval

Rules:
1. IDs must be unique across all subtasks.
2. Goals must be specific enough for an agent to execute without further clarification.
3. Dependencies must reference valid IDs within the same array.
4. No circular dependencies.
5. Minimize the number of subtasks — only split when meaningful.
6. Output ONLY the JSON array, no other text, no markdown fences.";

impl Tool for DecomposeTool {
    const NAME: &'static str = "decompose";

    type Error = ToolCallError;
    type Args = DecomposeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Decompose a complex goal into structured subtasks. \
                Each subtask gets a role, concrete goal, dependency relationships, \
                and an auto-confirm flag."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "The goal to decompose into subtasks"
                    }
                },
                "required": ["goal"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let parent_agent = self
            .responsible_agent_id
            .ok_or_else(|| ToolCallError("No active agent to decompose from".to_string()))?;

        let runtime_tx = self
            .runtime_event_tx
            .clone()
            .ok_or_else(|| ToolCallError("Runtime event channel not initialized".to_string()))?;

        // Get LLM provider.
        let provider = {
            let rt = self.runtime.read().await;
            rt.provider
                .clone()
                .ok_or_else(|| ToolCallError("No LLM provider configured".to_string()))?
        };

        // Call LLM to decompose the goal.
        let response = provider
            .complete(LlmRequest {
                model: String::new(), // provider uses the configured model
                messages: vec![
                    Message {
                        role: "system".to_string(),
                        content: DECOMPOSE_SYSTEM_PROMPT.to_string(),
                    },
                    Message {
                        role: "user".to_string(),
                        content: args.goal.clone(),
                    },
                ],
                temperature: 0.2,
                max_tokens: 4096,
                timeout_secs: Some(60),
                max_retries: Some(2),
            })
            .await
            .map_err(|e| ToolCallError(format!("LLM decomposition failed: {}", e)))?;

        // Parse JSON array from response.
        let content = response.content.trim();
        let subtasks: Vec<DecomposeSubtask> = parse_json_array(content)
            .map_err(|e| ToolCallError(format!("Failed to parse decomposition JSON: {}", e)))?;

        if subtasks.is_empty() {
            return Ok("No subtasks produced by decomposition.".to_string());
        }

        // Validate: unique IDs.
        let mut ids_seen = std::collections::HashSet::new();
        for st in &subtasks {
            if !ids_seen.insert(&st.id) {
                return Err(ToolCallError(format!(
                    "Duplicate task id '{}' in decomposition",
                    st.id
                )));
            }
        }

        // Validate: dependency references exist.
        for st in &subtasks {
            for dep in &st.depend_on {
                if !ids_seen.contains(dep) {
                    return Err(ToolCallError(format!(
                        "Task '{}' depends on unknown task '{}'",
                        st.id, dep
                    )));
                }
            }
        }

        // Spawn each subtask via RuntimeEvent::SpawnTask.
        let mut id_map: HashMap<String, crate::core::types::TaskId> = HashMap::new();
        for st in &subtasks {
            let (tx, rx) = oneshot::channel();
            let _ = runtime_tx
                .send(RuntimeEvent::SpawnTaskWithConfirm {
                    goal: st.goal.clone(),
                    role: st.role.clone(),
                    parent_agent,
                    auto_confirm: st.auto_confirm,
                    response_tx: tx,
                })
                .await;
            // Wait for the task_id back from the runtime loop.
            match rx.await {
                Ok(Ok(task_id)) => {
                    id_map.insert(st.id.clone(), task_id);
                }
                Ok(Err(e)) => {
                    return Err(ToolCallError(format!(
                        "Failed to spawn subtask '{}': {}",
                        st.id, e
                    )));
                }
                Err(_) => {
                    return Err(ToolCallError(format!(
                        "Runtime loop dropped response for subtask '{}'",
                        st.id
                    )));
                }
            }
        }

        // Wire dependencies directly in the task graph.
        {
            let rt = self.runtime.read().await;
            let mut g = rt.task_graph.lock().expect("task_graph mutex poisoned");
            for st in &subtasks {
                if let Some(&child_id) = id_map.get(&st.id) {
                    for dep_str in &st.depend_on {
                        if let Some(&dep_id) = id_map.get(dep_str) {
                            if let Err(e) = g.add_dependency(child_id, dep_id) {
                                tracing::warn!(
                                    "decompose: failed to add dependency {} → {}: {}",
                                    st.id,
                                    dep_str,
                                    e
                                );
                            }
                        }
                    }
                    // If auto_confirm=false, transition to PendingConfirm.
                    if !st.auto_confirm {
                        if let Err(e) = g.mark_pending_confirm(child_id) {
                            tracing::warn!(
                                "decompose: failed to mark {} as PendingConfirm: {}",
                                st.id,
                                e
                            );
                        }
                    }
                }
            }
        }

        let summary: Vec<String> = subtasks
            .iter()
            .map(|st| {
                format!(
                    "[{}] {} ({}){}",
                    st.id,
                    st.goal.chars().take(60).collect::<String>(),
                    st.role,
                    if st.auto_confirm {
                        ""
                    } else {
                        " [needs confirm]"
                    }
                )
            })
            .collect();

        Ok(format!(
            "Decomposed into {} subtask(s):\n{}",
            subtasks.len(),
            summary.join("\n")
        ))
    }
}

// ── ConfirmSubtasksTool ──

/// Confirm pending subtasks so they can be dispatched.
///
/// When `auto_confirm=false` subtasks are created in `PendingConfirm` status.
/// This tool transitions them to `Created` so the scheduler picks them up.
#[derive(Clone)]
pub struct ConfirmSubtasksTool {
    pub runtime: Arc<RwLock<AgentRuntime>>,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmSubtasksArgs {
    pub task_ids: Vec<String>,
}

impl Tool for ConfirmSubtasksTool {
    const NAME: &'static str = "confirm_subtasks";

    type Error = ToolCallError;
    type Args = ConfirmSubtasksArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Confirm pending subtasks so they can be dispatched and executed. \
                Use this after reviewing subtasks created with auto_confirm=false."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Hex-encoded task IDs to confirm"
                    }
                },
                "required": ["task_ids"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.task_ids.is_empty() {
            return Err(ToolCallError("No task IDs provided".to_string()));
        }

        let rt = self.runtime.read().await;
        let mut g = rt.task_graph.lock().expect("task_graph mutex poisoned");

        let mut confirmed = Vec::new();
        let mut errors = Vec::new();

        for hex_id in &args.task_ids {
            let task_id = hex_to_task_id(hex_id).map_err(ToolCallError)?;
            match g.mark_created_from_pending(task_id) {
                Ok(()) => confirmed.push(hex_id.clone()),
                Err(e) => errors.push(format!("{}: {}", hex_id, e)),
            }
        }

        let mut output = format!("Confirmed {} task(s).", confirmed.len());
        if !errors.is_empty() {
            output.push_str(&format!(" Errors: {}", errors.join("; ")));
        }
        Ok(output)
    }
}

// ── Helpers ──

/// Parse a JSON array from LLM output, handling markdown fences.
fn parse_json_array<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, String> {
    let content = content.trim();
    // Strip markdown code fences if present.
    let content = if content.starts_with("```") {
        let lines: Vec<&str> = content.lines().collect();
        let start = if lines.first().is_some_and(|l| l.starts_with("```")) {
            1
        } else {
            0
        };
        let end = if lines.last().is_some_and(|l| l.trim() == "```") {
            lines.len() - 1
        } else {
            lines.len()
        };
        lines[start..end].join("\n")
    } else {
        content.to_string()
    };

    serde_json::from_str(&content).map_err(|e| format!("JSON parse error: {}", e))
}

/// Convert a hex string (like "0a1b2c...") to a TaskId ([u8; 16]).
fn hex_to_task_id(hex: &str) -> Result<crate::core::types::TaskId, String> {
    let hex = hex.trim().trim_start_matches("0x");
    if hex.len() < 2 || hex.len() > 32 {
        return Err(format!("Invalid task id length: {}", hex.len()));
    }
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .filter_map(|i| {
            hex.get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect();
    if bytes.len() < 16 {
        let mut id = [0u8; 16];
        for (i, b) in bytes.iter().enumerate() {
            id[i] = *b;
        }
        Ok(id)
    } else {
        let mut id = [0u8; 16];
        id.copy_from_slice(&bytes[..16]);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_array_simple() {
        let json = r#"[
            {"id":"t1","role":"dev","goal":"Build API","depend_on":[],"auto_confirm":true},
            {"id":"t2","role":"tester","goal":"Test API","depend_on":["t1"],"auto_confirm":false}
        ]"#;
        let tasks: Vec<DecomposeSubtask> = parse_json_array(json).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "t1");
        assert!(tasks[0].auto_confirm);
        assert_eq!(tasks[1].depend_on, vec!["t1"]);
        assert!(!tasks[1].auto_confirm);
    }

    #[test]
    fn test_parse_json_array_with_fences() {
        let json = r#"```json
        [{"id":"t1","role":"dev","goal":"Do stuff"}]
        ```"#;
        let tasks: Vec<DecomposeSubtask> = parse_json_array(json).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_parse_json_array_defaults() {
        let json = r#"[{"id":"t1","role":"dev","goal":"Build"}]"#;
        let tasks: Vec<DecomposeSubtask> = parse_json_array(json).unwrap();
        assert!(tasks[0].auto_confirm); // default is true
        assert!(tasks[0].depend_on.is_empty()); // default is empty
    }

    #[test]
    fn test_hex_to_task_id() {
        let id = hex_to_task_id("0a1b").unwrap();
        assert_eq!(id[0], 0x0a);
        assert_eq!(id[1], 0x1b);
        assert_eq!(id[2], 0);
    }

    #[test]
    fn test_hex_to_task_id_full() {
        let id = hex_to_task_id("0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d").unwrap();
        assert_eq!(
            id,
            [
                0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x6a, 0x7b, 0x8c, 0x9d, 0x0e, 0x1f, 0x2a, 0x3b,
                0x4c, 0x5d
            ]
        );
    }
}
