<div align="center">

# ⚡ Workflow

**Gate-first multi-agent orchestration — agents are *presumed guilty* until cleared.**

[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust&style=flat-square)](#)
[![MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Yoimiya-Naganohara/workflow/ci.yml?style=flat-square&logo=githubactions&logoColor=white)](https://github.com/Yoimiya-Naganohara/workflow/actions/workflows/ci.yml)
[![Tokio](https://img.shields.io/badge/async-tokio-ff69b4?style=flat-square)](#)
[![SIMD](https://img.shields.io/badge/SIMD-AVX2%2BFMA-9cf?style=flat-square)](#)

</div>

---

## 🧠 The Idea

Every agent framework works the same way: **trust the LLM, then verify**. Let the agent run, potentially waste tokens, and clean up afterward.

**Workflow inverts that.** Before a single token is spent or an agent spawned, every request must pass **four independent gates** — admission control, budget/depth check, experience-based confidence assessment, and a rule/LLM audit. One rejection means the agent **never exists**. No wasted inference. No runaway costs.

> **"Presumed guilty"** — agents don't *earn* the right to run. They *request permission*. The pipeline decides. The default is **no**.

```rust
// An agent must clear all four gates before it exists.
match runtime.spawn_agent(request).await {
    Ok(agent_id) => /* cleared all gates — agent is running */,
    Err(SpawnRejection::L1Rejected { reason, confidence }) =>
        log::warn!("L1 confidence {confidence:.2} below threshold: {reason}"),
    Err(SpawnRejection::BudgetExhausted { requested, remaining }) =>
        log::error!("budget: need {requested}, have {remaining}"),
    Err(SpawnRejection::DepthExceeded { current, max }) =>
        log::error!("depth {current} exceeds limit {max}"),
    Err(SpawnRejection::L2Collapsed) =>
        log::error!("audit engine collapsed — system needs stabilization"),
    Err(e) => log::error!("spawn failed: {e}"),
}
```

---

## 🏛️ Architecture — The Four Gates

```
                    ┌──────────────────────────────────────────┐
                    │            SPAWN REQUEST                  │
                    │  (task_emb, role_emb, budget, depth, …)  │
                    └────────────────┬─────────────────────────┘
                                     │
                            ┌────────▼────────┐
                   ┌────────┤   L-1 ADMISSION ├────── Tokio semaphore
                   │        └────────┬────────┘      max concurrent agents
                   │                 │ pass           timeout: 100ms
                   │        ┌────────▼────────┐
                   │        │  L0 BUDGET/DEPTH │────── CAS atomics (lock-free)
                   │        │  Circuit Breaker │       budget, depth, tool bitmap
                   │        └────────┬────────┘
                   │                 │ pass
                   │        ┌────────▼────────┐
                   │        │  L1 EXPERIENCE  │────── 384-d cosine similarity
                   │        │  Confidence     │       vs dual-track memory pool
                   │        └────────┬────────┘
                   │                 │ pass
                   │        ┌────────▼────────┐
                   │        │  L2 AUDIT       │────── Rule engine or LLM judge
                   │        │  (screen + arb) │       collapse protection
                   │        └────────┬────────┘
                   │                 │ pass
                   │        ┌────────▼────────┐
                   │        │  ✅ AGENT SPAWN │
                   │        │  (RAII guards)  │
                   │        └─────────────────┘
                   │
                   ▼
           ❌ REJECTED
           (SystemOverloaded | BudgetExhausted | DepthExceeded |
            ResourceConflict | L1Rejected | L2Rejected | L2Collapsed)
```

| Layer | Gate | What It Stops | Mechanism |
|-------|------|--------------|-----------|
| **L-1** 🚦 | Admission | Concurrency overload | Tokio semaphore with timeout |
| **L0** 💰 | Budget & Depth | Token drain, infinite recursion | CAS atomic counters (lock-free) |
| **L1** 🧪 | Experience | Repeating past failures | 384-d cosine similarity vs. dual-track memory |
| **L2** ⚖️ | Audit | Resource conflicts, value violations | Rule engine (or optional LLM judge), collapse recovery |

Every gate is a **Rust trait** — swap any layer independently without touching the runtime.

---

## ✨ Key Features

### 🔀 Gate-First Pipeline
Agents don't run and get fixed later. They request permission. The pipeline decides. Four independent checks, one rejection = spawn denied.

### 🧠 Dual-Track Memory (Experience Pool)
- **Bedrock track** — mmap-backed, survives restarts (`~/.workflow/experience_a.bin`)
- **Fluid track** — in-memory, bounded (configurable, default 512), fast writes
- **Clustering** — representative fluid entries auto-consolidate into bedrock using cosine-similarity clustering
- **Role-scoped search** — experiences filtered by `role_template_id` for per-role learning
- **Time decay** — older entries naturally lose influence (7-day half-life)

### 📊 Task Graph DAG
Full `Created → Ready → Running → Decomposed → Completed | Failed | Rejected` lifecycle. Tracks spawn hierarchy, execution ordering, and failure propagation with `FailFast` policy.

### 🔬 Reflection Engine
After an agent responds, **8 heuristic + semantic rules** verify output quality before accepting it:
- `code_complete` — balanced braces in code blocks
- `error_awareness` — tool errors must be acknowledged
- `multi_question_coverage` — proportional response length
- `empty_promise` — "I will…" backed by tool calls
- `file_ref_used` — `@file` references addressed
- `min_output` — meaningful length
- `relevance` — semantic cosine similarity (embedding-based)
- `semantic_promise` — promises match execution (embedding-based)

### 🛡️ Collapse Protection
L2 audit engine has a **circuit breaker**: after `N` consecutive high-risk failures, the engine "collapses" and prunes all contending agents. A **time-based decay** mechanism (5 min half-life) allows eventual recovery from transient failures.

### 🔒 Filesystem Sandbox
Each agent gets fully isolated filesystem access:
```
~/.workflow/sandbox/{agent_id}/
  ├── work/     ← writable (all writes, compilation, shell cwd)
  └── src → /project  ← read-only symlink to project root
```
`../` escapes, absolute paths outside the boundary, and symlink traversal that leaves the project root are **all rejected**.

### 🔌 9 LLM Backends
OpenAI, Anthropic, Gemini, Ollama, Cohere, Mistral, Azure, LlamaFile, Copilot — swap at runtime, no API rewrites via the [`rig`](https://github.com/0xPlaygrounds/rig) provider abstraction.

### 🖥️ Terminal UI
Built-in TUI powered by `ratatui` + `crossterm` with:
- Agent tree visualization
- Chat message streaming (thinking → streaming → completed)
- Command palette
- Session checkpoint/restore
- Crash recovery (auto-restart with state preservation)

### 🧰 MCP Tool System
Full tool ecosystem with `read_file`, `write_file`, `sh`, `diff_edit`, `search_asset`, inter-agent messaging, and memo (key-value scratchpad) tools — all behind MCP's `ToolServer` interface.

### 🧩 Pluggable Architecture
Every major component is a trait:
- `AdmissionControl` (L-1)
- `CircuitBreaker` (L0)
- `ExperienceRetrieval` (L1)
- `AuditEngine` (L2)
- `EmbeddingService`

Swap implementations at build time with zero runtime cost.

---

## 🚀 Quickstart

### Prerequisites
- **Rust 1.85+**
- An LLM provider (set via environment variable)

| Provider | Env Variable | Example |
|----------|-------------|---------|
| OpenAI | `OPENAI_API_KEY` | `sk-...` |
| Anthropic | `ANTHROPIC_API_KEY` | `sk-ant-...` |
| Ollama | `OLLAMA_BASE_URL` | `http://localhost:11434` |
| Gemini | `GEMINI_API_KEY` | `AIza...` |

### Run

```bash
cargo build --release

# Terminal UI (interactive)
cargo run --release

# Headless CLI mode
cargo run --release -- --cli
```

### Local Development

```bash
./ci.sh        # check, fmt, clippy, test, docs (all gates)
./ci.sh --fix  # auto-fix formatting
```

---

## 📦 Crates

```
workflow/
├── wf-core           # Foundation: types, SIMD cosine sim, constants,
│                     #   guard layer, task graph DAG, conflict domain,
│                     #   metrics, event system
├── wf-llm            # LLM provider abstraction — 9 backends via rig,
│                     #   embedding service (fastembed/ONNX)
├── wf-l1             # L1 experience-based confidence assessment,
│                     #   arbitration, value classification
├── wf-l2             # L2 rule audit engine, LLM-powered judge,
│                     #   collapse recovery, override patch generation
├── wf-experience     # Dual-track memory (bedrock + fluid),
│                     #   clustering, role template store
├── wf-models         # Model registry, provider config, cost tracking
├── wf-agent          # Agent pool, plan registry, sandbox, memos,
│                     #   inter-agent messaging, suspend queue
├── wf-tools          # MCP tools — ReadFile, WriteFile, Shell,
│                     #   DiffEdit, search, agent messaging, memos
├── wf-reflection     # 8-rule reflection engine (heuristic + semantic),
│                     #   self-check prompt, continuation feedback
├── wf-persistence    # State serialization, key-value store
├── wf-runtime        # Pipeline wiring, scheduler, lifecycle,
│                     #   checkpoint/restore, strategy graph
├── wf-tui            # Terminal UI (ratatui + crossterm)
└── wf-workflow       # Binary entrypoint (TUI or --cli)
```

---

## ⚙️ Configuration

```rust
AgentRuntimeConfig {
    max_concurrent_agents: 10,     // L-1 semaphore cap
    admission_timeout_ms: 100,     // L-1 wait timeout
    max_depth: 5,                  // L0 spawn depth limit
    initial_budget: 10_000,        // L0 token budget
    l1_confidence_threshold: 0.5,  // L1 confidence floor
    semantic_conflict_threshold: -0.6, // L2 conflict boundary
    suspend_timeout_ms: 50,        // suspend queue timeout
    bedrock_path: None,            // ~/.workflow/experience_a.bin
    role_template_path: None,      // ~/.workflow/role_templates.json
}
```

### Key Constants (tunable in `wf-core/src/constants.rs`)

| Constant | Default | Description |
|----------|---------|-------------|
| `EMBEDDING_DIM` | 384 | Embedding vector dimension (all-MiniLM-L6-v2) |
| `DEFAULT_MAX_DEPTH` | 5 | Maximum agent spawn depth |
| `DEFAULT_RUNTIME_BUDGET` | 10,000 | Initial token budget |
| `DEFAULT_L1_CONFIDENCE` | 0.5 | L1 confidence threshold |
| `MAX_CONSECUTIVE_FAILURES` | 5 | L2 collapse threshold |
| `L2_OVERRIDE_BOOST` | 1.5 | L2 override weight multiplier |
| `BUDGET_ANOMALY_RATIO` | 0.8 | Budget anomaly detection |
| `COLLAPSE_RECOVERY_SECS` | 300 | L2 time-based decay period |

---

## 🧪 Extending

### Swap an Audit Engine

```rust
use wf_runtime::pipeline::DecisionPipelineBuilder;

let pipeline = DecisionPipelineBuilder::new()
    .embedding(my_embedding_service)
    .audit_engine(Box::new(MyCustomAuditEngine::new()))
    .build();

let runtime = AgentRuntime::from_pipeline(pipeline);
```

### Use the LLM Judge for L2

```rust
use wf_l2::{L2LlmAuditEngine, L2LlmConfig};

let pipeline = DecisionPipelineBuilder::new()
    .embedding(my_embedding_service)
    .llm_audit_engine(
        Arc::new(provider),
        L2LlmConfig {
            model_id: "gpt-4".into(),
            temperature: 0.3,
            max_tokens: 500,
            ..Default::default()
        },
    )
    .build();
```

### Add a Custom Reflection Rule

```rust
use wf_reflection::{ReflectionRule, RuleContext, RuleVerdict, RuleRegistry};

struct MyRule;
#[async_trait]
impl ReflectionRule for MyRule {
    fn id(&self) -> &'static str { "my_custom_rule" }
    fn description(&self) -> &'static str { "Checks something important" }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        // Your logic here
        RuleVerdict::Pass
    }
}

let mut registry = wf_reflection::default_registry();
registry.register(Box::new(MyRule));
```

---

## 🗺️ Roadmap

| Phase | Focus | Status |
|-------|-------|--------|
| 1 | Task graph DAG delegation | ✅ Complete |
| 2 | Runtime analytics & optimization | 🔄 In progress |
| 3 | Inter-agent messaging & tool tracing | 🔄 In progress |
| 4 | Security audit & production hardening | 📋 Planned |

---

## 🤝 Contributing

PRs welcome — and they go through the **same four gates** as agents 😄

1. **`cargo check`** — must compile cleanly
2. **`cargo fmt --check`** — must be formatted (`./ci.sh --fix` to auto-fix)
3. **`cargo clippy -- -D warnings`** — no new warnings
4. **`cargo test`** — all tests pass

Run `./ci.sh` locally before pushing.

---

## 📄 License

MIT — see [LICENSE](LICENSE).
