# Architecture

```
                         ┌──────────────────────────┐
                         │     User / TUI / CLI      │
                         └──────────┬───────────────┘
                                    │ SpawnRequest
                                    ▼
┌───────────────────────────────────────────────────────────────────┐
│                    AGENT RUNTIME (orchestrator)                    │
│  AgentRuntime ── runtime_loop ── event bus ── background eviction │
└────────────────────────────┬──────────────────────────────────────┘
                             │
                             ▼
┌───────────────────────────────────────────────────────────────────┐
│                    DECISION PIPELINE (4 layers)                    │
│                                                                   │
│   ┌─────────┐    ┌──────────┐    ┌─────────┐    ┌──────────┐     │
│   │  L-1    │───▶│   L0     │───▶│   L1    │───▶│   L2     │     │
│   │Admission│    │Circuit   │    │Exp.     │    │Audit     │     │
│   │(tokio   │    │Breaker   │    │Retrieval│    │Rule/LLM  │     │
│   │semaphore│    │(CAS +    │    │+ Value  │    │Judge     │     │
│   │)        │    │RAII)     │    │Classifier│   │          │     │
│   └─────────┘    └──────────┘    └─────────┘    └──────────┘     │
└───────────────────────────────────────────────────────────────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │   SpawnDecision      │
                  │  Approved / Rejected │
                  └─────────┬───────────┘
                            │
                            ▼
┌───────────────────────────────────────────────────────────────────────┐
│                      AGENT LIFECYCLE                                  │
│                                                                       │
│   AgentPool (DashMap) ── PlanRegistry ── SuspendQueue                 │
│        │                                                              │
│        ├── Idle ──▶ Planning ──▶ Executing (LLM + tools)              │
│        │                              │                               │
│        │               ┌──────────────┼───────────────┐               │
│        │               ▼              ▼               ▼               │
│        │         Completed        Failed        Suspended             │
│        │                                                              │
│        └── child_results ──▶ sibling inbox ──▶ synthesis              │
└───────────────────────────────────────────────────────────────────────┘
```

## Table of Contents

