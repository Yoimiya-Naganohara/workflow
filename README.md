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

Missions are decomposed into a directed acyclic graph with a formal state machine:

```mermaid
stateDiagram-v2
    [*] --> Created
    Created --> Ready: dependencies satisfied
    Created --> Decomposed: sub-tasks spawned
    Created --> Dispatching: scheduler acquires lock
    Created --> Rejected: pipeline declined

    Ready --> Running: agent assigned
    Ready --> Dispatching: scheduler acquires lock
    Ready --> Decomposed: sub-tasks spawned
    Ready --> Completed: no agent needed
    Ready --> Failed: scheduling error
    Ready --> Rejected: pipeline rejected

    Dispatching --> Running: pipeline approved
    Dispatching --> Rejected: pipeline rejected
    Dispatching --> Created: pipeline error → retry

    Running --> Completed: success
    Running --> Failed: error

    Decomposed --> Completed: all children done
    Decomposed --> Failed: FailFast propagation

    Completed --> [*]
    Failed --> [*]
    Rejected --> [*]
    Blocked --> [*]
    Skipped --> [*]
```

Key properties:

- **Parent/children** = decomposition hierarchy (who spawned whom)
- **Dependencies** = execution ordering (what must finish first)
- **FailurePolicy::FailFast** — a child failure immediately marks the parent `Failed` and propagates upward
- **Dispatching** — anti-double-dispatch lock preventing duplicate scheduling

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

Two parallel memory tracks work in concert — one ephemeral and fast, one durable and clustered:

```mermaid
flowchart LR
    subgraph Ingest[Experience Ingestion]
        Outcome[Agent Outcome] --> Embedder[384-d Embedding]
        Embedder --> Fast[Fast Track]
        Embedder --> Fluid[Fluid Track]
    end

    subgraph Fast[Fast Track - Ring Buffer]
        E1[(Recent Exps)]
        E2[(High Decay)]
        E3[(Low Persistence)]
    end

    subgraph Fluid[Fluid Track - MMap File]
        C1[(Clustered<br/>Durable Store)]
        C2[(L2 Overridable<br/>Weights)]
        C3[(Domain<br/>Versioned)]
    end

    subgraph Retrieval[L1 Retrieval]
        Query[Spawn Request<br/>embeddings] --> Scorer
        Fast --> Scorer[Cosine Similarity<br/>AVX2+FMA]
        Fluid --> Scorer
        Scorer --> Weighted[Weighted Score<br/>task + role + value + recency]
        Weighted --> Decision{confidence<br/>≥ threshold?}
        Decision -->|yes| Approve[✅ Approve]
        Decision -->|no| Reject[❌ L1Rejected]
    end
```

| Track | Backing | Decay | Persistence | L2 Override |
|-------|---------|-------|-------------|-------------|
| **Fast** | In-memory `Vec` ring buffer | High | None | No |
| **Fluid** | `mmap` + binary file | Low | Durable | Yes — boosts weight ×1.5 |

The L1 retriever scores incoming spawn requests using SIMD-accelerated cosine similarity (AVX2+FMA) against 384-d embeddings, combining four weighted factors: task similarity, role similarity, value alignment, and temporal recency.

### Sandboxed Tool System

Every agent gets an isolated filesystem sandbox with copy-on-write semantics:

```mermaid
flowchart TB
    subgraph Host[Host Filesystem]
        Project[/project]
        SandboxDir[~/.workflow/sandbox/]
    end

    subgraph AgentSandbox[Agent Sandbox<br/>agent_id:8]
        Work[work/<br/>writable]
        Src[src → /project<br/>read-only symlink]
    end

    SandboxDir --- AgentSandbox
    Src -.->|symlink| Project

    subgraph Tools[Tool Resolution]
        Write[write /work/...] --> Allow[✅ Allowed]
        Read[read /src/...] --> Allow
        Escape[.../../escape] --> Block[❌ Blocked]
        Outside[write /project/...] --> Block
    end
```

Path traversal, symlink escapes leaving the project root, and writes to the source tree are all blocked. Tool catalog:

| Category | Tools |
|----------|-------|
| **Built-in** | `read`, `write`, `search`, `grep`, `glob`, `shell`, `diff_edit` |
| **Agent** | `spawn_child`, `send_message`, `read_messages`, `list_agents`, `search_asset` |
| **Memo** | `memo_write`, `memo_read`, `memo_list` — per-role scratchpad |
| **MCP** | Full `ToolServer` with streaming, tool chaining, and `ToolDyn` dynamic dispatch |

### Structured Reflection

A two-stage quality gate fires after every agent completion. Lightweight rules run first; only if they flag a problem does the LLM spend a token on self-verification:

