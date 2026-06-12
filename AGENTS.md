# Workflow Repository

## Overview
A Rust implementation of a holographic self-evolving multi-agent system with layered decision architecture (L-1/L0/L1/L2), dynamic experience pool, and conflict arbitration.

## Key Commands
- `cargo check`: Verify compilation
- `cargo build`: Build the project
- `cargo test`: Run tests
- `cargo fmt --check`: Check formatting
- `cargo clippy`: Run linter
- `cargo run -- --tui`: Launch TUI dashboard (requires interactive terminal)

## Architecture

### Decision Pipeline
```
SpawnRequest → L-1 (Admission) → L0 (Hard Logic + Physical Arbitration) 
             → L1 (Local Reasoning + Cognitive Arbitration) 
             → L2 (Remote Audit + Final Arbitration) → SpawnDecision
```

### Core Modules
- `types.rs`: Core data structures (TaskId, AgentId, SpawnRequest, ExperienceEntry)
- `conflict.rs`: Conflict types and arbitration results
- `resource.rs`: TaskResourceState and BudgetGuard (RAII)
- `admission.rs`: L-1 semaphore-based admission control
- `l0.rs`: L0 circuit breaker (CAS budget, depth check, tool lock)
- `suspend.rs`: SuspendQueue with priority ordering
- `simd.rs`: SIMD-optimized cosine similarity
- `l1/` (mod.rs, classifier.rs): L1 experience retrieval and value classifier
- `l1_arbitration.rs`: L1 cognitive arbitration
- `l2.rs`: L2 rule-based audit engine with collapse detection
- `l2_llm.rs`: L2 LLM-powered audit engine with judge personas
- `llm/` (mod.rs, types.rs, factory.rs, chat.rs, embed.rs): LLM abstraction using rig (OpenAI/Anthropic providers)
- `embedding.rs`: Embedding service with caching and normalization
- `models.rs`: Model registry with models.dev/api.json integration
- `runtime.rs`: Agent runtime wiring full pipeline
- `tui/` (mod.rs, state.rs, render.rs, handler.rs): Terminal UI dashboard with ratatui
- `plan.rs`: Plan lifecycle + execution (via `execute_plan`)

### Key Data Structures
- `SpawnRequest`: Task/role/value embeddings (768-dim), budget, depth
- `ExperienceEntry`: Embedding, applicability vector, tool bitmap, weight
- `BudgetGuard`: RAII resource guard with `settle(actual)` and auto-rollback
- `ConflictManifest`: Conflict type, contending agents, context embeddings

## Tech Stack
- Runtime: tokio + rayon
- LLM Framework: rig 0.38 (OpenAI/Anthropic providers)
- Embeddings: OpenAI text-embedding-ada-002 via rig
- Vector Index: Flat partition + SIMD (AVX2+FMA)
- Persistence: mmap (memmap2) + Arc delayed reclamation
- Clustering: Threshold-based Leader Clustering (Welford update)
- TUI: ratatui 0.30 + crossterm 0.29
- Model Registry: models.dev/api.json

## Environment Variables
- `OPENAI_API_KEY`: OpenAI API key
- `OPENAI_BASE_URL`: Custom OpenAI-compatible endpoint
- `OPENAI_MODEL`: Model to use (default: gpt-4)
- `ANTHROPIC_API_KEY`: Anthropic API key
- `ANTHROPIC_BASE_URL`: Custom Anthropic endpoint
- `ANTHROPIC_MODEL`: Model to use (default: claude-sonnet-4-20250514)

## Persistence
- State saved to `~/.workflow/state.json`
- Persists: selected models list, configured providers, API keys
- Loaded automatically on startup
- API keys stored in plain text (consider OS keychain for production)

## TUI Controls

### Requirements
- Interactive terminal (TUI will fail with "No such device" in non-interactive environments)
- Network access for model registry (models.dev/api.json)

### Chat Panel
- `Ctrl+P`: Open model picker dialog (shows all models, not just configured providers)
- `Ctrl+X`: Stop current response
- `j/k` or mouse wheel: Scroll chat messages
- `Enter`: Submit task
- `Esc`: Clear input
- `Tab`: Toggle Plan/Build mode
- `Cmd+/`: Command popup (type `/` to show available commands)
- `Ctrl+C`: Quit

### Commands (type `/` for popup)
- `/connect`: Configure a provider (fetches models.dev API, shows cached data immediately)
- `/models`: Open model picker
- `/apply`: Approve and execute plan
- `/clear`: Clear conversation
- `/sh <cmd>`: Run a shell command
- `/help`: Show help

### Model Picker
- Shows all models from all providers (not filtered by configured API keys)
- Unconfigured providers marked with `⌁` indicator
- Enter on an unconfigured model auto-prompts for API key
- Search by name, family, provider
- `j/k` navigate, Enter toggle, Esc cancel
- `Ctrl+A`: Open provider picker to configure providers

