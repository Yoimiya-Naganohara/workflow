<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://img.shields.io/badge/workflow-runtime-8B5CF6?style=for-the-badge&logo=rust&logoColor=white">
    <img alt="workflow" src="https://img.shields.io/badge/workflow-runtime-8B5CF6?style=for-the-badge&logo=rust&logoColor=white">
  </picture>
</p>

<p align="center">
  <b>Multi-agent orchestration runtime with hierarchical delegation,<br>experience-driven learning, and sandboxed tool execution.</b>
</p>

<p align="center">
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?style=flat&logo=rust" alt="Rust"></a>
  <a href="https://github.com/WorkflowTeam/workflow/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat" alt="License"></a>
  <img src="https://img.shields.io/badge/edition-2024-important?style=flat" alt="Edition 2024">
  <img src="https://img.shields.io/badge/status-alpha-yellow?style=flat" alt="Alpha">
</p>

---

## Overview

**Workflow** is a Rust-native agentic runtime that orchestrates swarms of LLM-powered agents to decompose, delegate, and execute complex missions. It combines a multi-layer decision pipeline, a DAG-based task graph, experience-driven learning via semantic embeddings, and a sandboxed tool system — all surfaced through a rich terminal UI.

```mermaid
flowchart TB
    Mission -->|decompose| TaskGraph[Task Graph DAG]
    TaskGraph -->|spawn| DecisionPipeline
    TaskGraph -->|schedule| DecisionPipeline

    subgraph DecisionPipeline[Decision Pipeline]
        L1[L-1 Admission<br/>Tokio Semaphore]
        L0[L0 Circuit Breaker<br/>CAS Budget & Depth]
        L1e[L1 Experience Retrieval<br/>Semantic Similarity]
        L2[L2 Audit Engine<br/>Conflict Resolution]
        L1 --> L0 --> L1e --> L2
    end

    DecisionPipeline -->|approved| AgentPool
    DecisionPipeline -->|experience| ExperiencePool

    subgraph AgentPool[Agent Pool]
        Root[Root Agent]
        Child1[Child Agent 1]
        Child2[Child Agent 2]
        Root --> Child1
        Root --> Child2
    end

    subgraph ExperiencePool[Experience Pool]
        Fast[Fast Track<br/>In-Memory Ring]
        Fluid[Fluid Track<br/>MMap Persistent]
    end

    AgentPool --> Sandbox[Sandbox<br/>Filesystem]
    AgentPool --> MCPServer[MCP Tools]
    AgentPool --> TUI[TUI<br/>ratatui]
```

## Architecture

### Decision Pipeline

Every agent spawn request passes through a four-layer decision gate:

```mermaid
flowchart LR
    S[Spawn Request] --> L_1[L-1 Admission]
    L_1 -->|semaphore acquired| L0[L0 Circuit Breaker]
    L_1 -->|semaphore timeout| X1[❌ Rejected<br/>System Overloaded]
    L0 -->|budget & depth OK| L1[L1 Experience]
    L0 -->|budget exhausted| X2[❌ Rejected<br/>Budget Exhausted]
    L0 -->|depth exceeded| X3[❌ Rejected<br/>Depth Exceeded]
    L1 -->|confidence ≥ threshold| L2[L2 Audit]
    L1 -->|confidence < threshold| X4[❌ Rejected<br/>Low Confidence]
    L2 -->|arbitration passes| OK[✅ Approved<br/>ChildAgentConfig]
    L2 -->|rules violated| X5[❌ Rejected<br/>Prune / Collapse]
```

| Layer | Gate | Mechanism | Rejection |
|-------|------|-----------|-----------|
| **L-1** | Admission Control | Tokio `Semaphore` — caps concurrent agents | `SystemOverloaded` |
| **L0** | Circuit Breaker | CAS atomics on budget, depth, tool bitmap | `BudgetExhausted`, `DepthExceeded`, `ResourceConflict` |
| **L1** | Experience Retrieval | Cosine similarity (AVX2+FMA) against 384-d embeddings | `L1Rejected` with confidence score |
| **L2** | Audit Engine | Rules + priority scoring with automatic collapse | `Prune`, `Override`, `L2Collapsed` |

