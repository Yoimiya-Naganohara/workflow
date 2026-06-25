//! Agent lifecycle — spawn, execute, synthesize.
//!
//! Methods on [`AgentRuntime`] for creating agents, executing them,
//! and aggregating results.  Extracted from `runtime.rs`.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use super::AgentRuntime;
use super::config::RoleTemplate;

use crate::agent::plan::{PlanEntity, PlanStatus, Task, TaskStatus};
use crate::agent::{Agent, AgentPool, AgentStatus};
use crate::core::types::*;
use crate::llm::LlmProvider;

impl AgentRuntime {
    pub fn bootstrap_root_agent(
        &self,
        goal: &str,
        role: &str,
        agent_pool: &mut AgentPool,
    ) -> AgentId {
        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        // Phase 2B: Create a root task in the graph so Agent ↔ Task mapping
        // is always consistent — no more "agent exists but task doesn't" window.
        let root_task_id: crate::core::types::TaskId = {
            let mut g = self.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            let id = g.spawn_root(goal);
            // Mark as Decomposed so it can receive children.
            g.mark_decomposed(id).ok();
            id
        };

        let agent_id: AgentId = rand::random();
        // Create sandbox (best-effort — failure means no filesystem isolation).
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .ok()
            .map(std::sync::Arc::new);
        let agent = Agent {
            id: agent_id,
            name: format!(
                "{}-{:04x}",
                role,
                u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
            ),
            role: role.to_string(),
            role_template_id: Some(role_tpl.template_id),
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: goal.to_string(),
            config: crate::agent::AgentConfig {
                model_id: self.model_id.clone(),
                ..Default::default()
            },
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: crate::agent::now_secs(),
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: Some(root_task_id),
            sandbox,
            retry_count: 0,
            reasoning: String::new(),
        };
        agent_pool.add_agent(agent);
        agent_id
    }