### Provider Dialog
- fzf-style type-to-filter (always-on)
- `j/k` navigate, Enter select, Esc cancel
- Searches provider name, ID, model names, and families

## Testing Strategy
- L0: 100-thread concurrent CAS, zero budget/tool lock leakage
- L1: Fixed experience set recall ≥ 99%, SIMD vs scalar error < 1e-5
- L2: 50 adversarial samples, approval rate < 15%, repair coverage > 90%
- Conflicts: Simulate resource/semantic/value conflicts, verify determinism

## Key Conventions
- Default stance: "Presumed guilty" - requests rejected unless sufficient evidence
- All parameters dynamic at runtime (no static config)
- Human only validates final output (acceptance/rejection)
- Experience-driven learning with credibility weighting
- Defense in depth: L0 physical immunity → L1 cognitive defense → L2 value audit

## Anchored Summary

### Goal
Build a holographic self-evolving multi-agent system in Rust with layered decision pipeline, TUI dashboard, model registry, and real LLM chat integration via rig.

### Constraints & Preferences
- All parameters dynamic at runtime, no static config
- Human only validates final output (acceptance/rejection)
- Defense in depth: L0 physical immunity → L1 cognitive defense → L2 value audit
- TUI must be usable, similar to opencode design
- Uses rig crate for LLM/embedding providers
- Models registry sourced from models.dev/api.json
- Default stance: "Presumed guilty" – requests rejected unless sufficient evidence
- Provider dialog: fzf-style type-to-filter (always-on), j/k navigate, Enter select, Esc cancel
- Models panel shows all models from all providers (unconfigured ones prompt for API key)
- Chat is for chatting with an agent for a goal, NOT for spawning agents

### Progress

#### Done
- Core data structures (SpawnRequest, ExperienceEntry, BudgetGuard, ConflictManifest)
- TaskResourceState with atomic CAS operations for budget/tool locks
- BudgetGuard RAII pattern with settle() and catch_unwind rollback
- L-1 admission control (tokio semaphore, 100ms timeout)
- L0 circuit breaker (CAS budget deduction, depth check, tool lock arbitration)
- SuspendQueue with priority ordering and timeout pruning
- SIMD cosine similarity for 384-dim vectors
- L1 experience retrieval with confidence threshold
- L1 value classifier (keyword-based)
- L1 cognitive arbitration (semantic conflict detection)
- L2 rule-based audit engine with collapse detection
- L2 LLM-powered audit engine with judge personas
- LLM trait abstraction using rig (OpenAI/Anthropic providers)
- Embedding service with dashmap caching and normalization
- AgentRuntime wiring full L-1→L0→L1→L2 pipeline
- TUI dashboard with ratatui (Chat + Models tabs, sidebar, input, status bar)
- Model registry fetching models.dev/api.json with search
- Provider selection dialog with fzf-style type-to-filter
- Key dialog for API key input with masked display
- Model picker showing all models, auto-prompts for API key on select
- Real LLM chat integration (tokio::spawn for async, displays response)
- Command popup (type `/` for suggestions, Tab/Enter select)
- UTF-8 input support (CJK, emoji, accented characters)
- Mouse scroll wheel support (chat + list navigation)
- Chat rendering cache (no rebuild on idle frames)
- Provider data cache (loads instantly, refreshes in background)
- `/sh <cmd>` shell command execution in chat
- Lazy models.dev fetch (only on `/connect`, not on startup)
- Module split: tui/, llm/, l1/ directories
- Dependency bumps: ratatui 0.30, crossterm 0.29, reqwest 0.13, rig 0.38
- **Experience pool with mmap persistence** (A-track bedrock, memmap2-backed, file format with header + entries, auto-grow, flush-on-mutation)
- **Dual-track memory** (A-track bedrock via mmap + B-track fluid via in-memory Vec, merged search with credibility weighting)
- **Leader clustering with Welford update** (online centroid/variance, configurable threshold, consolidate fluid→bedrock with min-cluster-size filter)
- **Auto-consolidation** (fluid auto-drains to bedrock via clustering when exceeding threshold)
- 92 passing tests, zero clippy warnings, clean compile
- **DualTrackMemory wired as default ExperienceRetrieval** in AgentRuntime::new() — new runtimes use persistent pool
- **Background pool flush** every 30s via tokio background task; final flush on shutdown
- **Pool stats in sidebar** (bedrock/fluid counts, pending suspend, budget, permits)
- **`/pool` command** with subcommands: stats, flush, clear, export, import
- **Thinking animation** — cycling dots (●●●) replacing static blinking indicator
- **Auto-scroll to bottom** during streaming responses
- **Word wrap** — long lines wrap at chat boundary instead of overflowing
- **Multi-line input** — `Alt+Enter` inserts newline, `Enter` sends, input box grows dynamically up to 5 lines
- **Improved code blocks** — bordered style (`┌─ lang` / `│` / `└───`) with better visual separation
- **Inline backtick highlighting** with italic cyan styling
- **Better status bar hints** — shows keyboard shortcuts contextually
- **MCP tool calling via `rmcp` crate** — integrated with rig's `ToolServer` + `ToolServerHandle` infrastructure
- **Built-in tools** — `read_file`, `write_file`, `sh`, `list_dir` implementing `rig::tool::Tool` trait with typed Args
- **Dynamic tool registration** — tools registered on `ToolServer` at runtime, agent uses `.tool_server_handle()`
- **Tool-enabled streaming** — `chat_with_tools_stream_mcp()` yields `ToolEvent::Text` / `ToolEvent::ToolCall` / `ToolEvent::Done`
- **Multi-provider support** — all 9 provider variants via `mcp_stream_arm!` macro
- **Tools wired into TUI chat** — `ToolServerHandle` in `AppState`, initialized with `create_tool_server()`
- **Tool call display** — tool invocations shown as `Decision` messages in chat with formatted args

