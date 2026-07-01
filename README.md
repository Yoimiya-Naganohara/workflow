<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://socialify.git.ci/user/workflow/image?description=1&font=Inter&forks=1&issues=1&language=1&name=1&owner=1&pattern=Circuit%20Board&pulls=1&stargazers=1&theme=Dark">
  <img alt="workflow" src="https://socialify.git.ci/user/workflow/image?description=1&font=Inter&forks=1&issues=1&language=1&name=1&owner=1&pattern=Circuit%20Board&pulls=1&stargazers=1&theme=Light">
</picture>

<div align="center">

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org)
[![CI](https://img.shields.io/badge/CI-passing-brightgreen)](ci.sh)
[![SIMD](https://img.shields.io/badge/SIMD-AVX2%2BFMA-blueviolet)](crates/wf-core/src/simd.rs)
[![TUI](https://img.shields.io/badge/TUI-ratatui-ff69b4)](crates/wf-tui/)

</div>

# Workflow

**A holographic self-evolving multi-agent system** — a layered decision pipeline for spawning, auditing, and orchestrating AI agents at scale.

Workflow combines a lock-free circuit breaker, SIMD-accelerated experience retrieval, dual-track memory, and an LLM-powered audit engine to safely orchestrate collaborative AI agent hierarchies — all from a terminal UI or headless CLI.

## Features

- **🧠 4-Stage Decision Pipeline** — Every agent spawn passes through Admission (L-1), Circuit Breaker (L0), Experience Arbitration (L1), and Audit (L2) before execution.
- **🔒 Lock-Free Resource Accounting** — CAS loops with exponential backoff manage budget, tool conflicts, and depth — no coarse locks, no deadlocks.
- **⚡ SIMD-Accelerated Retrieval** — AVX2+FMA cosine similarity (384-dim vectors) for nearest-neighbor experience lookup. Verified scalar fallback with <1e-5 error.
- **🧠 Dual-Track Memory** — mmap-backed bedrock track (durable, cross-session) + in-memory fluid track (fast, bounded, FIFO-evicted). Automatic k-means consolidation.
- **🛡️ RAII Safety** — `BudgetGuard` auto-rolls back budget, tool bitmap, and depth on drop — even across panics.
- **🤖 LLM-Powered Audit** — L2 combines hardcoded rules with an LLM judge (OpenAI / Anthropic) for semantic screening. Collapses after 5 consecutive failures.
- **🔧 MCP Tool System** — Built on `rmcp` (rig's MCP) with 20+ tools: filesystem operations, sandboxed execution, agent management, and a persisted memo system.
- **🖥️ Terminal UI** — ratatui-powered interface with agent tree visualization, chat panel, command overlay, and real-time status.
- **👥 Multi-Agent Orchestration** — Hierarchical agent spawning with child-result synthesis, sibling inbox messaging, and suspend queue with priority-based eviction.
- **📦 13 Cargo Crates** — Clean bottom-up dependency graph. Zero circular dependencies. Edition 2024, MSRV 1.85.

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

Or use the CI script:

```bash
./ci.sh              # check all gates
./ci.sh --fix        # auto-fix formatting
```

## Architecture

```
┌──────────────────────────────────────────────┐
│         L-1 (Admission Gate)                 │  Token-bucket concurrency throttle
│         tokio semaphore                      │
├──────────────────────────────────────────────┤
│         L0 (Circuit Breaker)                 │  CAS budget guard, depth/conflict check
│         BudgetGuard / CAS                    │
├──────────────────────────────────────────────┤
│         L1 (Arbitration)                     │  SIMD experience retrieval, value classifier
│         Retrieval + Classifier               │
├──────────────────────────────────────────────┤
│         L2 (Audit)                           │  Rule-based audit + LLM judge
│         Rule engine + LLM                    │
├──────────────────────────────────────────────┤
│         Runtime Orchestrator                 │  Agent lifecycle, pipeline, event loop
│         AgentRuntime                         │
├──────────────────────────────────────────────┤
│         Agent Pool                           │  Plans, suspend queue, eviction
│         AgentPool / Plan                     │
├──────────────────────────────────────────────┤
│         Experience System                    │  Dual-track: mmap bedrock + fluid Vec
│         Memory / Clustering                  │
├──────────────────────────────────────────────┤
│         Tools (MCP)                          │  rmcp-based tool server, sandbox exec
│         Built-in / Shell / Memo              │
├──────────────────────────────────────────────┤
│         LLM Providers                        │  OpenAI, Anthropic, Embeddings
│         rig / fastembed ONNX                 │
├──────────────────────────────────────────────┤
│         Terminal UI                          │  ratatui TUI — chat, agent tree, dialogs
│         ratatui / crossterm                  │
└──────────────────────────────────────────────┘
```

Every spawn request flows through the pipeline:

1. **L-1** — Concurrency gate (tokio semaphore, configurable permits)
2. **L0** — Circuit breaker: CAS budget check, depth limit, resource conflict detection
3. **L1** — Experience-based arbitration: SIMD nearest-neighbor retrieval, task/role/value classification
4. **L2** — Rule-based audit followed by an LLM judge evaluation

## Project Structure

```
crates/
├── wf-core/           # Foundation: types, constants, SIMD, guard, task graph
├── wf-llm/            # LLM provider abstraction, embedding, chat (rig)
├── wf-l1/             # L1 experience retrieval & arbitration
├── wf-l2/             # L2 audit engine (rules + LLM judge)
├── wf-experience/     # Dual-track memory, clustering, role templates
├── wf-models/         # Model registry, provider config, provider client
├── wf-agent/          # Agent lifecycle, pool, plan, suspend, sandbox
├── wf-tools/          # MCP tool server, built-in tools, diff editor, memo
├── wf-persistence/    # State/session persistence, keystore
├── wf-reflection/     # Self-check, rule engine
├── wf-runtime/        # Pipeline, lifecycle, scheduler, orchestration
├── wf-tui/            # Terminal UI (ratatui)
└── wf-workflow/       # Binary entry point (TUI or CLI)
```

Dependency direction: **bottom-up**, no circular dependencies.

## Key Concepts

### Decision Pipeline

Every agent spawn request passes through a 4-stage pipeline:

| Stage | Layer | Purpose |
|-------|-------|---------|
| L-1 | Admission | Token-bucket concurrency limiter (tokio semaphore) |
| L0 | Circuit Breaker | CAS budget guard, depth/conflict checks |
| L1 | Arbitration | SIMD experience retrieval + value classifier |
| L2 | Audit | Rule audit + LLM judge evaluation |

### Dual-Track Memory

The experience system uses a **dual-track** architecture:

- **Bedrock track** — memory-mapped (`mmap`) file for stable, long-term storage at `~/.workflow/experience_a.bin`
- **Fluid track** — in-memory `Vec` for fast read/write of recent experiences
- Automatic k-means clustering promotes representative entries from fluid to bedrock

### SIMD Acceleration

AVX2+FMA cosine similarity for 384-dimensional vectors — 12 iterations of 256-bit SIMD per vector pair with a verified scalar fallback (error < 1e-5).

### MCP Tool System

Built on **rmcp** (rig's MCP implementation). 20+ tools including:
- Filesystem: read, write, edit, grep, glob, patch, bash
- Agent management: spawn, message, memo, list
- Per-agent filesystem sandbox at `~/.workflow/sandbox/<id>/`

### Zero-Tolerance Contract

Every agent receives a behavioral contract in its system prompt enforcing code completeness, deterministic chain-of-thought, tool call discipline, and a refusal protocol — no placeholders, no guessing.

## Documentation

| Document | Contents |
|----------|----------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Full architecture deep-dive: pipeline, memory, SIMD, agent lifecycle, TUI, safety contracts |
| [AGENTS.md](AGENTS.md) | Version control workflow (jj), CI gates, testing, conventions, GVSD protocol |

## Configuration

- **API keys** stored in `~/.workflow/state.json` (plaintext)
- **Experience mmap** at `~/.workflow/experience_a.bin`
- **Provider auto-detection**: `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` env vars
- **Model registry** fetched lazily on `/connect` (not at startup)

## Testing

```bash
# Run all workspace tests
cargo test --workspace

# Single test by name
cargo test <test_name>

# Scoped to a specific crate
cargo test -p wf-core

# With stdout
cargo test -p wf-core -- --nocapture
```

Key testing properties:
- **L0** — 100 concurrent threads for CAS stress testing
- **SIMD** — validates error < 1e-5 against scalar reference
- **L2** — 50 adversarial samples, approval rate must stay < 15%

## Contributing

We welcome contributions! Here's how to get started:

1. **Read the docs** — [ARCHITECTURE.md](ARCHITECTURE.md) (system design) and [AGENTS.md](AGENTS.md) (workflow & conventions)
2. **Set up** — Rust 1.85+, `jj` installed, `cargo build --release` compiles
3. **GVSD Protocol** — All changes must follow the Global Verified System Designer protocol (see [AGENTS.md](AGENTS.md)). This means: global model first, adversarial testing, and system-wide impact analysis before any patch.
4. **CI Gates** — Every PR must pass: `cargo check` → `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test` → `cargo doc --no-deps`
5. **Version control** — We use `jj` (Jujutsu). Create a new change with `jj new`, describe it with `jj describe -m "..."`.

## License

MIT — see [LICENSE](LICENSE).

## Acknowledgements

- Built with [rig](https://github.com/0xplaygrounds/rig) — the Rust LLM framework
- [ratatui](https://github.com/ratatui/ratatui) — terminal UI framework
- [rmcp](https://github.com/0xplaygrounds/rig) — Model Context Protocol implementation
- [fastembed](https://github.com/Anush008/fastembed-rs) — ONNX embeddings
- [memmap2](https://github.com/RazrFalcon/memmap2-rs) — memory-mapped file I/O
