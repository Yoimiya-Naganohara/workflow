# Workflow

**Holographic Self-Evolving Multi-Agent System** — a layered decision pipeline for spawning, auditing, and orchestrating AI agents at scale.

## Architecture

```
┌──────────────────────────────────────┐
│         L-1 (Admission Gate)         │  Token-bucket concurrency throttle
│         tokio semaphore              │
├──────────────────────────────────────┤
│         L0 (Circuit Breaker)         │  CAS budget guard, depth/conflict check
│         BudgetGuard / CAS            │
├──────────────────────────────────────┤
│         L1 (Arbitration)             │  Experience retrieval, value classifier
│         Retrieval + Classifier       │
├──────────────────────────────────────┤
│         L2 (Audit)                   │  Rule-based audit + LLM judge
│         Rule engine + LLM            │
├──────────────────────────────────────┤
│         Runtime Orchestrator         │  Agent lifecycle, pipeline, event loop
│         AgentRuntime                 │
├──────────────────────────────────────┤
│         Agent Pool                   │  Plans, suspend queue, eviction
│         AgentPool / Plan             │
├──────────────────────────────────────┤
│         Experience System            │  Dual-track: mmap bedrock + fluid Vec
│         Memory / Clustering          │
├──────────────────────────────────────┤
│         Tools (MCP)                  │  rmcp-based tool server, sandbox exec
│         Built-in / Shell / Memo      │
├──────────────────────────────────────┤
│         LLM Providers                │  OpenAI, Anthropic, Embeddings
│         rig 0.38 / fastembed ONNX    │
├──────────────────────────────────────┤
│         Terminal UI                  │  ratatui TUI — chat, agent tree, dialogs
│         ratatui / crossterm          │
└──────────────────────────────────────┘
```

Each spawn request flows through the decision pipeline:
1. **L-1** — Concurrency gate (tokio semaphore, configurable permits)
2. **L0** — Circuit breaker: CAS budget check, depth limit, resource conflict detection
3. **L1** — Experience-based arbitration: retrieves similar past spawns, classifies task/role/value alignments
4. **L2** — Rule-based audit followed by an LLM judge that evaluates the proposed spawn

## Quick Start

### Prerequisites

- **Rust 1.85+** (edition 2024)
- **jj** (Jujutsu) for version control — see [AGENTS.md](AGENTS.md)
- An **OpenAI** or **Anthropic** API key (optional — runs with a test key in CLI mode)

### Build & Run

```bash
# Release build (LTO + single codegen unit)
cargo build --release

# TUI mode (interactive terminal required)
cargo run --release

# Headless CLI mode
cargo run --release -- --cli
```

### CI Gates

