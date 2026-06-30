//! Background event loop for the agent runtime state machine.
//!
//! Pipeline: `ActivateAgent → execute_agent_inner → ChildCompleted
//! → all_done? → ReadyForAggregation → spawn synthesis → AggregationCompleted`
//!
//! # Channel topology
//!
//! ```text
//! Tool (decompose_task) ──► event_tx (from AppState)
//!                           │
//!                           ▼
//!                    RuntimeEventLoop::run()
//!                     ├─ ActivateAgent  → handle (execute child)
//!                     ├─ ChildCompleted → forward to broker_tx
//!                     ├─ AgentFailed    → forward to broker_tx
//!                     └─ (others)       → forward to broker_tx
//!                           │
//!                           ▼
//!                    runtime_bridge (→ AppEvent → TUI)
//! ```

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};

use wf_agent::{AgentPool, AgentStatus};
use wf_core::AgentId;
use crate::runtime::AgentRuntime;
use crate::runtime::event::RuntimeEvent;
use crate::runtime::graph_analytics::TemplateEvolution;
use crate::runtime::orchestration::{
    CapabilityRegistry, DefaultEscalationPolicy, DefaultRoleSelector, EmbeddingGoalAnalyzer,
    NoopDecompositionEngine, PipelineDispatchDecider, ReferenceEmbeddings, TaskOutcomeStore,
};
use crate::runtime::scheduler::TaskScheduler;
use crate::runtime::strategy_graph::{CompetitionProtocol, StrategyGraph};
use wf_tools::ToolServerHandle;

/// Background agent lifecycle loop.
pub struct RuntimeEventLoop {
    runtime: Arc<RwLock<AgentRuntime>>,
    pool: Arc<RwLock<AgentPool>>,
    event_rx: mpsc::Receiver<RuntimeEvent>,
    /// Events that the loop does not consume are forwarded here
    /// to the TUI broker.
    broker_tx: mpsc::Sender<RuntimeEvent>,
    tool_server: ToolServerHandle,
    /// Channel sender used to push events to the runtime event loop.
    /// Passed directly (not via AppState) to avoid wf-tui dependency.
    runtime_event_tx: Option<mpsc::Sender<RuntimeEvent>>,
    /// Phase 2B: extracted scheduler — graph query + pipeline + agent spawn.
    scheduler: TaskScheduler,
    /// Checkpoint manager for durable agent/task state.
    checkpoint: crate::checkpoint::Checkpoint,
    /// Event counter for periodic checkpoint saves.
    checkpoint_tick: u64,
}

