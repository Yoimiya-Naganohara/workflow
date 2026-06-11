# Workflow

[![Rust](https://img.shields.io/badge/rust-1.85%2B-blue)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Edition](https://img.shields.io/badge/edition-2024-purple)](https://doc.rust-lang.org/edition-guide/)

**A holographic self-evolving multi-agent system with layered decision architecture, persistent dual-track memory, SIMD-optimized experience retrieval, LLM-powered auditing, MCP tool integration, and a real-time TUI dashboard.**

Workflow is a next-generation cognitive architecture for autonomous AI agents. It implements a four-layer defense-in-depth decision pipeline (L-1 → L0 → L1 → L2) that evaluates every agent spawn request through admission control, circuit-breaking, experience-based reasoning, and dual audit engines (rule-based + LLM-powered). The system learns continuously from its decisions using a persistent mmap-backed dual-track memory with online clustering consolidation.

---

## Architecture

### Decision Pipeline

```
SpawnRequest ──→ L-1 (Admission Control)
                      │
                      ▼
                  L0 (Circuit Breaker) — budget CAS, depth check, tool lock
                      │
                      ▼
                  L1 (Cognitive Reasoning) — experience retrieval, value classification, arbitration
                      │
                      ▼
                  L2 (Audit) ──→ Rule Engine ──→ SpawnDecision
                    or        ──→ LLM Judge  ──→ Approved | Rejected
```

**Default stance: "Presumed guilty"** — requests are rejected unless sufficient evidence exists in the experience pool to approve them.

| Layer | Name | Responsibility |
|-------|------|----------------|
| L-1 | Admission | Semaphore-based concurrency control with 100ms timeout |
| L0 | Circuit Breaker | Atomic CAS budget deduction, depth limits, tool lock arbitration via `BudgetGuard` (RAII) |
| L1 | Cognitive Defense | SIMD-optimized similarity search, value classifier, semantic conflict detection |
| L2 | Value Audit | Rule-based collapse detection **and/or** LLM-powered judge with coding, security, and ethics personas |

### Memory Architecture

```
┌──────────────────────────────────────────────────┐
│              DualTrackMemory                      │
│                                                   │
│  A-track (Bedrock)    B-track (Fluid)             │
│  ┌──────────────┐    ┌──────────────┐             │
│  │  mmap-backed │    │  in-memory   │             │
│  │  persistent  │    │  volatile    │             │
│  │  disk image  │    │  bounded Vec │             │
│  └──────┬───────┘    └──────┬───────┘             │
│         │                   │                      │
│         └─────┬─────────────┘                      │
│               ▼                                    │
│       Merged Search (credibility-weighted)         │
│               │                                    │
│               ▼                                    │
│    Leader Clustering (Welford update)              │
│    Fluid → Bedrock promotion on threshold          │
└──────────────────────────────────────────────────┘
```

- **A-track (Bedrock):** Persistent, mmap-backed experience pool. Survives restarts. Auto-flushed every 30s and on shutdown.
- **B-track (Fluid):** In-memory bounded `Vec`. New experiences land here first. FIFO eviction at capacity.
- **Clustering:** Threshold-based leader clustering with Welford's online algorithm. When fluid exceeds capacity, similar entries are consolidated into representatives and promoted to bedrock.
- **Search:** Both tracks are queried and merged with credibility weighting. SIMD-accelerated cosine similarity (AVX2+FMA) for 384-dim vectors.

---

## Features

### 🧠 Multi-Layer Decision Pipeline
- **Admission Control** — tokio semaphore limiting concurrent agents
- **Circuit Breaker** — CAS-based budget atomics, depth validation, tool lock arbitration with `BudgetGuard` RAII (auto-rollback on panic via `catch_unwind`)
- **Experience Retrieval** — SIMD cosine similarity with configurable confidence thresholds
- **Value Classification** — Keyword-based classifier for value alignment
- **Conflict Arbitration** — Semantic, resource, and value conflict detection and resolution

### 🗄️ Dual-Track Persistent Memory
- **mmap-backed bedrock** — memory-mapped file using `memmap2` crate with header + entries format, auto-growth
- **Fluid track** — in-memory bounded store with FIFO eviction
- **Welford clustering** — online centroid/variance tracking, threshold-based consolidation from fluid to bedrock
- **Background flush** — every 30 seconds; final flush on graceful shutdown
- **Pool statistics** — view counts via `/pool stats` command

### 🤖 LLM Integration (via `rig` 0.38)
- **9 provider variants:** OpenAI, Anthropic, Cohere, Google Gemini, Mistral, Ollama, Llamafile, Azure, GitHub Copilot
- **OpenAI-compatible** — works with DeepSeek, Groq, OpenRouter, and any OpenAI-compatible endpoint
- **Local embeddings** — fastembed (ONNX, all-MiniLM-L6-v2, 384-dim) with GPU acceleration (CUDA/CPU fallback)
- **Embedding cache** — DashMap-based with LRU-like semantics
- **Provider health tracking** — connection error counting, automatic health status

### 🛠️ MCP Tool System (via `rmcp`)
- **Built-in tools:** `read_file`, `write_file`, `sh`, `list_dir` implementing `rig::tool::Tool`
- **Dynamic registration** — tools registered on a shared `ToolServerHandle`
- **Tool-enabled streaming** — `chat_with_tools_stream_mcp()` yields text/tool-call/done events
- **Tool call display** — formatted `Decision` messages in the TUI chat

### 🖥️ Terminal UI (ratatui + crossterm)
- **Chat panel** with streaming responses, thinking animation, auto-scroll, word wrap
- **Model picker** — all models from all providers (unconfigured ones prompt for API key)
- **Provider dialog** — fzf-style type-to-filter, search by name/ID/family
- **API key dialog** — masked input, auto-sets environment variables
- **Custom provider wizard** — add custom OpenAI-compatible providers through the UI
- **Command popup** — type `/` for completions
- **Code blocks** — bordered style with language tags
- **Multi-line input** — `Alt+Enter` inserts newline, grows up to 5 lines
- **Mouse support** — scroll wheel for chat and list navigation
- **Chat rendering cache** — no rebuild on idle frames

### 🌐 Model Registry
- Fetches from [models.dev/api.json](https://models.dev/api.json)
- Lazily loaded (only on `/connect`, not on startup)
- Provider data cached for instant startup
- 100+ models across all major providers

### 📋 Plan System
- Plan lifecycle: creation, task assignment, status tracking
- Plan registry with agent-scoped lookups
- Task execution via `execute_plan`

### 🔧 Slash Commands
| Command | Description |
|---------|-------------|
| `/connect` | Configure a provider (fetches model registry) |
| `/models` | Open model picker to manage selected models |
| `/custom` | Add/list/remove custom OpenAI-compatible providers |
| `/clear` | Clear conversation history |
| `/sh <cmd>` | Run a shell command |
| `/pool` | Experience pool management (stats, flush, clear, export, import) |
| `/keymap` | Show keyboard shortcuts |
| `/help` | Show help |

---

## Getting Started

### Prerequisites

- **Rust** 1.85+ (edition 2024)
- An **interactive terminal** (TUI mode is not supported in non-interactive environments)
- **Network access** for model registry and LLM APIs
- Optional: **CUDA toolkit** for GPU-accelerated ONNX embeddings (falls back to CPU automatically)

### Build

```bash
# Release build (recommended, enables LTO)
cargo build --release

# Debug build
cargo build
```

### Run

```bash
# TUI mode (default) — interactive dashboard
cargo run --release

# CLI mode — headless single-request processing
cargo run --release -- --cli
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `OPENAI_API_KEY` | OpenAI API key | — |
| `OPENAI_BASE_URL` | Custom OpenAI-compatible endpoint | — |
| `OPENAI_MODEL` | Default OpenAI model | `gpt-4` |
| `ANTHROPIC_API_KEY` | Anthropic API key | — |
| `ANTHROPIC_BASE_URL` | Custom Anthropic endpoint | — |
| `ANTHROPIC_MODEL` | Default Anthropic model | `claude-sonnet-4-20250514` |

> API keys are stored in `~/.workflow/state.json` in plain text. Consider using a system keychain for production.

### Quick Start

1. **Launch the TUI**: `cargo run --release`
2. **Configure a provider**: Type `/connect` and select your provider from the dialog
3. **Enter your API key**: The key dialog will appear for providers that require authentication
4. **Select a model**: Press `Ctrl+P` to open the model picker and select models for your agent pool
5. **Start chatting**: Describe your goal in the chat input and press `Enter`

---

## TUI Controls

### Navigation

| Key | Action |
|-----|--------|
| `Ctrl+C` | Quit |
| `Ctrl+P` | Open model picker |
| `Ctrl+X` | Stop current response |
| `Enter` | Submit message |
| `Alt+Enter` | Insert newline in input |
| `Esc` | Clear input |
| `Tab` | Toggle Plan/Build mode |
| `j` / `k` | Scroll chat / navigate lists |
| Mouse wheel | Scroll chat / navigate lists |

### Commands

Type `/` in the chat input to see the command popup with available slash commands.

---

## Project Structure

```
src/
├── main.rs                  # Entry point: TUI or CLI mode
├── lib.rs                   # Module declarations
├── config.rs                # Unified provider configuration layer
├── models.rs                # Model registry (models.dev/api.json)
├── provider.rs              # Provider client pool with health tracking
├── persistence.rs           # State persistence (JSON + mmap)
│
├── core/                    # Foundation data structures
│   ├── types.rs             # Core types (TaskId, AgentId, SpawnRequest, ExperienceEntry, constants)
│   ├── conflict.rs          # Conflict types and arbitration results
│   └── simd.rs              # SIMD-optimized cosine similarity (AVX2+FMA)
│
├── admission/               # L-1: admission control
│   └── mod.rs               # Tokio semaphore, 100ms timeout
│
├── l0/                      # L0: circuit breaker
│   ├── mod.rs               # CAS budget, depth check, tool lock
│   └── resource.rs          # TaskResourceState + BudgetGuard (RAII)
│
├── l1/                      # L1: cognitive reasoning
│   ├── mod.rs               # Experience retrieval + confidence check
│   ├── classifier.rs        # Value classifier (keyword-based)
│   └── arbitration.rs       # Semantic conflict detection
│
├── l2/                      # L2: value audit
│   ├── mod.rs               # Rule-based audit (collapse detection)
│   └── llm.rs              # LLM-powered audit with judge personas
│
├── experience/              # Dual-track memory
│   ├── mod.rs               # Re-exports
│   ├── pool.rs              # mmap-backed A-track (bedrock)
│   ├── dual_track.rs        # DualTrackMemory (A + B track)
│   ├── clustering.rs        # Leader clustering (Welford update)
│   └── role_template_store.rs # Role template persistence
│
├── llm/                     # LLM abstraction layer
│   ├── mod.rs               # LlmProvider enum, ProviderProtocol, tests
│   ├── types.rs             # LlmRequest, LlmResponse, Message
│   ├── chat.rs              # Streaming chat with tools
│   ├── factory.rs           # Provider factory
│   ├── embed.rs             # Embedding via rig providers
│   └── embedding.rs         # Local fastembed service + EmbeddingRouter
│
├── runtime/                 # Agent runtime
│   ├── mod.rs               # AgentRuntime: config, role templates, agent pool
│   └── pipeline.rs          # DecisionPipeline builder + orchestration
│
├── agent/                   # Agent lifecycle
│   ├── mod.rs               # Agent, AgentPool, AgentStatus
│   ├── agent.rs             # Agent implementation
│   ├── plan.rs              # Plan lifecycle + execution
│   └── suspend.rs           # SuspendQueue with priority ordering
│
├── tools/                   # MCP tool system
│   ├── mod.rs               # ToolServerHandle, registration
│   ├── builtin.rs           # Built-in tools (read/write/sh/ls)
│   └── agent.rs             # Agent-aware tools
│
└── tui/                     # Terminal UI (ratatui)
    ├── mod.rs               # Tui struct, event loop, run()
    ├── state.rs             # AppState (messages, models, runtime)
    ├── render.rs            # Layout + rendering
    ├── handler.rs           # Input handling
    ├── sidebar.rs           # Sidebar panel
    ├── status.rs            # Status bar
    ├── style.rs             # Theme/style constants
    ├── effect.rs            # Effect system (async operations)
    ├── controller.rs        # Effect → AppEvent execution
    ├── commands.rs          # Slash command dispatch
    ├── chat.rs              # Chat renderer
    ├── chat_lines.rs        # Chat line wrapping
    ├── keymap.rs            # Keybindings
    └── dialogs/             # Dialog overlays
        ├── mod.rs
        ├── provider.rs      # Provider selection dialog
        ├── model_picker.rs  # Model picker dialog
        ├── key.rs           # API key input dialog
        ├── custom_wizard.rs # Custom provider wizard
        └── command_popup.rs # Command completion popup
```

---

## Key Concepts

### SpawnRequest
A request to spawn a new agent, containing:
- **Task description** and **role** expectations
- **Value embeddings** (768-dim combined vector)
- **Budget** (compute resources) and **depth** (spawn recursion level)
- **Tool bitmap** — which tools the agent is allowed to use

### ExperienceEntry
A recorded experience that influences future decisions:
- **Embedding** (384-dim) for similarity matching
- **Applicability vector** — which domains the experience applies to
- **Weight** — credibility score (boosted by L2 override)
- **L2 override weight** — how much the audit engine influenced it
- **Tool bitmap** — which tools were used
- **Timestamp** for recency decay

### BudgetGuard
RAII guard for resource management:
- Acquires budget atomically on creation
- `settle(actual)` adjusts budget on completion
- Auto-rollbacks on drop (panic-safe via `catch_unwind`)

### Conflict Types
- **ResourceLockContention** — two agents competing for the same resource
- **ActionContradiction** — agents taking contradictory actions
- **ValueDivergence** — agents diverging in value alignment

---

## Testing

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Check compilation
cargo check

# Lint
cargo clippy

# Format check
cargo fmt --check
```

The test suite covers:
- **L0**: 100-thread concurrent CAS, zero budget/tool lock leakage
- **L1**: Fixed experience set recall ≥ 99%, SIMD vs scalar error < 1e-5
- **L2**: 50 adversarial samples, approval rate < 15%, repair coverage > 90%
- **Conflicts**: Resource, semantic, and value conflict determinism
- **Clustering**: Welford update correctness, consolidation hygiene
- **Dual-track**: Track counts, filtering, push/promote semantics
- **SIMD**: Vector comparison correctness across alignment boundaries
- **LLM routing**: Provider protocol detection, `from_env`, `from_key`

---

## Persistence

State is stored in `~/.workflow/`:

```
~/.workflow/
├── state.json           # Provider configs, API keys, selected models
└── experience_a.bin     # Bedrock experience pool (mmap file)
```

- State is loaded automatically on startup
- Experience pool is flushed to disk every 30 seconds and on shutdown
- API keys are stored in plain text (consider OS keychain for production)

---

## Tech Stack

| Component | Technology |
|-----------|------------|
| Runtime | tokio + rayon |
| LLM Framework | [rig](https://github.com/0xPlaygrounds/rig) 0.38 |
| Embeddings | fastembed (ONNX, all-MiniLM-L6-v2) |
| Vector Similarity | SIMD (AVX2+FMA), 384-dim |
| Persistence | memmap2 + JSON |
| TUI | [ratatui](https://ratatui.rs) 0.30 + [crossterm](https://github.com/crossterm-rs/crossterm) 0.29 |
| MCP Tools | [rmcp](https://github.com/0xPlaygrounds/rmcp) via rig |
| Clustering | Threshold-based Leader Clustering (Welford update) |
| Model Registry | [models.dev/api.json](https://models.dev/api.json) |

---

## License

MIT

---

## See Also

- [AGENTS.md](./AGENTS.md) — Full project context, architecture deep-dive, and development notes
- [PROBLEMS.md](./PROBLEMS.md) — Known issues and limitations
- [plan_context_enhancement.md](./plan_context_enhancement.md) — Plan system design notes