#### In Progress
- (none currently)

### Key Decisions
- Used rig-core/rig for LLM providers instead of custom HTTP clients
- Used reqwest for models.dev fetch (separate JSON API, not LLM provider)
- Made model cost/temperature fields optional with serde defaults (ollama-cloud models lack them)
- Panels switched via Tab/1/2; Ctrl+C to quit (not 'q')
- Provider dialog: fzf-style always-on type-to-filter (no `/` prefix, no search mode toggle)
- j/k navigate providers, other chars type to filter, Enter opens key dialog, Esc cancel
- Key dialog: masked input, Enter sets env var + selects provider
- Ctrl+P opens model picker (shows all models from all providers)
- Unconfigured models marked with `⌁` indicator; Enter auto-prompts for API key
- Key dialog returns to model picker after setting key (not chat)
- Models picker maintains a ranked list of selected models for agent creation
- Always use model ID (not name) for agent creation
- Chat uses `LlmProvider::chat()` with system preamble and selected model
- Async LLM calls via `tokio::spawn` to avoid blocking TUI
- DualTrackMemory is the default ExperienceRetrieval; DecisionPipelineBuilder keeps L1Retriever for backward compat
- Added `flush()`, `bedrock_count()`, `fluid_count()` to ExperienceRetrieval trait with no-op defaults
- Background flush every 30s via tokio::spawn; final flush on TUI Drop
- `/pool` command parsed in handler, delegated to controller::execute_pool_command
- Pool stats shown in sidebar as: total, bedrock, fluid, pending suspend

### Next Steps
- Test with real API keys in interactive terminal
- Add `/pool export <path>` and `/pool import <path>` with JSON serialization
- Add `/pool clear` with confirmation (requires runtime write access)
- Add pool stats auto-refresh in TUI sidebar (currently reads on render tick)
- Implement pool compaction (remove stale/low-weight entries from bedrock)
- Add memmap2 rescue/repair on file corruption
- **P1: 角色 embedding 自动计算**（启动时 + `/role embed`）
- **P2: Prompt 优化引擎**（LLM 分析经验 → 改进提示词 → TUI 触发）
- **P3: 副作用与反馈**（工具使用记录 + L2 反馈）

### Completed
- **P0: 角色与经验连接**（Agent 带 role_template_id → 经验记录 → 按角色搜索 → Cluster 角色跟踪 → `/role` TUI 命令）

### Relevant Files
- /home/user/Code/workflow/AGENTS.md: instruction file
- /home/user/Code/workflow/src/tui.rs: TUI dashboard with ratatui (provider dialog, key dialog, model picker, real LLM chat)
- /home/user/Code/workflow/src/models.rs: model registry from models.dev/api.json
- /home/user/Code/workflow/src/runtime.rs: AgentRuntime wiring pipeline
- /home/user/Code/workflow/src/llm.rs: LLM trait using rig providers (chat, embed, from_env)
- /home/user/Code/workflow/src/embedding.rs: embedding service with cache
- /home/user/Code/workflow/src/l0.rs: L0 circuit breaker
- /home/user/Code/workflow/src/admission.rs: L-1 semaphore admission
- /home/user/Code/workflow/src/l1.rs: L1 retrieval and value classifier
- /home/user/Code/workflow/src/l2_llm.rs: LLM-powered L2 audit
- /home/user/Code/workflow/src/resource.rs: TaskResourceState + BudgetGuard
- /home/user/Code/workflow/src/suspend.rs: SuspendQueue with priority ordering
- /home/user/Code/workflow/src/core/simd.rs: cosine similarity for 384-dim vectors
- /home/user/Code/workflow/src/experience/pool.rs: mmap-backed experience pool (A-track)
- /home/user/Code/workflow/src/experience/dual_track.rs: dual-track memory (A-track + B-track)
- /home/user/Code/workflow/src/experience/clustering.rs: leader clustering with Welford update