impl RuntimeEventLoop {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        event_rx: mpsc::Receiver<RuntimeEvent>,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        tool_server: ToolServerHandle,
        runtime_event_tx: Option<mpsc::Sender<RuntimeEvent>>,
    ) -> Self {
        let decider = Box::new(PipelineDispatchDecider::new(runtime.clone()));

        // Extract embedding service and build decomposition/routing components.
        let goal_analyzer: Arc<dyn crate::runtime::orchestration::GoalAnalyzer> = {
            let rt = runtime.read().await;
            let svc = rt.embedding_service();
            let refs = ReferenceEmbeddings::compute(&*svc).await;
            Arc::new(EmbeddingGoalAnalyzer::new(refs))
        };

        let scheduler =
            TaskScheduler::new(runtime.clone(), pool.clone(), broker_tx.clone(), decider)
                .with_strategy_graph(Arc::new(std::sync::Mutex::new(StrategyGraph::new(
                    CompetitionProtocol::default(),
                ))))
                .with_escalation(
                    Box::new(DefaultEscalationPolicy::default()),
                    Arc::new(RwLock::new(TaskOutcomeStore::new())),
                )
                .with_decomposition(Box::new(NoopDecompositionEngine))
                .with_routing(
                    Box::new(DefaultRoleSelector::new(goal_analyzer)),
                    Arc::new(RwLock::new(CapabilityRegistry::new())),
                )
                .with_graph_analytics(Arc::new(
                    std::sync::Mutex::new(TemplateEvolution::default()),
                ));

        let checkpoint = crate::checkpoint::Checkpoint::new();
        Self {
            runtime,
            pool,
            event_rx,
            broker_tx,
            tool_server,
            runtime_event_tx,
            scheduler,
            checkpoint,
            checkpoint_tick: 0,
        }
    }

    pub async fn run(mut self) {
        // Periodic eviction interval: evict stale agents every 120 events.
        let mut eviction_tick = 0u64;
        const EVICTION_INTERVAL: u64 = 120;

        while let Some(event) = self.event_rx.recv().await {
            eviction_tick += 1;
            if eviction_tick % EVICTION_INTERVAL == 0 {
                let mut pool_guard = self.pool.write().await;
                let stale = pool_guard.evict_stale(None);
                let lru = pool_guard.evict_lru(None);
                let total = stale + lru;
                if total > 0 {
                    tracing::info!(
                        "Event loop evicted {} agent(s) (stale: {}, lru: {})",
                        total,
                        stale,
                        lru
                    );
                }
            }
            // Periodic checkpoint: save pool + graph at configured interval.
            // Two-phase: serialize under brief lock, write outside lock.
            if eviction_tick % 10 == 0 {
                let cp_interval = {
                    let p = self.pool.read().await;
                    p.checkpoint_interval
                };
                if cp_interval > 0 && self.checkpoint_tick % cp_interval == 0 {
                    // Phase 1: serialize under lock (fast, in-memory).
                    let serialized = {
                        let p = self.pool.read().await;
                        let rt = self.runtime.read().await;
                        let g = rt.task_graph.lock().expect("runtime_loop mutex poisoned");
                        self.checkpoint.serialize_snapshot(&p, &g)
                    };
                    // Phase 2: write to disk outside lock (slow I/O).
                    match serialized {
                        Ok((pool_bytes, graph_bytes)) => {
                            if let Err(e) =
                                self.checkpoint.write_snapshot(&pool_bytes, &graph_bytes)
                            {
                                tracing::warn!("Checkpoint write failed: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Checkpoint serialize failed: {}", e);
                        }
                    }
                }
                self.checkpoint_tick += 1;
            }

            // Yield to the async runtime so other tasks
            // (spawned agents, lock holders) can make progress.
            tokio::task::yield_now().await;

            match event {
                RuntimeEvent::InboxMessage { .. } => {
                    // Clone before destructuring so we can forward to broker.
                    let forward = event.clone();
                    if let RuntimeEvent::InboxMessage {
                        agent_id,
                        from_name: _,
                        preview: _,
                        unread_count: _,
                    } = &event
                    {
                        // Re-activate idle/completed agents so they process
                        // incoming messages promptly (notification mode).
                        let needs_reactivation = {
                            let p = self.pool.read().await;
                            p.get_agent(agent_id).is_some_and(|a| {
                                matches!(
                                    a.status,
                                    wf_agent::AgentStatus::Completed
                                        | wf_agent::AgentStatus::Failed
                                        | wf_agent::AgentStatus::Idle
                                )
                            })
                        };
                        if needs_reactivation {
                            if let Ok(mut p) = self.pool.try_write() {
                                if let Some(agent) = p.get_agent_mut(agent_id) {
                                    agent.status = wf_agent::AgentStatus::Planning;
                                    agent.last_active_at = wf_agent::now_secs();
                                }
                            }
                            let rt = self.runtime.clone();
                            let pool = self.pool.clone();
                            let ts = self.tool_server.clone();
                            let bt = self.broker_tx.clone();
                            let retx = self.runtime_event_tx.clone();
                            let bt_mon = bt.clone();
                            let agent_id_mon = *agent_id;
                            let work_handle = tokio::spawn(async move {
                                Self::handle_activate_inner(
                                    rt,
                                    pool,
                                    ts,
                                    bt,
                                    retx,
                                    agent_id_mon,
                                    None,
                                )
                                .await;
                            });
                            tokio::spawn(async move {
                                if let Err(join_err) = work_handle.await {
                                    if join_err.is_panic() {
                                        let panic_payload = join_err.into_panic();
                                        let msg = panic_payload
                                            .downcast_ref::<&str>()
                                            .map(|s| s.to_string())
                                            .or_else(|| {
                                                panic_payload.downcast_ref::<String>().cloned()
                                            })
                                            .unwrap_or_else(|| {
                                                format!("Agent task panicked: {:?}", panic_payload)
                                            });
                                        tracing::error!(
                                            agent_id = ?agent_id_mon,
                                            "Agent task panicked: {}",
                                            msg
                                        );
                                        let _ = bt_mon
                                            .send(RuntimeEvent::AgentFailed {
                                                agent_id: agent_id_mon,
                                                error: format!("Agent panicked: {}", msg),
                                            })
                                            .await;
                                    }
                                }
                            });
                        } else {
                            // Agent is already running — bump last_active_at
                            // so the inbox hint is visible on next LLM call.
                            if let Ok(mut p) = self.pool.try_write() {
                                if let Some(agent) = p.get_agent_mut(agent_id) {
                                    agent.last_active_at = wf_agent::now_secs();
                                }
                            }
                        }
                    }
                    // Always forward to broker so the TUI shows notification.
                    let _ = self.broker_tx.send(forward).await;
                }

                RuntimeEvent::ActivateAgent {
                    agent_id,
                    parent_id,
                } => {
                    // Spawn in background — the event loop must NOT block
                    // on LLM calls; siblings must execute concurrently.
                    let rt = self.runtime.clone();
                    let pool = self.pool.clone();
                    let ts = self.tool_server.clone();
                    let bt = self.broker_tx.clone();
                    let retx = self.runtime_event_tx.clone();
                    // Clone bt for the monitoring task before moving the
                    // original into the work spawn.
                    let bt_mon = bt.clone();
                    let agent_id_mon = agent_id;
                    // Spawn the work and monitor its JoinHandle for panics.
                    // When a tokio::spawn task panics, the runtime catches it
                    // and the JoinHandle yields a JoinError with is_panic()=true.
                    let work_handle = tokio::spawn(async move {
                        Self::handle_activate_inner(rt, pool, ts, bt, retx, agent_id, parent_id)
                            .await;
                    });
                    tokio::spawn(async move {
                        if let Err(join_err) = work_handle.await {
                            if join_err.is_panic() {
                                let panic_payload = join_err.into_panic();
                                let msg = panic_payload
                                    .downcast_ref::<&str>()
                                    .map(|s| s.to_string())
                                    .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                                    .unwrap_or_else(|| {
                                        format!("Agent task panicked: {:?}", panic_payload)
                                    });
                                tracing::error!(
                                    agent_id = ?agent_id_mon,
                                    "Agent task panicked: {}",
                                    msg
                                );
                                let _ = bt_mon
                                    .send(RuntimeEvent::AgentFailed {
                                        agent_id: agent_id_mon,
                                        error: format!("Agent panicked: {}", msg),
                                    })
                                    .await;
                            }
                        }
                    });
                }
                RuntimeEvent::TaskCompleted { task_id, result } => {
                    self.handle_task_completed(task_id, &result).await;
                }

                RuntimeEvent::TaskFailed { task_id, error } => {
                    self.handle_task_failed(task_id, &error).await;
                }

                RuntimeEvent::DecomposeTask {
                    parent_agent,
                    subtasks,
                } => {
                    self.handle_decompose_task(parent_agent, subtasks).await;
                }

                other => {
                    // Everything else → forward to broker.
                    let _ = self.broker_tx.send(other).await;
                }
            }
        }
    }

    // ── Phase 2A: Delegation handlers ──

    /// Handle `TaskCompleted` — mark a task done in the graph, then schedule.
    async fn handle_task_completed(&self, task_id: wf_core::TaskId, result: &str) {
        {
            let rt = self.runtime.read().await;
            let mut g = rt.task_graph.lock().expect("runtime_loop mutex poisoned");
            if let Err(e) = g.mark_complete(task_id) {
                tracing::warn!(
                    "TaskCompleted: mark_complete({:02x}..) failed: {}",
                    task_id[0],
                    e
                );
            } else {
                // Store the result on the node.
                if let Some(node) = g.get_mut(&task_id) {
                    node.result = Some(result.to_string());
                }
                tracing::info!("TaskCompleted: task {:02x}.. done", task_id[0]);
            }
        }
        self.scheduler.dispatch().await;
    }

    /// Handle `TaskFailed` — mark a task failed in the graph, then schedule.
    async fn handle_task_failed(&self, task_id: wf_core::TaskId, error: &str) {
        {
            let rt = self.runtime.read().await;
            let mut g = rt.task_graph.lock().expect("runtime_loop mutex poisoned");
            if let Err(e) = g.mark_failed(task_id, error) {
                tracing::warn!(
                    "TaskFailed: mark_failed({:02x}..) failed: {}",
                    task_id[0],
                    e
                );
            } else {
                tracing::info!("TaskFailed: task {:02x}.. failed", task_id[0]);
            }
        }
        self.scheduler.dispatch().await;
    }

    /// Handle `DecomposeTask` — create child task nodes in the DAG from
    /// LLM-defined subtask list. Replaces the heuristic DecompositionEngine.
    async fn handle_decompose_task(
        &self,
        parent_agent: AgentId,
        subtasks: Vec<wf_core::SubtaskDef>,
    ) {
        // Step 1: Extract parent's task_id and clone task_graph Arc.
        let effective_parent_id = {
            let p = self.pool.read().await;
            match p.get_agent(&parent_agent).and_then(|a| a.task_id) {
                Some(id) => id,
                None => {
                    tracing::error!(
                        "handle_decompose_task: parent {:02x}.. has no task_id",
                        parent_agent[0]
                    );
                    return;
                }
            }
        };

        let task_graph = {
            let rt = self.runtime.read().await;
            rt.task_graph.clone()
        };

        // Step 2: First pass — spawn all child task nodes.
        let mut id_map: std::collections::HashMap<String, wf_core::TaskId> =
            std::collections::HashMap::new();

        {
            let mut g = task_graph.lock().expect("runtime_loop mutex poisoned");
            for sub in &subtasks {
                let child_id = match g.spawn_child(effective_parent_id, &sub.goal) {
                    Some(cid) => cid,
                    None => {
                        tracing::error!(
                            "handle_decompose_task: spawn_child failed under {:02x}..",
                            effective_parent_id[0]
                        );
                        return;
                    }
                };
                if let Some(node) = g.get_mut(&child_id) {
                    node.role = Some(sub.role.clone());
                    if sub.auto_confirm {
                        node.metadata.insert("auto_confirm".into(), "true".into());
                    }
                }
                id_map.insert(sub.id.clone(), child_id);
            }

            // Step 3: Second pass — add dependency edges.
            for sub in &subtasks {
                let Some(&child_id) = id_map.get(&sub.id) else {
                    continue;
                };
                for dep_id_str in &sub.depend_on {
                    let Some(&dep_task_id) = id_map.get(dep_id_str) else {
                        tracing::warn!(
                            "handle_decompose_task: subtask '{}' depends on unknown '{}' — skipping",
                            sub.id,
                            dep_id_str
                        );
                        continue;
                    };
                    if let Err(e) = g.add_dependency(child_id, dep_task_id) {
                        tracing::warn!("handle_decompose_task: add_dependency failed: {}", e);
                    }
                }
            }

            // Step 4: Mark parent as Decomposed.
            if let Some(parent) = g.get(&effective_parent_id) {
                if parent.status == wf_core::task_graph::TaskStatus::Created
                    || parent.status == wf_core::task_graph::TaskStatus::Ready
                {
                    g.mark_decomposed(effective_parent_id).ok();
                }
            }

            let child_count = subtasks.len();
            tracing::info!(
                "DecomposeTask: parent {:02x}.. → {} subtask(s)",
                effective_parent_id[0],
                child_count
            );
        }

        self.scheduler.dispatch().await;
    }

    // ── Handlers ──

    /// Query the TaskGraph for ready tasks and activate agents for them.
    ///
    /// For each ready task:
    /// 1. MARK_RUNNING first (anti-double-dispatch — prevents `ready_tasks()`
    ///    from returning this task again while the pipeline runs)
    /// 2. Runs the decision pipeline (L-1/L0/L1/L2)
    /// 3. If approved, creates an agent and sends `ActivateAgent`
    /// 4. If rejected, marks the task as `Rejected` (not `Failed` — the task
    ///    never started execution)
    async fn handle_activate_inner(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        tool_server: ToolServerHandle,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        runtime_event_tx: Option<mpsc::Sender<RuntimeEvent>>,
        agent_id: AgentId,
        parent_id: Option<AgentId>,
    ) {
        // Determine which tool server to use.
        let agent_sandbox = {
            let p = pool.read().await;
            p.get_agent(&agent_id).and_then(|a| a.sandbox.clone())
        };
        // Build the tool handle and, when a sandbox is available, inject
        // the local embedding engine so SearchAsset tool can perform
        // semantic retrieval against large assets (Shell output, etc.).
        let tool_handle = match &agent_sandbox {
            Some(sb) => {
                // Inject the local ONNX embedding engine into the sandbox.
                // The embedder is obtained directly from the AgentRuntime
                // (which is already available) rather than from AppState.
                {
                    if let Ok(runtime_guard) = runtime.try_read() {
                        let embedder = runtime_guard.embedding_service();
                        sb.attach_embedder(embedder);
                    }
                }
                wf_tools::create_sandboxed_agent_tool_server(
                    pool.clone(),
                    runtime_event_tx.clone(),
                    Some(agent_id),
                    Some(sb.clone()),
                )
            }
            None => tool_server.clone(),
        };

        // Execute the agent (LLM call + tools).
        let (result, status) = AgentRuntime::execute_agent_detached(
            runtime.clone(),
            agent_id,
            pool.clone(),
            Some(tool_handle),
        )
        .await;

        // ── Phase 0: Auto-retry on transient failure ──
        // If the agent failed but has retries left, re-activate it through the
        // event loop.  This ensures the request goes through the full pipeline
        // (L0/L1/L2) again and gets a fresh budget allocation.
        if status == AgentStatus::Failed {
            let should_retry = {
                let p = pool.read().await;
                let max_retries = p.max_retries;
                p.get_agent(&agent_id)
                    .map(|a| a.retry_count < max_retries)
                    .unwrap_or(false)
            };
            if should_retry {
                // Increment retry count, reset status, and re-activate.
                let (task_id, _) = {
                    // Read max_retries before the mutable borrow.
                    let max_retries = pool.read().await.max_retries;
                    let mut p = pool.write().await;
                    if let Some(agent) = p.get_agent_mut(&agent_id) {
                        agent.retry_count += 1;
                        // Reset to Idle so the scheduler picks it up.
                        agent.status = AgentStatus::Idle;
                        // Clear stale result from previous failed attempt.
                        agent.result = None;
                        agent.last_active_at = wf_agent::now_secs();
                        let rc = agent.retry_count;
                        tracing::info!(
                            "Agent {:02x}.. retry {}/{} — re-activating through pipeline",
                            agent_id[0],
                            rc,
                            max_retries,
                        );
                        (agent.task_id, max_retries)
                    } else {
                        (None, max_retries)
                    }
                };
                // Also reset the task in the task graph from Running back to
                // Created so the scheduler can re-dispatch it.
                if let Some(tid) = task_id {
                    let rt = runtime.read().await;
                    if let Ok(mut g) = rt.task_graph.lock() {
                        let prev = g.get(&tid).map(|n| n.status);
                        match prev {
                            Some(wf_core::task_graph::TaskStatus::Running) => {
                                let _ = g.mark_created(tid);
                                tracing::info!(
                                    "Retry: task {:02x}.. reset from Running to Created",
                                    tid[0],
                                );
                            }
                            Some(wf_core::task_graph::TaskStatus::Dispatching) => {
                                let _ = g.mark_created(tid);
                            }
                            _ => {}
                        }
                    }
                }
                // Emit ActivateAgent so the scheduler re-processes this agent.
                // This goes through the pipeline (L0/L1/L2) and gets a fresh budget.
                if let Some(ref tx) = runtime_event_tx {
                    tx.send(RuntimeEvent::ActivateAgent {
                        agent_id,
                        parent_id,
                    })
                    .await
                    .ok();
                }
                // Do NOT emit AgentFailed/TaskFailed — the retry will handle it.
                return;
            }
        }

        // Report completion with structured handoff.
        match status {
            AgentStatus::Completed => {
                if let Some(pid) = parent_id {
                    // ── Phase 1: Extract child agent metadata (brief lock) ──
                    let (child_sandbox, child_name) = {
                        let p = pool.read().await;
                        let child = p.get_agent(&agent_id);
                        (
                            child.and_then(|a| a.sandbox.clone()),
                            child.map(|a| a.name.clone()).unwrap_or_default(),
                        )
                    };

                    // ── Phase 2: Build structured handoff (no lock held) ──
                    // If the result exceeds 1 KB, create a semantic asset in
                    // the child's sandbox so the parent can SearchAsset it.
                    let (summary, payload) = if result.len() > 1024 {
                        if let Some(ref sb) = child_sandbox {
                            match sb.create_embedded_asset("agent_result", &result).await {
                                Ok(handle) => {
                                    // Parse asset_id from the handle string
                                    let asset_id = handle
                                        .find("ID: ")
                                        .and_then(|i| {
                                            let rest = &handle[i + 4..];
                                            rest.find(']').map(|j| rest[..j].trim().to_string())
                                        })
                                        .unwrap_or_default();
                                    // Compact summary (first 200 chars + size)
                                    let preview: String = result.chars().take(200).collect();
                                    let summary = format!(
                                        "[≈{}KB, asset: {}]\n{}",
                                        result.len() / 1024,
                                        asset_id,
                                        preview,
                                    );
                                    let payload =
                                        Some(wf_agent::MessagePayload::AssetPointer {
                                            asset_id,
                                            tool_name: "agent_result".into(),
                                            hint: format!("Agent produced {} bytes", result.len()),
                                        });
                                    (summary, payload)
                                }
                                Err(_) => {
                                    // Fallback: truncated raw text
                                    let preview: String = result.chars().take(512).collect();
                                    (preview, None)
                                }
                            }
                        } else {
                            let preview: String = result.chars().take(512).collect();
                            (preview, None)
                        }
                    } else {
                        // Small result: pass through directly
                        (result.clone(), None)
                    };

                    // ── Phase 3: Deliver to parent inbox (brief lock) ──
                    {
                        let mut p = pool.write().await;

                        // Copy asset indices from child sandbox → parent sandbox
                        // so the parent's SearchAsset tool can find child assets.
                        if payload.is_some() {
                            if let (Some(child_sb), Some(parent)) =
                                (&child_sandbox, p.get_agent_mut(&pid))
                            {
                                if let Some(ref parent_sb) = parent.sandbox {
                                    let child_idx = child_sb
                                        .asset_indices
                                        .read()
                                        .expect("runtime_loop mutex poisoned");
                                    let mut parent_idx = parent_sb
                                        .asset_indices
                                        .write()
                                        .expect("runtime_loop mutex poisoned");
                                    for (k, v) in child_idx.iter() {
                                        parent_idx.entry(k.clone()).or_insert_with(|| v.clone());
                                    }
                                }
                            }
                        }

                        // Send structured message to parent's inbox
                        if let Some(parent) = p.get_agent_mut(&pid) {
                            let msg = wf_agent::AgentMessage {
                                from: agent_id,
                                from_name: child_name.clone(),
                                content: summary.clone(),
                                payload,
                                timestamp: wf_agent::now_secs(),
                            };
                            if parent.inbox.len() >= wf_agent::MAX_INBOX_SIZE {
                                parent.inbox.pop_front();
                            }
                            parent.inbox.push_back(msg);
                        }
                    }

                    // Emit InboxMessage to notify the parent about the
                    // child's result (notification mode for online agents).
                    let inbox_count = {
                        let p = pool.read().await;
                        p.get_agent(&pid).map(|a| a.inbox.len()).unwrap_or(0)
                    };
                    let _ = broker_tx
                        .send(RuntimeEvent::InboxMessage {
                            agent_id: pid,
                            from_name: child_name.clone(),
                            preview: summary.chars().take(200).collect(),
                            unread_count: inbox_count,
                        })
                        .await;

                    let _ = broker_tx
                        .send(RuntimeEvent::ChildCompleted {
                            parent_id: pid,
                            child_id: agent_id,
                            result: summary.clone(),
                        })
                        .await;
                    Self::maybe_advance_parent_inner(
                        runtime.clone(),
                        pool.clone(),
                        broker_tx.clone(),
                        pid,
                    )
                    .await;

                    // ── Phase 4: Emit TaskCompleted to update the task graph ──
                    if let Some(ref tx) = runtime_event_tx {
                        let task_id = {
                            let p = pool.read().await;
                            p.get_agent(&agent_id).and_then(|a| a.task_id)
                        };
                        if let Some(tid) = task_id {
                            tx.send(RuntimeEvent::TaskCompleted {
                                task_id: tid,
                                result: summary.clone(),
                            })
                            .await
                            .ok();
                        }
                    }
                }
            }
            AgentStatus::Failed => {
                let error_msg = if result.is_empty() {
                    "Agent execution failed (no result)".to_string()
                } else {
                    result.clone()
                };
                let _ = broker_tx
                    .send(RuntimeEvent::AgentFailed {
                        agent_id,
                        error: error_msg.clone(),
                    })
                    .await;

                // ── Phase 4: Emit TaskFailed to update the task graph ──
                if let Some(ref tx) = runtime_event_tx {
                    let task_id = {
                        let p = pool.read().await;
                        p.get_agent(&agent_id).and_then(|a| a.task_id)
                    };
                    if let Some(tid) = task_id {
                        tx.send(RuntimeEvent::TaskFailed {
                            task_id: tid,
                            error: error_msg,
                        })
                        .await
                        .ok();
                    }
                }
            }
            _ => {}
        }

        // ── Notification mode: re-activate if unread messages remain ──
        // After completing (or failing), check if the agent still has
        // pending messages. If so, dispatch a new activation so the
        // agent processes them without waiting for external events.
        let has_unread = {
            let p = pool.read().await;
            p.get_agent(&agent_id)
                .map(|a| !a.inbox.is_empty())
                .unwrap_or(false)
        };
        if has_unread {
            // Use runtime_event_tx directly if available;
            // fall back to broker_tx so the event loop picks it up.
            let dispatched = if let Some(ref tx) = runtime_event_tx {
                tx.send(RuntimeEvent::ActivateAgent {
                    agent_id,
                    parent_id,
                })
                .await
                .is_ok()
            } else {
                false
            };
            if !dispatched {
                tracing::warn!(
                    ?agent_id,
                    "Agent has unread messages but no event channel to re-activate"
                );
            }
        }
    }

    async fn maybe_advance_parent_inner(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        parent_id: AgentId,
    ) {
        let all_done = {
            let p = pool.read().await;
            let Some(parent) = p.get_agent(&parent_id) else {
                return;
            };
            parent.children.iter().all(|cid| {
                p.get_agent(cid)
                    .map(|c| matches!(c.status, AgentStatus::Completed | AgentStatus::Failed))
                    .unwrap_or(false)
            })
        };

        if !all_done {
            return;
        }

        // All children done → advance parent and spawn synthesis.
        {
            let mut p = pool.write().await;
            if let Some(parent) = p.get_agent_mut(&parent_id) {
                parent.status = AgentStatus::Aggregating;
            }
        }

        let pool_clone = pool.clone();
        tokio::spawn(async move {
            match runtime
                .read()
                .await
                .synthesize_aggregation(parent_id, &pool_clone)
                .await
            {
                Ok(result) => {
                    {
                        let mut p = pool.write().await;
                        if let Some(parent) = p.get_agent_mut(&parent_id) {
                            parent.result = Some(result.clone());
                            parent.status = AgentStatus::Completed;
                            p.release_budget_guard(&parent_id);
                        }
                    }
                    let _ = broker_tx
                        .send(RuntimeEvent::AggregationCompleted {
                            agent_id: parent_id,
                            result,
                        })
                        .await;
                }
                Err(e) => {
                    // Graceful degradation: concatenate child results.
                    let fallback = {
                        let p = pool.read().await;
                        let agent = match p.get_agent(&parent_id) {
                            Some(a) => a,
                            None => {
                                let _ = broker_tx
                                    .send(RuntimeEvent::AgentFailed {
                                        agent_id: parent_id,
                                        error: format!("Synthesis failed (parent gone): {}", e),
                                    })
                                    .await;
                                return;
                            }
                        };
                        if agent.child_results.is_empty() {
                            format!(
                                "Aggregation synthesis failed: {}\n\nAll sub-tasks completed but no results were captured.",
                                e
                            )
                        } else {
                            let parts: Vec<String> =
                                agent.child_results.iter().map(|(_, r)| r.clone()).collect();
                            format!(
                                "Aggregation synthesis failed ({}).  Raw sub-task results:\n\n---\n{}\n\n---\n*Degraded output*",
                                e,
                                parts.join("\n\n---\n\n")
                            )
                        }
                    };
                    {
                        let mut p = pool.write().await;
                        if let Some(parent) = p.get_agent_mut(&parent_id) {
                            parent.result = Some(fallback.clone());
                            parent.status = AgentStatus::Completed;
                            p.release_budget_guard(&parent_id);
                        }
                    }
                    let _ = broker_tx
                        .send(RuntimeEvent::AggregationCompleted {
                            agent_id: parent_id,
                            result: fallback,
                        })
                        .await;
                }
            }
        });
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use wf_agent::{Agent, AgentConfig};

    fn stub_agent() -> Agent {
        Agent {
            id: [0u8; 16],
            name: String::new(),
            role: String::new(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: String::new(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: 0,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: None,
            sandbox: None,
            retry_count: 0,
            reasoning: String::new(),
        }
    }

    fn pool_with_parent_and_two_children() -> (AgentPool, AgentId, AgentId, AgentId) {
        let mut pool = AgentPool::new();
        let parent = [0u8; 16];
        let child_a = [1u8; 16];
        let child_b = [2u8; 16];

        pool.add_agent(Agent {
            id: parent,
            name: "planner".into(),
            role: "planner".into(),
            status: AgentStatus::AwaitingChildren,
            children: vec![child_a, child_b],
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: child_a,
            name: "dev-a".into(),
            role: "developer".into(),
            parent_id: Some(parent),
            status: AgentStatus::Planning,
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: child_b,
            name: "dev-b".into(),
            role: "developer".into(),
            parent_id: Some(parent),
            status: AgentStatus::Planning,
            ..stub_agent()
        });

        (pool, parent, child_a, child_b)
    }

    #[tokio::test]
    async fn test_maybe_advance_parent_not_done_yet() {
        let (pool, parent, child_a, _) = pool_with_parent_and_two_children();
        let pool = Arc::new(RwLock::new(pool));

        {
            let mut p = pool.write().await;
            if let Some(c) = p.get_agent_mut(&child_a) {
                c.status = AgentStatus::Completed;
            }
        }

        {
            let p = pool.read().await;
            let pe = p.get_agent(&parent).unwrap();
            assert_eq!(pe.status, AgentStatus::AwaitingChildren);
        }
    }

    #[tokio::test]
    async fn test_maybe_advance_parent_all_done() {
        let (pool, parent, child_a, child_b) = pool_with_parent_and_two_children();
        let pool = Arc::new(RwLock::new(pool));

        {
            let mut p = pool.write().await;
            if let Some(c) = p.get_agent_mut(&child_a) {
                c.status = AgentStatus::Completed;
            }
            if let Some(c) = p.get_agent_mut(&child_b) {
                c.status = AgentStatus::Completed;
            }
        }

        let all_done = {
            let p = pool.read().await;
            let pe = p.get_agent(&parent).unwrap();
            pe.children.iter().all(|cid| {
                p.get_agent(cid)
                    .map(|c| matches!(c.status, AgentStatus::Completed | AgentStatus::Failed))
                    .unwrap_or(false)
            })
        };
        assert!(all_done);

        {
            let mut p = pool.write().await;
            if let Some(pe) = p.get_agent_mut(&parent) {
                pe.status = AgentStatus::Aggregating;
            }
        }

        assert_eq!(
            pool.read().await.get_agent(&parent).unwrap().status,
            AgentStatus::Aggregating
        );
    }

    #[tokio::test]
    async fn test_advance_skipped_when_child_failed() {
        let (pool, parent, child_a, child_b) = pool_with_parent_and_two_children();
        let pool = Arc::new(RwLock::new(pool));

        {
            let mut p = pool.write().await;
            if let Some(c) = p.get_agent_mut(&child_a) {
                c.status = AgentStatus::Completed;
            }
            if let Some(c) = p.get_agent_mut(&child_b) {
                c.status = AgentStatus::Failed;
            }
        }

        let all_done = {
            let p = pool.read().await;
            let pe = p.get_agent(&parent).unwrap();
            pe.children.iter().all(|cid| {
                p.get_agent(cid)
                    .map(|c| matches!(c.status, AgentStatus::Completed | AgentStatus::Failed))
                    .unwrap_or(false)
            })
        };
        assert!(all_done, "Failed should count as terminal");
    }
}