### Task Graph (DAG)

Missions are decomposed into a directed acyclic graph. Nodes track status through `Created → Ready → Running → Decomposed → Completed`, with `Failed`, `Rejected`, `Blocked`, and `Skipped` as terminal states. An anti-double-dispatch `Dispatching` lock prevents duplicate scheduling. Failure propagates via `FailurePolicy::FailFast` — a child failure immediately marks the parent `Failed` and cascades upward.

### Agent Lifecycle

```mermaid
flowchart TB
    subgraph Pipeline[Decision Gate]
        L1[L-1 Admission] --> L0[L0 Budget] --> L1e[L1 Experience] --> L2[L2 Audit]
    end

    subgraph Execution[Agent Execution]
        Create[Create Agent<br/>sandbox + config] --> Plan[Planning Phase]
        Plan --> Loop{Tool Call Loop}
        Loop -->|tool result| Loop
        Loop -->|needs delegation| Spawn[Spawn Children]
        Loop -->|Final Response| Complete[Complete]
        Spawn --> Aggregate[Aggregate Results]
        Aggregate --> Complete
    end

    subgraph Reflection[Reflection Pipeline]
        Rules[Rule Engine<br/>heuristic checks] --> SelfCheck[LLM Self-Check<br/>1-token yes/no]
        SelfCheck -->|both flag issue| Continue[🔄 Continuation Round]
        SelfCheck -->|pass| Done[✅ Done]
    end

    Pipeline -->|approved| Create
    Complete --> Reflection
```

### Experience Pool (Dual-Track Memory)

Two parallel memory tracks — an ephemeral in-memory ring buffer (fast) and an `mmap`-backed persistent store (fluid). The L1 retriever scores spawn requests via SIMD cosine similarity (AVX2+FMA, 384-d embeddings), combining task similarity, role alignment, value alignment, and recency.

| Track | Backing | Decay | Persistence | L2 Override |
|-------|---------|-------|-------------|-------------|
| **Fast** | In-memory ring buffer | High | None | No |
| **Fluid** | `mmap` + binary file | Low | Durable | Yes (×1.5 boost) |

### Sandboxed Tool System

Every agent gets an isolated sandbox (`~/.workflow/sandbox/{id}/`): a writable `work/` dir and a read-only `src` symlink to the project root. Path traversal, symlink escapes, and writes to the source tree are all blocked. Tool catalog:

| Category | Tools |
|----------|-------|
| **Built-in** | `read`, `write`, `search`, `grep`, `glob`, `shell`, `diff_edit` |
| **Agent** | `spawn_child`, `send_message`, `read_messages`, `list_agents`, `search_asset` |
| **Memo** | `memo_write`, `memo_read`, `memo_list` — per-role scratchpad |
| **MCP** | Full `ToolServer` with streaming, tool chaining, and `ToolDyn` dynamic dispatch |

### Structured Reflection

A two-stage quality gate after each agent completion: (1) lightweight heuristic rules (length, relevance, semantic promise), then (2) a 1-token LLM self-check. Only if both flag a problem does the runtime trigger a continuation round.

### Persistence & Checkpointing

| Component | Format | Trigger | Recovery |
|-----------|--------|---------|----------|
| Agent Pool | `bincode` | Each agent completion | Full restore on restart |
| Task Graph | `bincode` | After each mutation | Rebuilt from checkpoint |
| Experience Pool | `mmap` + binary | Continuous (dual-track) | Instant — mmap persists in kernel |
| Role Templates | JSON | Read at startup | Missing file → seed defaults |
| State | JSON (XOR-obfuscated keys) | Config change | Graceful fallback |
| Session Logs | JSON | Periodic autosave | Chat history on TUI restart |

### Terminal UI