1. [System Overview](#system-overview)
2. [Decision Pipeline (L-1 → L0 → L1 → L2)](#decision-pipeline)
3. [Dual-Track Memory](#dual-track-memory)
4. [SIMD Acceleration](#simd-acceleration)
5. [Agent Lifecycle](#agent-lifecycle)
6. [Dependency Injection](#dependency-injection)
7. [Role Template System](#role-template-system)
8. [MCP Tool System](#mcp-tool-system)
9. [TUI Architecture](#tui-architecture)
10. [Safety & Contracts](#safety--contracts)

---

## System Overview

**workflow** is a holographic self-evolving multi-agent system with a layered decision architecture. Every agent spawn request flows through a 4-stage decision pipeline (L-1/L0/L1/L2) before execution. Approved agents run in an event-driven lifecycle backed by dual-track memory, SIMD-accelerated retrieval, lock-free resource accounting, and an MCP-based tool system.

### Key design principles

| Principle | Implementation |
|-----------|---------------|
| **Lock-free resource accounting** | CAS loops with exponential backoff (no coarse locks) |
| **RAII safety** | `BudgetGuard` auto-rollbacks budget/tools/depth on drop |
| **Dependency injection** | Every layer is a trait object, swappable via builder |
| **Deadlock prevention** | Never hold `RwLock` across `.await` points — extract data first, then call async |
| **Fail-deadly** | L2 collapses after 5 consecutive failures (stops approvals) |
| **Two-tier memory** | mmap bedrock (durable) + fluid Vec (fast, bounded, clustered) |

---

## Decision Pipeline

All in `src/runtime/pipeline.rs`. Built via `DecisionPipelineBuilder`:

```rust
pub struct DecisionPipelineBuilder {
    admission: Option<Box<dyn AdmissionControl>>,     // L-1
    circuit_breaker: Option<Box<dyn CircuitBreaker>>,  // L0
    experience: Option<Box<dyn ExperienceRetrieval>>,  // L1
    audit_engine: Option<Box<dyn AuditEngine>>,        // L2
    embedding: Option<Arc<dyn EmbeddingService>>,
    suspend: Option<Box<SuspendQueue>>,
    plans: Option<Box<PlanRegistry>>,
}
```

### L-1: Admission (`src/admission.rs`)

A **tokio semaphore** concurrency gate. Configurable permits (default 10).

| Aspect | Detail |
|--------|--------|
| Mechanism | `tokio::sync::Semaphore` with owned permits |
| Timeout | 100ms (`DEFAULT_ADMISSION_TIMEOUT_MS`) |
| Failure | `SpawnRejection::SystemOverloaded` |
| Trait | `AdmissionControl: Send + Sync { async fn acquire() }` |

### L0: Circuit Breaker (`src/l0.rs`)

Three atomic resource guards, each using a CAS loop with exponential backoff:

```
                               ┌─────────────────────────────┐
                               │      TaskResourceState       │
                               │  (lock-free, atomic fields)  │
                               │                             │
                               │  remaining_budget: AtomicI64│
                               │  tool_bitmap:     AtomicU64 │
                               │  current_depth:   AtomicU32 │
                               └─────────────────────────────┘
                                       │
                ┌──────────────────────┼──────────────────────┐
                ▼                      ▼                      ▼
        ┌──────────────┐      ┌──────────────┐      ┌──────────────┐
        │ Budget Guard │      │ Tool Guard   │      │ Depth Guard  │
        │ CAS acquire  │      │ CAS acquire  │      │ fetch_update │
        │ RAII drop    │      │ RAII drop    │      │ RAII drop    │
        └──────────────┘      └──────────────┘      └──────────────┘
```

| Guard | Type | Strategy |
|-------|------|----------|
| **Budget** | `AtomicI64` | `compare_exchange_weak` — decrement if `remaining >= requested` |
| **Tools** | `AtomicU64` bitmap | `compare_exchange_weak` — OR if no bit collision |
| **Depth** | `AtomicU32` | `fetch_update` — increment if `current < max` |

All three are acquired atomically in L0. If any fails, the already-acquired ones are rolled back.

**Suspend on conflict**: If a tool collision occurs, the request is enqueued with a priority score:
```
priority = BUDGET_PRIORITY_WEIGHT(0.6) * (remaining/requested)
         + DEPTH_PRIORITY_WEIGHT(0.4)  * (1/depth)
```

**RAII**: `BudgetGuard` releases all resources on `Drop` unless `commit()` or `settle()` was called. `L0Permit` auto-rolls back on drop.

### L1: Experience Retrieval & Classification (`src/l1/`)

Two sub-systems:

**`L1Retriever`** — SIMD nearest-neighbor search against dual-track memory, returning top-k `ExperienceEntry` matches with cosine similarity scores.

**Weighted confidence formula** (sums to 1.0):

| Component | Weight | Source |
|-----------|--------|--------|
| Task similarity | 0.35 | `cosine(task_emb, experience_emb)` |
| Role similarity | 0.25 | `cosine(role_emb, experience_emb)` |
| Value alignment | 0.25 | `cosine(value_emb, experience_emb)` |
| Recency decay | 0.15 | `exp(-hours_since / 1h)` |

Threshold: `DEFAULT_L1_CONFIDENCE = 0.5`. Below → `SpawnRejection::L1Rejected`.

### L2: Audit (`src/l2/`)

Two swappable engines:

| Engine | File | Behavior |
|--------|------|----------|
| **`L2RuleAuditEngine`** | `src/l2/mod.rs` | Hardcoded rules. Collapses after `MAX_CONSECUTIVE_FAILURES` (5). |
| **`L2LlmAuditEngine`** | `src/l2/llm.rs` | LLM judge (500 tokens, temp 0.3). Semantic screening of the request. |

The `L2_OVERRIDE_BOOST` multiplier (1.5×) is applied to experience entries where a human previously overrode L2's rejection.

---

## Dual-Track Memory

`src/experience/dual_track.rs` — two-tier experience storage:

```
                    ┌─────────────────────────────────────┐
                    │          Experience Pool             │
                    │     (shared via trait object)        │
                    └──────────┬──────────────────────────┘
                               │
                ┌──────────────┴──────────────┐
                ▼                              ▼
    ┌────────────────────┐         ┌────────────────────┐
    │   A-TRACK (Bedrock)│         │  B-TRACK (Fluid)   │
    │                    │         │                    │
    │  memmap2-backed    │         │  VecDeque in-memory│
    │  ~/.workflow/      │         │  Bounded capacity  │
    │  experience_a.bin  │         │  FIFO eviction     │
    │                    │         │                    │
    │  Durable,          │         │  Volatile,         │
    │  cross-session     │         │  fast writes       │
    └────────────────────┘         └────────────────────┘
               ▲                            │
               │     Consolidation          │
               └──── (clustering) ──────────┘
                          │
                          ▼
              ┌──────────────────────┐
              │  ClusterConsolidator │
              │  (k-means on fluid   │
              │   entries → promote  │
              │   reps to bedrock)   │
              └──────────────────────┘
```

**Read path** — queries both tracks in parallel, merges top-k weighted by per-track credibility factors.

**Write path** — new `ExperienceEntry` objects land in the fluid track first. When capacity is exceeded, consolidation runs: clustering (k-means) over fluid entries creates representative vectors that are promoted to bedrock.

### Experience Entry Schema

```rust
#[repr(C)]
pub struct ExperienceEntry {
    embedding: [f32; EMBEDDING_DIM],        // 384 floats
    applicability_vector: [f32; 128],        // domain applicability
    tool_bitmap: u64,                       // which tools were used
    role_template_id: Option<u32>,          // associated role
    weight: f32,                            // credibility weight
    domain_version: u64,                    // domain version marker
    timestamp: u64,                         // creation time
    l2_override_weight: f32,                // human override boost
    l2_override_created_at: u64,            // when override was set
}
```

`#[repr(C)]` ensures binary layout matches the mmap file.

---

## SIMD Acceleration

`src/core/simd.rs` — AVX2+FMA cosine similarity for 384-dimensional vectors:

```
384 floats = 12 iterations × 32 bytes (256-bit AVX2 register)
  Each iteration:
    vmovups      ymm, [ptr]      // load 8 floats
    vfmadd231ps  sum, ymm, ymm   // fused multiply-add
    vfmadd231ps  sum2, ymm, ymm  // second accumulator (norm^2)

  Horizontal sum:
    vextractf128  xmm1, ymm0, 1
    vaddps        xmm0, xmm0, xmm1
    vhaddps       xmm0, xmm0, xmm0       // 2× horizontal add
    vmovss        -> scalar
```

- Scalar reference implementation validates error < 1e-5
- Falls back to scalar if `avx2` feature is unavailable
- Used by L1 retriever for nearest-neighbor search

---

## Agent Lifecycle

`src/runtime/runtime_loop.rs` — background event loop driving agent state:

```
                          ┌──────────────┐
                          │  Activate    │
                          │  Agent       │  ← RuntimeEvent::ActivateAgent(id)
                          └──────┬───────┘
                                 │
                                 ▼
                      ┌──────────────────────┐
                      │   Planning phase     │
                      │   (status: Planning) │
                      └──────────┬───────────┘
                                 │
                                 ▼
┌──────────────────────────────────────────────────────────────────┐
│                     Execution phase                               │
│  execute_agent_detached():                                        │
│                                                                   │
│  1. Extract config under brief read lock                          │
│  2. Build system_prompt (role + memos + zero-tolerance + inbox)   │
│  3. LLM call: chat_with_tools_stream_mcp (no lock held)           │
│  4. Tool events stream back (text, tool_call, tool_result)        │
│  5. Record result + update status under brief write lock          │
└──────────────────────────────────────────────────────────────────┘
                                 │
                    ┌────────────┴────────────┐
                    ▼                         ▼
             ┌──────────────┐          ┌──────────────┐
             │ Spawn child  │          │  Complete     │
             │ (recursive)  │          │  → notify     │
             │              │          │    parent      │
             │ Pipeline     │          │               │
             │ again (L-1…  │          │  status:       │
             │ L2)          │          │  Completed /   │
             └──────────────┘          │  Failed        │
                                        └──────────────┘
```

### Key deadlock prevention

`execute_agent_detached` is the **only** way agents execute from spawned tasks. It never holds the runtime lock across `.await`:

```rust
// Phase 1: extract data under brief lock
let (provider, role_template_store, _) = {
    let rt = runtime.read().await;   // ← brief
    (rt.provider.clone(), ...)
};  // ← lock released here

// Phase 2: LLM call (no lock held)
let stream = provider.chat_with_tools_stream_mcp(...).await;
```

### Eviction

Every 120 events, the loop evicts:
- **Stale agents** — agents idle beyond a threshold
- **LRU agents** — least recently used when pool is over capacity

---

## Dependency Injection

Every layer is a trait. `DecisionPipelineBuilder` wires them together:

```
           ┌──────────────────────────────────────┐
           │         DecisionPipeline             │
           │                                      │
           │  admission:    Box<dyn AdmissionControl│
           │  circuit_breaker: Box<dyn CircuitBreaker│
           │  experience:   Box<dyn ExperienceRetrieval│
           │  audit_engine: Box<dyn AuditEngine>  │
           │  embedding:    Arc<dyn EmbeddingService>│
           │  suspend:      Box<SuspendQueue>     │
           │  plans:        Arc<RwLock<PlanRegistry>>│
           └──────────────────────────────────────┘
```

### Extension paths

```rust
// Quick-start (default impls tuned by config)
AgentRuntime::new(config, embedding_service)

// Full control (custom audit engine, custom experience retriever)
let pipeline = DecisionPipelineBuilder::new()
    .embedding(my_embedding)
    .audit_engine(Box::new(MyCustomAudit::new()))
    .llm_audit_engine(provider, L2LlmConfig { .. })
    .build();
AgentRuntime::from_pipeline(pipeline);
```

### Trait definitions

| Layer | Trait | Location |
|-------|-------|----------|
| L-1 | `AdmissionControl` | `src/admission.rs` |
| L0 | `CircuitBreaker` | `src/l0.rs` |
| L1 | `ExperienceRetrieval` | `src/l1/mod.rs` |
| L2 | `AuditEngine` | `src/l2/mod.rs` |
| Embedding | `EmbeddingService` | `src/llm/embed.rs` |

---

## Role Template System

7 built-in roles seeded on first run via `RoleTemplateStore::seed_if_empty()`:

```rust
pub struct RoleTemplate {
    pub role: String,              // e.g. "planner"
    pub label: String,             // e.g. "Project Planner"
    pub system_prompt: String,     // full system prompt text
    pub template_id: u32,          // 0-7 (built-in)
    pub embedding: Option<[f32; EMBEDDING_DIM]>,
    pub min_experiences: usize,    // min matching exp. for confidence (default 0)
}
```

| ID | Role | Purpose | Min Exp. |
|----|------|---------|----------|
| 0 | `general_business_analyst` | Requirements analysis (INVEST framework) | 0 |
| 1 | `tester` | QA engineering | 0 |
| 2 | `developer` | Feature implementation | 0 |
| 3 | `reviewer` | Code review | 0 |
| 4 | `planner` | Strategic decomposition into plans | 3 |
| 5 | `security_auditor` | Threat modeling + remediation | 3 |
| 6 | `researcher` | Technical research and synthesis | 3 |
| 7 | `devops` | Infrastructure, CI/CD, cloud | 0 |

**Matching**: exact role name lookup first. Falls back to nearest-embedding search with 0.85 cosine threshold.

**Embeddings** computed asynchronously on startup via `compute_role_embeddings_async()`.

**Persistence**: `~/.workflow/role_templates.json` (JSON file).

---

## MCP Tool System

`src/tools/` — built on **rmcp** (rig's Model Context Protocol implementation):

```
                      ┌────────────────────────┐
                      │   ToolServer (MCP)     │
                      │   rmcp::Router         │
                      └──────────┬─────────────┘
                                 │ registers
              ┌──────────────────┼──────────────────┐
              ▼                  ▼                  ▼
     ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
     │ Built-in     │   │ Agent        │   │ Sandbox      │
     │ Tools        │   │ Management  │   │ Filesystem   │
     │              │   │              │   │              │
     │ read_file    │   │ spawn_agent  │   │ per-agent    │
     │ write_file   │   │ read_memo    │   │ ~/.workflow/ │
     │ sh           │   │ write_memo   │   │ sandbox/<id>/│
     │ edit         │   │ call_agent   │   │              │
     │ grep         │   │ send_message │   │ cleanup on   │
     │ glob         │   │ list_agents  │   │ shutdown     │
     └──────────────┘   └──────────────┘   └──────────────┘
```

**Tool bitmap**: each tool maps to a bit position (0-20). L0 enforces exclusivity via CAS on the bitmap. The full bitmap (`!0u64`) grants all tools.

| Bit | Tool | Bit | Tool |
|-----|------|-----|------|
| 0 | read_file | 11 | glob |
| 1 | write_file | 12 | spawn_agent |
| 2 | sh | 13 | read_memo |
| 3 | list_dir | 14 | write_memo |
| 4 | grep | 15 | delete_memo |
| 5 | find_files | 16 | list_memos |
| 6 | move_file | 17 | call_agent |
| 7 | copy_file | 18 | list_agents |
| 8 | delete_file | 19 | send_message |
| 9 | append_file | 20 | read_messages |
| 10 | patch_file | | |

---

## TUI Architecture

`src/tui/` — ratatui terminal UI with 17 modules:

### State model

```rust
pub struct AppState {
    pub core: CoreState,
    pub ui: UiState,
    pub popup_mode: PopupMode,
    // channels to/from runtime
    pub event_tx: mpsc::Sender<RuntimeEvent>,
    pub event_rx: ...,
}
```

### Component hierarchy

```
┌─────────────────────────────────────────────────┐
│                   Status Bar                     │
│  Budget | Permits | Agents | Mode | Model       │
├──────────────────────────┬──────────────────────┤
│                          │                      │
│   Agent Tree             │   Chat Panel         │
│   (hierarchical)         │   (conversation)     │
│                          │                      │
│   ┌─ planner-0a1b       │   > User message     │
│   │  ├─ developer-2c3d  │   < Agent response   │
│   │  └─ tester-4e5f     │   > User message     │
│   └─ reviewer-6g7h      │                      │
│                          │   Input: [________]  │
├──────────────────────────┴──────────────────────┤
│               Command Tree (toggle)              │
└─────────────────────────────────────────────────┘
```

### Event flow

```
User Input (crossterm event stream)
    │
    ▼
handler.rs ──────► keymap.rs ──────► controller.rs
                                         │
                                    ┌────┴────┐
                                    ▼         ▼
                              effect.rs   runtime_bridge.rs
                              (local UI)     │
                                              ▼
                                         event_tx
                                              │
                                              ▼
                                      RuntimeEventLoop
                                      (runtime_loop.rs)
                                              │
                                              ▼
                                         broker_tx
                                              │
                                              ▼
                                      AppEvent → TUI render
```

### Key TUI files

| File | Responsibility |
|------|---------------|
| `state.rs` | Data model, popup modes, focus tracking |
| `render.rs` | Layout composition (panels, split ratios) |
| `handler.rs` | Event dispatch |
| `controller.rs` | Command-to-effect mapping |
| `effect.rs` | Side-effect execution (model selection, agent spawn) |
| `runtime_bridge.rs` | `RuntimeEvent` ↔ `AppEvent` translation |
| `keymap.rs` | Key binding definitions |
| `chat.rs` | Chat panel rendering and message history |
| `agent_tree.rs` | Hierarchical agent tree with status colors |
| `popup.rs` | Dialog rendering (confirm, input, list) |
| `status.rs` | Bottom status bar |
| `style.rs` | Color theme and styling |
| `command_tree.rs` | Available command overlay |

---

## Safety & Contracts

### Resource safety

| Mechanism | Scope | Protection |
|-----------|-------|------------|
| `BudgetGuard::Drop` | Budget + tools + depth | Auto-rollback on panic or crash |
| `L0Permit::Drop` | Budget + tools + depth | Auto-rollback on permit leak |
| `AdmissionPermit::Drop` | Semaphore permit | Returns permit to semaphore |
| `catch_unwind` in BudgetGuard::Drop | BudgetGuard | Prevents double-panic abort |

### Deadlock prevention

```rust
// ❌ BAD: holding lock across .await
let rt = runtime.read().await;
let result = rt.some_async_fn().await;  // ← lock held!
drop(rt);

// ✅ GOOD: extract then call
let (provider, model) = {
    let rt = runtime.read().await;  // brief
    (rt.provider.clone(), rt.model_id.clone())
};  // lock released
let result = provider.chat(...).await;  // no lock
```

### L2 collapse

After `MAX_CONSECUTIVE_FAILURES` (5) consecutive L2 rejections, the audit engine enters collapsed state — all requests are rejected. This prevents resource waste when the audit system itself is broken.

```rust
pub struct L2RuleAuditEngine {
    consecutive_failures: AtomicU32,  // increments on rejection
    collapsed: AtomicBool,            // set when >= MAX
    max_consecutive: u32,
}
// Collapse resets only on manual intervention or restart.
```

### Zero-tolerance behavioral contract

`ZERO_TOLERANCE_INSTRUCTIONS` in `src/core/constants.rs` is appended to every agent's system prompt. It enforces:

1. **Code completeness** — no placeholders, no `TODO`, no pseudo-code
2. **Deterministic chain-of-thought** — invariants, edge cases, error paths, lifetime analysis in `<cognitive_scratchpad>`
3. **Tool call discipline** — never blind-write; verify state before mutation; check every tool result
4. **Refusal protocol** — refuse to write code when context is insufficient; never guess

This is enforced at the prompt level, not at the Rust level — a behavioral contract rather than a compiler-enforced one.