```bash
cargo check && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

## Project Structure

```
src/
├── main.rs               # Entry — TUI or CLI based on --cli flag
├── lib.rs                # Module declarations
├── admission.rs          # L-1: tokio semaphore concurrency gate
├── l0.rs                 # L0: circuit breaker, CAS budget
├── config.rs             # Unified provider config layer
├── models.rs             # Model registry types
├── persistence.rs        # ~/.workflow/ state (JSON + mmap)
├── provider.rs           # LLM provider selection
├── reflection.rs         # Agent reflection / self-evaluation
├── core/
│   ├── mod.rs
│   ├── types.rs          # Shared types: AgentId, TaskId, SpawnRequest, etc.
│   ├── constants.rs      # Default constants (budget, depth, embedding dim)
│   ├── simd.rs           # AVX2+FMA cosine similarity (scalar fallback)
│   └── conflict.rs       # Resource conflict detection
├── agent/
│   ├── mod.rs
│   ├── agent.rs          # Agent lifecycle
│   ├── plan.rs           # Agent plans
│   └── suspend.rs        # Suspend queue
├── experience/
│   ├── mod.rs
│   ├── dual_track.rs     # Bedrock (mmap) + fluid (Vec) memory tracks
│   ├── pool.rs           # Experience pool management
│   ├── clustering.rs     # Experience clustering
│   ├── role_template_store.rs  # Role template persistence
│   └── simple_retriever.rs     # Nearest-neighbor retrieval
├── l1/
│   ├── mod.rs
│   ├── classifier.rs     # Value classifier (task/role/value)
│   └── arbitration.rs    # L1 arbitration logic
├── l2/
│   ├── mod.rs
│   └── llm.rs            # LLM judge for L2 audit
├── llm/
│   ├── mod.rs
│   ├── types.rs          # LLM provider types
│   ├── chat.rs           # Chat completions
│   ├── embed.rs          # Embedding abstraction
│   ├── embedding.rs      # fastembed ONNX embedding service
│   └── factory.rs        # Provider factory
├── runtime/
│   ├── mod.rs
│   ├── runtime.rs        # AgentRuntime orchestrator
│   ├── runtime_loop.rs   # Main event loop
│   ├── pipeline.rs       # Decision pipeline (L-1/L0/L1/L2)
│   ├── config.rs         # Runtime configuration
│   ├── optimizer.rs      # Budget optimizer
│   └── event.rs          # Runtime event types
├── tools/
│   ├── mod.rs
│   ├── agent.rs          # Agent tool server (MCP via rmcp)
│   ├── builtin.rs        # Built-in tools
│   ├── memo.rs           # Persisted role memos
│   └── sandbox.rs        # Isolated filesystem sandbox
└── tui/
    ├── mod.rs
    ├── state.rs           # AppState, PopupMode, Focus
    ├── render.rs          # Rendering layout
    ├── handler.rs         # Event handling
    ├── controller.rs      # TUI controller
    ├── effect.rs          # Side-effect handlers
    ├── runtime_bridge.rs  # Bridge to agent runtime
    ├── keymap.rs          # Key binding map
    ├── style.rs           # Styling / theme
    ├── status.rs          # Status bar
    ├── chat.rs            # Chat panel
    ├── chat_lines.rs      # Chat line rendering
    ├── agent_tree.rs      # Agent hierarchy tree
    ├── command_tree.rs    # Command tree view
    ├── commands.rs        # TUI commands
    ├── popup.rs           # Dialog popups
    └── tokenizer.rs       # Token counting for context windows
```

## Key Concepts

### Decision Pipeline

Every agent spawn request passes through a 4-stage pipeline:

| Stage | Layer | Purpose |
|-------|-------|---------|
| L-1 | `admission.rs` | Token-bucket concurrency limiter |
| L0 | `l0.rs` | CAS budget guard, depth/conflict checks |
| L1 | `l1/` | Experience retrieval + value classifier |
| L2 | `l2/` | Rule audit + LLM judge evaluation |

### Dual-Track Memory

The experience system uses a **dual-track** architecture:

- **Bedrock track** — memory-mapped (`mmap`) file for stable, long-term storage at `~/.workflow/experience_a.bin`
- **Fluid track** — in-memory `Vec` for fast read/write of recent experiences
- Automatic clustering and nearest-neighbor retrieval for L1 arbitration

### SIMD Acceleration

`src/core/simd.rs` implements cosine similarity using **AVX2+FMA** intrinsics with a verified scalar fallback (error < 1e-5).

### MCP Tool System

Agent tools are built on **rmcp** (rig's MCP implementation) and include:
- Built-in tools (read, write, edit, bash)
- Filesystem sandbox for isolated execution
- Persisted role memo system

## Version Control

This repository uses **jj** (Jujutsu), not git. See [AGENTS.md](AGENTS.md) for workflow details.

```bash
jj status        # Check current state
jj new           # Create a new change before editing
jj describe -m "message"
jj log --limit 5
```

## Configuration

- **API keys** stored in `~/.workflow/state.json` (plaintext)
- **Experience mmap** at `~/.workflow/experience_a.bin`
- **Provider auto-detection**: `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` env vars
- **Model registry** fetched lazily from `models.dev/api.json` on `/connect`

## Testing

```bash
# Run all tests
cargo test

# Single test
cargo test <test_name>

# With stdout
cargo test -- --nocapture
```

Key test areas:
- L0 CAS stress tests (100 concurrent threads)
- SIMD alignment against scalar reference (< 1e-5 error)
- L2 adversarial evaluation (< 15% approval rate on 50 adversarial samples)

## License

MIT — see [LICENSE](LICENSE) (not yet created).