```mermaid
flowchart TB
    AgentDone[Agent Responds] --> Rules{Rule Engine}

    subgraph Rules[Stage 1: Rule Engine]
        Len[📏 Length Check<br/>too short?] --> Pass1
        Rel[🎯 Relevance Check<br/>semantic similarity] --> Pass1
        Sem[📐 Semantic Promise<br/>embedding distance] --> Pass1
        Pass1{all rules pass?}
    end

    Pass1 -->|yes| SelfCheck{Stage 2:<br/>LLM Self-Check}
    Pass1 -->|no| SelfCheck

    SelfCheck -->|"yes (1 token)"| Done[✅ Accept]
    SelfCheck -->|"no (1 token)"| Continue[🔄 Continuation Round<br/>re-prompt agent]
    Continue --> AgentDone
```

### Persistence & Checkpointing

```mermaid
flowchart LR
    subgraph Runtime[Runtime State]
        Pool[Agent Pool]
        Graph[Task Graph]
    end

    subgraph Disk[~/.workflow/]
        direction TB
        AP[agent_pool.bin<br/>bincode]
        TG[task_graph.bin<br/>bincode]
        EP[experience_a.bin<br/>mmap + binary]
        RT[role_templates.json]
        ST[state.json<br/>obfuscated keys]
        SL[sessions/*.json]
    end

    Pool -->|on agent complete| AP
    Graph -->|on mutation| TG
    ExperiencePool -->|continuous| EP
    Config -->|on change| ST
    TUI -->|periodic| SL
    TemplateStore -->|on startup| RT
```

| Component | Format | Trigger | Recovery |
|-----------|--------|---------|----------|
| Agent Pool | `bincode` | Each agent completion | Full restore on restart |
| Task Graph | `bincode` | After each mutation | Rebuilt from checkpoint |
| Experience Pool | `mmap` + binary | Continuous (dual-track) | Instant — mmap persists in kernel |
| Role Templates | JSON | Read at startup | Missing file → seed defaults |
| State | JSON (XOR-obfuscated keys) | Config change | Graceful fallback |
| Session Logs | JSON | Periodic autosave | Chat history on TUI restart |

### Terminal UI

Built with [ratatui](https://github.com/ratatui/ratatui), the TUI provides a split-panel real-time cockpit:

```mermaid
flowchart TB
    subgraph Screen[TUI Layout - ratatui]
        Status[Status Bar<br/>runtime health · token usage · agent count]

        subgraph Main[Main Split]
            Sidebar[Left Sidebar<br/>Agent Tree<br/>hierarchy · status · tool trace]
            Content[Right Content<br/>Chat Panel / Command Palette / Diagnostics]
        end

        Command[Command Bar<br/>fuzzy-searchable commands]
    end

    subgraph State[Shared State - AppState]
        Core[Core Runtime<br/>agent pool · events · config]
        UI[UI State<br/>focus · scroll · selected agent]
    end

    RuntimeEvents[Runtime Event Stream] -->|broker| Core
    Core --> Sidebar
    Core --> Content
    KeyEvents[Keyboard/Mouse Events] --> Controller[Controller]
    Controller --> Core
    Controller --> UI
```

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

```mermaid
flowchart LR
    Clone[git clone] --> Prereqs{Rust 1.85+<br/>API key / local LLM}
    Prereqs --> Build[cargo build --release]
    Build --> Run[cargo run --release]
    Run --> TUI[🎛️ TUI launches]
    TUI --> Config[Configure providers<br/>in TUI command palette]
    Config --> Mission[🎯 Enter your mission]
    Mission --> Agents[🤖 Agents spawn & execute]
```

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

```mermaid
flowchart TB
    PR[PR / Push] --> Check[1. cargo check]
    Check --> Fmt{2. cargo fmt --check}
    Fmt -->|pass| Clippy[3. cargo clippy -D warnings]
    Fmt -->|fail + --fix| FmtFix[cargo fmt]
    Fmt -->|fail| Fail1[❌]
    Clippy -->|pass| Test[4. cargo test]
    Clippy -->|fail| Fail2[❌]
    Test -->|pass| Pass[✅ All Gates Pass]
    Test -->|fail| Fail3[❌]
```

| Gate | Command | Fail exit |
|------|---------|-----------|
| `cargo check` | `cargo check` | 1 |
| `cargo fmt` | `cargo fmt --check` (auto-fix via `--fix`) | 1 |
| `cargo clippy` | `cargo clippy -- -D warnings` | 1 |
| `cargo test` | `cargo test` | 1 |

## Design Principles

```mermaid
mindmap
  root((Workflow<br/>Principles))
    Lock-Free by Default
      CAS atomics for budget & depth
      Mutex only for short non-await ops
      Bounded contention ~10 agents
    RAII Lifecycle
      BudgetGuard releases on drop
      AdmissionPermit returns to semaphore
      SandboxHandle cleaned on eviction
    Dependency Injection
      Every pipeline layer swappable
      DecisionPipelineBuilder
      Mock-friendly for testing
    Fail-Closed
      L0 rejects on allocation failure
      L2 collapses on consecutive failures
      Sandbox blocks path escapes
      Checkpoint recovery on restart
    Observability
      Per-agent tool traces & metrics
      Token usage tracking
      Reasoning chain capture
      Real-time TUI diagnostics
```

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