Built with [ratatui](https://github.com/ratatui/ratatui): status bar, agent tree sidebar, chat panel / command palette / diagnostics content area, and a fuzzy-searchable command bar.

## Crate Map

```mermaid
flowchart TB
    subgraph Binary[Binary]
        WF[wf-workflow]
    end

    subgraph Core[Foundation]
        CORE[wf-core<br/>types · SIMD · guard · task graph]
        LLM[wf-llm<br/>providers · embedding · chat]
    end

    subgraph Agent[Agent Layer]
        AGENT[wf-agent<br/>pool · lifecycle · sandbox]
        TOOLS[wf-tools<br/>MCP server · built-in tools]
        REFLECT[wf-reflection<br/>rules engine · self-check]
    end

    subgraph Runtime[Runtime Layer]
        RUNTIME[wf-runtime<br/>pipeline · scheduler · checkpoint]
        L1[wf-l1<br/>experience retrieval]
        L2[wf-l2<br/>audit engine · arbitration]
    end

    subgraph Memory[Memory & Config]
        EXP[wf-experience<br/>dual-track · clustering]
        MODELS[wf-models<br/>registry · provider config]
        PERSIST[wf-persistence<br/>atomic I/O]
    end

    subgraph UI[UI]
        TUI[wf-tui<br/>ratatui terminal]
    end

    AGENT --> CORE
    AGENT --> LLM
    TOOLS --> AGENT
    TOOLS --> CORE
    REFLECT --> CORE
    REFLECT --> LLM

    RUNTIME --> CORE
    RUNTIME --> AGENT
    RUNTIME --> L1
    RUNTIME --> L2
    RUNTIME --> EXP
    RUNTIME --> PERSIST

    L1 --> CORE
    L2 --> CORE
    EXP --> CORE
    EXP --> LLM
    MODELS --> CORE

    TUI --> RUNTIME
    TUI --> CORE

    WF --> RUNTIME
    WF --> TUI
    WF --> MODELS
    WF --> PERSIST

    style WF fill:#8B5CF6,color:#fff
    style CORE fill:#3B82F6,color:#fff
    style RUNTIME fill:#10B981,color:#fff
```

## Getting Started

```bash
# Build
cargo build --release

# Run
cargo run --release

# CI gates
./ci.sh
```

### Prerequisites

- Rust 1.85+ (edition 2024)
- An LLM provider API key (OpenAI, Anthropic, etc.) or a local Ollama/Llamafile instance

### Configuration

Provider keys and model selection are configured through the TUI or persisted in `~/.workflow/state.json`. Keys can be stored in obfuscated form (XOR with machine ID) for casual security.

```bash
# Set environment variables for API keys
export OPENAI_API_KEY="sk-..."
export ANTHROPIC_API_KEY="sk-ant-..."
```

## CI Gates

```bash
./ci.sh           # Run all gates (check, format, clippy, test)
./ci.sh --fix     # Auto-fix formatting issues
```

| Gate | Command | Fail exit |
|------|---------|-----------|
| `cargo check` | `cargo check` | 1 |
| `cargo fmt` | `cargo fmt --check` (auto-fix via `--fix`) | 1 |
| `cargo clippy` | `cargo clippy -- -D warnings` | 1 |
| `cargo test` | `cargo test` | 1 |

## Design Principles

1. **Lock-free by default** — shared state uses CAS atomics; `Mutex` only for short, non-`.await`-held operations
2. **RAII resource lifecycle** — budget permits, admission slots, and sandbox handles all release on drop
3. **Dependency injection** — every pipeline layer can be swapped (mocks, custom audit engines, etc.)
4. **Fail-closed** — L0 rejects on allocation failure, L2 collapses on consecutive audit failures, sandbox rejects on path escape
5. **Observability** — every agent records tool traces, token usage, metrics, and reasoning for TUI diagnostics

## License

MIT — see [LICENSE](LICENSE).

---

<p align="center">
  <sub>Built with Rust, tokio, ratatui, rig, and fastembed.</sub>
</p>