    pub async fn spawn_root_agent(
        &self,
        goal: &str,
        role: &str,
        value_statement: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let role_emb = self.pipeline.embedding().embed(role).await?;
        let task_emb = self.pipeline.embedding().embed(goal).await?;

        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85))
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();

        // Run the decision pipeline
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: 0,
            responsibility_chain: vec![agent_id],
            raw_text_ref: None,
        };

        let role_tpl_id = Some(role_tpl.template_id);
        let decision = self
            .pipeline
            .process_request(request, role_tpl_id, Some(role_tpl.min_experiences))
            .await?;
        match decision {
            SpawnDecision::Approved(config) => {
                // Attach budget guard to the agent (ownership transferred).
                if let Some(guard) = self.pipeline.take_pending_guard() {
                    agent_pool.attach_budget_guard(agent_id, guard);
                }
                let agent = Agent {
                    id: agent_id,
                    name: format!(
                        "{}-{:04x}",
                        role,
                        u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                    ),
                    role: role.to_string(),
                    role_template_id: role_tpl_id,
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        model_id: self.model_id.clone(),
                        allowed_tools: config.allowed_tools,
                        ..Default::default()
                    },
                    status: AgentStatus::Idle,
                    result: None,
                    child_results: Vec::new(),
                    context: Vec::new(),
                    last_active_at: crate::agent::now_secs(),
                    tokens_input: 0,
                    tokens_output: 0,
                    tool_trace: std::collections::VecDeque::new(),
                    inbox: std::collections::VecDeque::new(),
                    sandbox: sandbox.clone(),
                    task_id: None,
                    retry_count: 0,
                    reasoning: String::new(),
                };
                agent_pool.add_agent(agent);
                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => {
                Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_child(
        &self,
        parent_id: AgentId,
        parent_depth: u32,
        role: &str,
        goal: &str,
        value_statement: &str,
        responsibility_chain: &[AgentId],
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let max_depth = crate::core::constants::DEFAULT_MAX_DEPTH;
        if parent_depth + 1 >= max_depth {
            return Err(anyhow::anyhow!(
                "Agent depth limit ({}) reached — cannot spawn '{}' at depth {}",
                max_depth,
                role,
                parent_depth + 1
            ));
        }
        let role_emb = self.pipeline.embedding().embed(role).await?;

        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85))
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();
        let task_emb = self.pipeline.embedding().embed(goal).await?;
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

        let mut chain = responsibility_chain.to_vec();
        chain.push(agent_id);

        // Derive parent_span_id from the first agent in the responsibility chain.
        let parent_span_id: u64 = responsibility_chain
            .first()
            .and_then(|id| Some(u64::from_le_bytes(id[0..8].try_into().ok()?)))
            .unwrap_or(0);
        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: parent_depth + 1,
            responsibility_chain: chain,
            raw_text_ref: None,
        };

        let role_tpl_id = Some(role_tpl.template_id);
        let decision = self
            .pipeline
            .process_request(request, role_tpl_id, Some(role_tpl.min_experiences))
            .await?;
        match decision {
            SpawnDecision::Approved(config) => {
                // Attach budget guard to the child agent.
                if let Some(guard) = self.pipeline.take_pending_guard() {
                    agent_pool.attach_budget_guard(agent_id, guard);
                }
                let agent = Agent {
                    id: agent_id,
                    name: format!(
                        "{}-{:04x}",
                        role,
                        u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                    ),
                    role: role.to_string(),
                    role_template_id: Some(role_tpl.template_id),
                    parent_id: Some(parent_id),
                    children: Vec::new(),
                    depth: parent_depth + 1,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        model_id: self.model_id.clone(),
                        allowed_tools: config.allowed_tools,
                        ..Default::default()
                    },
                    status: AgentStatus::Idle,
                    result: None,
                    child_results: Vec::new(),
                    context: Vec::new(),
                    last_active_at: crate::agent::now_secs(),
                    tokens_input: 0,
                    tokens_output: 0,
                    tool_trace: std::collections::VecDeque::new(),
                    inbox: std::collections::VecDeque::new(),
                    task_id: None,
                    sandbox: sandbox.clone(),
                    retry_count: 0,
                    reasoning: String::new(),
                };
                agent_pool.add_agent(agent);
                // Register plan entity
                let plan_entity = PlanEntity {
                    plan_name: format!(
                        "{}-{}-{:04x}",
                        role,
                        goal.chars().take(16).collect::<String>(),
                        agent_id[0] as u16
                    ),
                    agent_id,
                    parent_plan: None,
                    goal: goal.to_string(),
                    tasks: vec![Task {
                        id: 0,
                        description: goal.to_string(),
                        status: TaskStatus::Pending,
                        result: None,
                    }],
                    status: PlanStatus::Draft,
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                {
                    let mut reg = self.pipeline.plans().write().await;
                    reg.insert(plan_entity);
                }

                // Link parent → child
                if let Some(parent) = agent_pool.get_agent_mut(&parent_id) {
                    parent.children.push(agent_id);
                }

                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => {
                Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection))
            }
        }
    }

    pub async fn spawn_plan_task_agent(
        &self,
        owner_id: AgentId,
        role: &str,
        goal: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let (parent_depth, responsibility_chain) = agent_pool
            .get_agent(&owner_id)
            .map(|agent| (agent.depth, vec![owner_id]))
            .ok_or_else(|| anyhow::anyhow!("Responsible agent not found"))?;

        self.spawn_child(
            owner_id,
            parent_depth,
            role,
            goal,
            "default",
            &responsibility_chain,
            agent_pool,
        )
        .await
    }

    pub async fn synthesize_plan_result(
        &self,
        owner_id: AgentId,
        plan_goal: &str,
        task_results: &[(usize, String)],
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> String {
        let (config, role, provider) = {
            let mut pool = agent_pool.write().await;
            let (config, role) = match pool.get_agent_mut(&owner_id) {
                Some(agent) => {
                    agent.status = AgentStatus::Aggregating;
                    (agent.config.clone(), agent.role.clone())
                }
                None => return "Responsible agent not found".to_string(),
            };
            (config, role, self.provider.clone())
        };

        let result = if let Some(provider) = provider {
            let role_system_prompt = self
                .role_template_store
                .get_by_role(&role)
                .map(|t| t.system_prompt)
                .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));
            let task_summary = task_results
                .iter()
                .map(|(id, result)| format!("Task {}:\n{}", id, result))
                .collect::<Vec<_>>()
                .join("\n\n");
            let prompt = format!(
                "You own this approved plan.\n\nPlan goal: {}\n\nCompleted task results:\n{}\n\nSynthesize the final result for the user.",
                plan_goal, task_summary
            );
            provider
                .chat(&config.model_id, &role_system_prompt, &prompt)
                .await
                .unwrap_or_else(|e| format!("Plan synthesis failed: {}", e))
        } else {
            "No LLM provider configured".to_string()
        };

        let mut pool = agent_pool.write().await;
        if let Some(agent) = pool.get_agent_mut(&owner_id) {
            agent.result = Some(result.clone());
            agent.status = AgentStatus::Completed;
            pool.release_budget_guard(&owner_id);
            pool.notify_completed(&owner_id);
        }
        result
    }

    /// Aggregate child results into a final synthesis by calling
    /// `provider.chat()` (pure text-in-text-out, no tools, no role
    /// alternation constraints).
    ///
    /// Reads `child_results` from the pool, builds a structured
    /// prompt, and stores the LLM response as the parent's `result`.
    pub async fn synthesize_aggregation(
        &self,
        owner_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> Result<String> {
        // Phase 1: Drain inbox + child_results under a single write lock.
        // Collect raw data; closures must NOT borrow `pool` to avoid conflicts.
        let (config, role, provider, all_summaries, goal): (
            crate::agent::AgentConfig,
            String,
            Option<std::sync::Arc<crate::llm::LlmProvider>>,
            Vec<String>,
            String,
        ) = {
            let mut pool = agent_pool.write().await;
            let agent = pool
                .get_agent_mut(&owner_id)
                .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

            // Drain both inbox (structured handoff) and child_results (legacy).
            let inbox_msgs: Vec<crate::agent::AgentMessage> = agent.inbox.drain(..).collect();
            let cr_raw: Vec<(crate::core::types::AgentId, String)> =
                agent.child_results.drain(..).collect();

            let cfg = agent.config.clone();
            let rl = agent.role.clone();
            let gl = agent.goal.clone();

            // Build inbox summaries inline (no pool access needed).
            let inbox_summaries: Vec<String> = inbox_msgs
                .iter()
                .map(|msg| {
                    let hint = match &msg.payload {
                        Some(crate::agent::MessagePayload::AssetPointer {
                            asset_id, hint, ..
                        }) => format!(" (asset: {}, hint: {})", asset_id, hint),
                        Some(crate::agent::MessagePayload::StateSummary { summary, .. }) => {
                            format!(" (summary: {})", summary)
                        }
                        None => String::new(),
                    };
                    format!("[{}]{}[{}]", msg.from_name, hint, msg.content)
                })
                .collect();

            // Build legacy summaries (no pool access — use raw IDs).
            let cr_summaries: Vec<String> = cr_raw
                .iter()
                .map(|(_, result)| format!("[agent]\n{}", result))
                .collect();

            let all_summaries: Vec<String> = inbox_summaries
                .into_iter()
                .chain(cr_summaries.into_iter())
                .collect();

            (cfg, rl, self.provider.clone(), all_summaries, gl)
        };

        let provider = provider.ok_or_else(|| anyhow::anyhow!("No LLM provider configured"))?;

        let role_system_prompt = self
            .role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));

        let child_count = all_summaries.len();
        let task_summary = all_summaries.join("\n\n---\n\n");

        // Include a note about SearchAsset when there are asset pointers.
        let has_assets = all_summaries.iter().any(|s| s.contains("(asset:"));
        let asset_note = if has_assets {
            concat!(
                "\n\n(NOTE) Some sub-tasks produced large outputs that are stored as assets. ",
                "If you need details, use `search_asset(asset_id, query)`. ",
                "Your current context only contains compact summaries. ",
                "Do not ask for the full raw output unless you truly need it."
            )
        } else {
            ""
        };

        let prompt = format!(
            "You delegated this goal to {} sub-agent(s).\n\nOriginal goal: {}\n\nCompleted sub-task results:\n{}{}\n\nSynthesize the final result for the user.",
            child_count, goal, task_summary, asset_note
        );

        let result = provider
            .chat(&config.model_id, &role_system_prompt, &prompt)
            .await
            .map_err(|e| anyhow::anyhow!("Synthesis LLM call failed: {}", e))?;

        Ok(result)
    }

    /// Map a tool name to a bit position for the tool bitmap.
    pub(crate) fn tool_bit(name: &str) -> u64 {
        match name {
            "read_file" => 1 << 0,
            "write_file" => 1 << 1,
            "sh" => 1 << 2,
            "list_dir" => 1 << 3,
            "grep" => 1 << 4,
            "find_files" => 1 << 5,
            "move_file" => 1 << 6,
            "copy_file" => 1 << 7,
            "delete_file" => 1 << 8,
            "append_file" => 1 << 9,
            "patch_file" => 1 << 10,
            "glob" => 1 << 11,
            "spawn_agent" => 1 << 12,
            "read_memo" => 1 << 13,
            "write_memo" => 1 << 14,
            "delete_memo" => 1 << 15,
            "list_memos" => 1 << 16,
            "call_agent" => 1 << 17,
            "list_agents" => 1 << 18,
            "send_message" => 1 << 19,
            "read_messages" => 1 << 20,
            _ => 0,
        }
    }

    // ── Shared stream processing ──

    /// Process a ToolChatStream: consume events, update tool_trace, detect loops.
    /// Shared by `execute_agent_detached` and `execute_agent_inner`.
    async fn process_tool_stream(
        stream: crate::llm::ToolChatStream,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> (String, u64) {
        use futures::StreamExt;
        futures::pin_mut!(stream);
        let mut text = String::new();
        let mut tools_used: u64 = 0;
        let mut tool_call_count = 0usize;
        let mut done_received = false;

        while let Some(event) = stream.next().await {
            match event {
                crate::llm::ToolEvent::Text(t) => text.push_str(&t),
                crate::llm::ToolEvent::Reasoning(t) => {
                    if let Ok(mut pool) = agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.reasoning.push_str(&t);
                        }
                    }
                }
                crate::llm::ToolEvent::ToolCall { name, args, .. } => {
                    tool_call_count += 1;
                    tools_used |= Self::tool_bit(&name);
                    let args_preview = serde_json::to_string(&args).unwrap_or_default();
                    let args_preview = if args_preview.len() > 80 {
                        format!("{}…", args_preview.chars().take(80).collect::<String>())
                    } else {
                        args_preview
                    };
                    if let Ok(mut pool) = agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.tool_trace.push_back(crate::agent::ToolCallRecord {
                                name,
                                args_preview,
                                status: crate::agent::ToolStatus::Success,
                            });
                            if agent.tool_trace.len() > crate::agent::MAX_TOOL_TRACE {
                                agent.tool_trace.pop_front();
                            }
                        }
                    }
                }
                crate::llm::ToolEvent::TokenUsage { input, output, .. } => {
                    // Accumulate token counts on the agent for UI display.
                    if input > 0 || output > 0 {
                        if let Ok(mut pool) = agent_pool.try_write() {
                            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                                agent.tokens_input = agent.tokens_input.saturating_add(input);
                                agent.tokens_output = agent.tokens_output.saturating_add(output);
                            }
                        }
                    }
                }
                crate::llm::ToolEvent::Done { reason } => {
                    done_received = true;
                    if reason == crate::llm::DoneReason::LoopTerminated
                        || reason == crate::llm::DoneReason::StreamError
                    {
                        tracing::info!(
                            "Agent {:02x}.. completed with {} ({} bytes)",
                            agent_id[0],
                            if reason == crate::llm::DoneReason::LoopTerminated {
                                "loop termination"
                            } else {
                                "stream error"
                            },
                            text.len(),
                        );
                    }
                    break;
                }
            }
        }

        if !done_received {
            tracing::warn!(
                "Agent {:02x}.. stream ended without Done event ({} bytes)",
                agent_id[0],
                text.len(),
            );
        }

        // Empty text fallback
        if text.trim().is_empty() && tool_call_count > 0 {
            text = format!(
                "Completed after {} tool call{}.",
                tool_call_count,
                if tool_call_count == 1 { "" } else { "s" }
            );
        }

        // Heuristic tool error detection
        if !text.is_empty() {
            let error_keywords = [
                "error:",
                "Error:",
                "ERROR:",
                "failed:",
                "Failed:",
                "FAILED:",
                "Not Found",
                "permission denied",
                "Permission denied",
                "timed out",
                "Timed Out",
                "timeout",
                "Tool execution error",
                "connection refused",
                "Connection refused",
                "no such file",
                "No such file",
                "exit code:",
                "Tool execution failed",
                "cannot read",
                "Cannot read",
                "is a directory",
                "Is a directory",
            ];
            let has_error = error_keywords.iter().any(|pat| text.contains(pat));
            if has_error {
                if let Ok(mut pool) = agent_pool.try_write() {
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        for record in agent.tool_trace.iter_mut().rev().take(2) {
                            if record.status == crate::agent::ToolStatus::Success {
                                record.status = crate::agent::ToolStatus::Error;
                                break;
                            }
                        }
                    }
                }
            }
        }

        (text, tools_used)
    }

    // ── Agent execution ──

    /// Execute an agent without holding the runtime read lock across the LLM call.
    ///
    /// This is the deadlock-safe variant for use from spawned tasks where the
    /// caller cannot hold `&self` behind a read guard across `.await` points.
    /// The method internally acquires the lock only for brief data extraction
    /// and experience recording, releasing it before the async LLM call.
    pub(crate) async fn execute_agent_detached(
        runtime: Arc<RwLock<Self>>,
        agent_id: AgentId,
        agent_pool: Arc<RwLock<AgentPool>>,
        tool_server: Option<crate::tools::ToolServerHandle>,
    ) -> (String, AgentStatus) {
        // Phase 1: Extract needed data under a brief read lock
        let (provider, role_template_store, embedding_service) = {
            let rt = runtime.read().await;
            (
                rt.provider.clone(),
                Arc::clone(&rt.role_template_store),
                rt.pipeline.embedding().clone(),
            )
        };

        let (goal, role, config) = {
            let pool = agent_pool.read().await;
            let agent = match pool.get_agent(&agent_id) {
                Some(a) => a.clone(),
                None => return (String::new(), AgentStatus::Failed),
            };
            (agent.goal, agent.role, agent.config.clone())
        };

        let provider: Arc<LlmProvider> = match provider {
            Some(p) => p,
            None => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some("No LLM provider configured".to_string());
                    pool.release_budget_guard(&agent_id);
                    pool.notify_completed(&agent_id);
                }
                return (
                    "No LLM provider configured".to_string(),
                    AgentStatus::Failed,
                );
            }
        };

        // Mark planning
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Planning;
            }
        }

        // Phase 2: Build system prompt (no lock on runtime needed)
        let role_system_prompt = role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));

        let memo_block = {
            let pool = agent_pool.read().await;
            pool.format_role_memos(&role)
        };
        let memos = memo_block.as_deref().unwrap_or("");

        // Read reasoning_effort + reasoning_options from pool (brief read lock).
        let (reasoning_effort, reasoning_options) = {
            let pool = agent_pool.read().await;
            (
                pool.reasoning_effort.clone(),
                pool.reasoning_options.clone(),
            )
        };

        // Check for pending messages and inject notification into prompt.
        let inbox_hint = {
            let pool = agent_pool.read().await;
            pool.get_agent(&agent_id)
                .map(|a| {
                    let count = a.inbox.len();
                    if count > 0 {
                        format!(
                            "\n\nYou have {} unread message(s) in your inbox. \
Use the `read_messages` tool to read them before proceeding. \
Messages may contain important context from sibling agents.",
                            count
                        )
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default()
        };

        let system_prompt = format!(
            "{}\n\n{}\n\n{}{}{}",
            role_system_prompt,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
            memos,
            inbox_hint,
        );
        let leaf_goal = format!(
            "Your goal: {}\n\nWork independently and produce a concrete result. Do not request sub-agents — you are a leaf agent.\n\nTOOL DISCIPLINE: Only call tools when you truly need information you cannot infer. Prefer answering directly from your knowledge. You have up to 6 tool call rounds available, with a maximum of 12 total tool calls per session. No single tool may be called more than 6 times. If you have called several tools and still cannot answer, summarize what you found and explain what is missing. Repeated calls to the same tool with the same arguments (3+ times) will be treated as a loop and terminated.",
            goal
        );

        // Phase 3: Execute LLM call (no lock held on runtime)
        let (response, tool_bitmap) = if let Some(handle) = &tool_server {
            let additional_params = reasoning_effort
                .as_ref()
                .and_then(|effort| provider.reasoning_params(effort, &reasoning_options));
            let stream = match provider
                .chat_with_tools_stream_mcp(
                    &config.model_id,
                    &system_prompt,
                    &leaf_goal,
                    &[],
                    handle,
                    additional_params.as_ref(),
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            let (text, tools_used) = Self::process_tool_stream(stream, agent_id, &agent_pool).await;
            (text, tools_used)
        } else {
            let text = match provider
                .chat(&config.model_id, &system_prompt, &leaf_goal)
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            (text, 0)
        };

        // Phase 4: Record result under brief lock
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Completed;
                agent.result = Some(response.clone());
            }
        }

        // Phase 5: Record experience (re-acquire runtime lock briefly)
        if !response.is_empty() {
            let goal_for_emb = goal.clone();
            if let Ok(emb) = embedding_service.embed(&goal_for_emb).await {
                let rt = runtime.read().await;
                rt.pipeline
                    .add_experience(crate::core::types::ExperienceEntry {
                        embedding: emb,
                        applicability_vector: [0.0f32; 128],
                        tool_bitmap,
                        role_template_id: role_template_store
                            .get_by_role(&role)
                            .map(|t| t.template_id),
                        weight: 1.0,
                        domain_version: 0,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        l2_override_weight: 0.0,
                        l2_override_created_at: 0,
                    });
            }
        }

        // Phase 6: Release budget guard and notify completion
        let mut pool = agent_pool.write().await;
        pool.release_budget_guard(&agent_id);
        pool.notify_completed(&agent_id);

        (response, AgentStatus::Completed)
    }
}
