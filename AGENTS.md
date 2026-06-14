# Workflow Repository

## Overview
A Rust implementation of a holographic self-evolving multi-agent system with layered decision architecture (L-1/L0/L1/L2), dynamic experience pool, and conflict arbitration.

# 核心执行准则：零妥协与防御性编程 (Zero-Tolerance & Defensive Execution)

你正运行在一个任务关键型（Mission-Critical）的 Rust 系统中。系统对你的默认立场是"有罪推定（Presumed Guilty）"。任何由于你的草率、偷懒或主观臆断导致的编译失败、死锁或状态不一致，都将触发 L0/L2 熔断，直接判定当前任务失败。

## 1. 严格的代码完整性约束 (Code Completeness)
- **禁用任何占位符**：绝对禁止在输出的代码中使用 `// TODO`、`// 依此类推`、`// 请在这里实现逻辑`、`...` 或任何形式的伪代码占位符。
- **全量代码交付**：如果需要修改一个函数，你必须输出该函数 100% 完整的、可直接编译的代码，包含所有的修饰符、泛型约束和生命周期标注。禁止只写修改部分。
- **上下文继承**：在重构代码时，必须保留并正确处理原有文件的所有依赖项、导入语句（Imports）和现有的辅助函数，不得在生成新代码时选择性遗漏。

## 2. 确定性思维链 (Chain of Thoughts & Invariants)
在输出任何实际代码之前，你必须在 `<cognitive_scratchpad>` 标记块内进行显式推演，且推演必须覆盖以下四点：
1. **状态不变量 (Invariants)**：这段代码必须维持的物理/逻辑边界是什么？
2. **破坏性自检 (Breaking Edge Cases)**：如果传入空值、并发冲突、边界溢出或异步超时，这段代码会发生什么？
3. **显式错误路径 (Error Paths)**：禁止吞掉任何错误。所有的 `Result` 必须被显式处理，严禁使用 `.unwrap()` 或 `.expect()`（除非在单测中）。
4. **生命周期与所有权 (Ownership & Lifetimes)**：涉及到异步 Tokio 调度或跨线程传递时，引用的生存期和 `Send + Sync` 标记是否绝对安全？

## 3. 工具调用的有罪推定 (Tool Call Discipline)
- 在调用任何写操作工具（如 `WriteFile`, `PatchFile`, `Shell`）前，必须通过读工具（如 `ReadFile`, `Grep`）百分之百确认当前的目标状态，严禁基于历史记忆进行"盲写"。
- 如果某项任务需要连续调用 3 个以上的工具，你必须在每一步调用后检查其副作用（Side Effects）和返回值。一旦发现异常返回值，立即停止后续调用并进入自我修复逻辑。

## 4. 输出拒绝协议 (Refusal Protocol)
如果你因为上下文不足、语义模糊或缺少关键依赖而无法写出 100% 正确且能跑通的代码：
- 严格禁止"先随意写一个凑合的实现"。
- 你必须使用明确的结构说明你缺少什么（例如：缺少特定的结构体定义或 API Key 权限），并向用户请求明确的输入。此时，"拒绝编写"比"写出错误代码"的Credibility权重更高。

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

### Agent Lifecycle
```
User Goal → AgentRuntime.spawn_root_agent() → AgentPool → Execute
  ├─ Decision Pipeline (L-1/L0/L1/L2) approves spawn
  ├─ BudgetGuard attached, agent enters Idle → Planning
  ├─ LLM execution (chat_with_tools_stream_mcp / chat)
  ├─ Experience recorded (tool_bitmap, embedding, weight)
  ├─ Agent → Completed/Failed, BudgetGuard released
  └─ TTL eviction cleans stale agents from pool
```

### MCP Tool System
```
AppState → create_agent_tool_server() → ToolServerHandle
  ├─ Built-in tools: ReadFile, WriteFile, Shell, ListDir, Grep,
  │                  FindFiles, MoveFile, CopyFile, DeleteFile,
  │                  AppendFile, PatchFile, Glob, LineEdit, Fetch
  ├─ Agent tools: spawn_agent
  └─ Memo tools: read_memo, write_memo, delete_memo, list_memos
```

### Role Template & Optimization Flow
```
Agent with role_template_id → Experience recorded per-role
  ├─ /role list → show all templates
  ├─ /role embed → async computation of embeddings (tokio::spawn)
  ├─ /role optimize [name] → LLM analyzes experiences → improved prompt
  └─ OptimizationTracker enforces rate limits (1h + 3 new experiences)
```

### Core Modules
- `core/types.rs`: Core data structures (TaskId, AgentId, SpawnRequest, ExperienceEntry)
- `core/conflict.rs`: Conflict types and arbitration results
- `core/simd.rs`: SIMD-optimized cosine similarity
- `core/constants.rs`: Core constants
- `resource.rs`: TaskResourceState and BudgetGuard (RAII)
- `admission.rs`: L-1 semaphore-based admission control
- `l0.rs`: L0 circuit breaker (CAS budget, depth check, tool lock)
- `agent/` (agent.rs, plan.rs, suspend.rs): Agent lifecycle, plan parsing, suspend queue
- `l1/` (mod.rs, classifier.rs, arbitration.rs): L1 experience retrieval, value classifier, cognitive arbitration
- `l2/` (mod.rs, llm.rs): L2 rule-based audit + LLM-powered audit with judge personas
- `llm/` (mod.rs, types.rs, factory.rs, chat.rs, embed.rs, embedding.rs): LLM abstraction using rig (OpenAI/Anthropic providers), embedding router
- `models.rs`: Model registry with models.dev/api.json integration
- `runtime/` (runtime.rs, pipeline.rs, config.rs, optimizer.rs): Agent runtime wiring pipeline, role templates, prompt optimization
- `tui/` (mod.rs, state.rs, render.rs, handler.rs, chat.rs, chat_lines.rs, commands.rs, controller.rs, effect.rs, keymap.rs, popup.rs, status.rs, style.rs): Terminal UI dashboard with ratatui
- `config.rs`: Unified provider configuration layer (env/file/defaults merging)
- `provider.rs`: Provider client pool with health tracking
- `persistence.rs`: State persistence with atomic writes, obfuscated key store
- `reflection.rs`: Structured reflection pipeline (6 rules + LLM self-check)
- `tools/` (mod.rs, builtin.rs, agent.rs, memo.rs): MCP tool system (14 built-in tools + agent/memo tools)
- `experience/` (pool.rs, dual_track.rs, clustering.rs, role_template_store.rs, simple_retriever.rs): Experience pool, dual-track memory, clustering, role templates

### Key Data Structures
- `SpawnRequest`: Task/role/value embeddings (768-dim), budget, depth
- `ExperienceEntry`: Embedding, applicability vector, tool bitmap, weight
- `BudgetGuard`: RAII resource guard with `settle(actual)` and auto-rollback
- `ConflictManifest`: Conflict type, contending agents, context embeddings
- `Agent`: Full agent with id, role, goal, config, status, context (message history), parent/children tracking
- `AgentPool`: Thread-safe pool with budget guards, role-scoped memos, TTL eviction, completion notifiers
- `MemoEntry`: Role-scoped key-value scratchpad with timestamp
- `RoleTemplate`: Role name, label, system prompt, embedding, min_experiences for spawn threshold
- `ProviderConfig`: Unified provider config with protocol, timeout, retry, connection settings
- `ProviderClient`: Tracked provider client with health/last_used/error_count atomics
- `AgentRuntimeConfig`: Tuning knobs for all pipeline layers (budget, depth, timeout, threshold)

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
- `@`: Open file picker (type path after `@` to filter, Enter to select)
- `Alt+Enter`: Insert newline (multi-line input, grows up to 5 lines)

### Commands (type `/` for popup)
- `/connect`: Configure a provider (fetches models.dev API, shows cached data immediately)
- `/models`: Open model picker
- `/pool`: Pool management (stats/flush/clear/query)
- `/role`: Role templates (list/show/create/edit/delete/embed/optimize/default)
- `/memo`: Role-scoped memos (list/show/write/delete/roles)
- `/reflect`: Control reflection (on/off/status/rule/max)
- `/sh <cmd>`: Run a shell command
- `/clear`: Clear conversation
- `/help`: Show help
- `/keymap`: Show keyboard shortcuts

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

### Sub-Command Popups
- Commands with subcommands (`/role`, `/pool`, `/memo`) show a popup with available actions
- Dynamic items (role names, memo keys) resolved at runtime via `resolve_dynamic_items()`
- `j/k` navigate, Enter select, Esc cancel
- Type to filter items

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
- **`@file` reference support** — type `@` to open a file picker popup, navigate with arrows, select with Enter, automatically resolves to file content on submit
- **Reflection pipeline** — auto/manual self-check after agent responses: 6 local rules (code completeness, error awareness, multi-question coverage, empty promise detection, file ref usage, min output) + self-check LLM call ("yes/no", 1 token). Default off, opt-in via `/reflect on`. Configurable max retries and per-rule toggle.
- **14 built-in MCP tools** — ReadFile, WriteFile, Shell, ListDir, Grep, FindFiles, MoveFile, CopyFile, DeleteFile, AppendFile, PatchFile, Glob, LineEdit, Fetch — all implementing `rig::tool::Tool` trait
- **Memo MCP tools** — read_memo, write_memo, delete_memo, list_memos — role-scoped key-value scratchpad shared across agents of same role
- **Agent spawn MCP tools** — spawn_agent tool for dynamic agent creation at runtime
- **`create_agent_tool_server()`** — ToolServer with built-in + agent + memo tools, wired from AppState
- **Agent lifecycle** — AgentPool with Agent struct (id, name, role, depth, goal, config, status, context, children, parent), lifecycle states (Idle→Planning→AwaitingChildren→Aggregating→Completed/Failed)
- **Agent TTL eviction** — completed/failed agents evicted after configurable TTL (default 1h), preserves active/protected agents
- **Budget guard integration** — BudgetGuard attached to agents on spawn approval, released on completion/failure
- **Conversation context** — `Agent.context: Vec<Message>` stores message history for future LLM calls
- **`/role` command** — list, show, create, edit, delete, embed (auto-compute embeddings), optimize (LLM-driven prompt improvement), default (set bootstrap role)
- **`/memo` command** — list, show, write, delete, roles — manage role-scoped memos from TUI
- **`/keymap` command** — show all keyboard shortcuts
- **Provider configuration system** — ProviderConfig with multi-source merge (env variables / JSON file / defaults), timeout/retry/connection settings, requires_api_key/supports_tools protocol flags
- **ProviderClient with health tracking** — AtomicBool healthy flag, error count, last used timestamp, active connection tracking
- **Persistence with atomic writes** — state.json via write_atomic (rename-based), KeyStore XOR obfuscation, periodic flush, role memos persisted
- **Role template store** — ~/.workflow/role_templates.json, embedding-based similarity search, seeded defaults (general_business_analyst, tester, developer, reviewer, planner, security_auditor, researcher, devops), persisted via serde
- **Prompt optimization engine** — `/role optimize` collects experiences, runs LLM analysis, produces improved system prompt. OptimizationTracker with rate limiting (min interval, min new experiences)
- **Role embedding auto-computation** — on startup (async via tokio::spawn) and via `/role embed`, persisted to JSON
- **Refined TUI styling** — Catppuccin Mocha palette, bordered code blocks, inline backtick highlighting, styled popups, sub-command popups with dynamic item resolution
- **317 passing tests**, zero clippy warnings, clean cargo fmt

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
- Role templates backed by JSON file (~/.workflow/role_templates.json) instead of mmap (kept separate from experience pool)
- Role embeddings computed asynchronously on startup via tokio::spawn, persisted in JSON
- Prompt optimization uses LLM with structured analysis prompt (experience stats → improved prompt)
- OptimizationTracker prevents frequent re-optimization of same role (1h cooldown + 3 new experiences minimum)
- Memos are per-role (not per-agent), shared across all agents of same role, persisted in state.json
- Agent TTL eviction defaults to 1h, skips agents in non-terminal states (reusable idle agents retained)
- Tools: 14 built-in tools via rig's Tool trait + agent/memo tools separately registerable
- `create_tool_server()` for basic tools, `create_agent_tool_server()` for full tool stack with state access
- Provider config uses multi-source merge (env > JSON file > defaults), last-source-wins for collisions
- State persistence uses XOR obfuscation (machine-ID derived key) instead of plaintext; not real encryption
- Atomic writes via rename-based write_atomic (prevents partial write corruption)

### Next Steps
- Test with real API keys in interactive terminal
- Add `/pool export <path>` and `/pool import <path>` with JSON serialization
- Add pool stats auto-refresh in TUI sidebar (currently reads on render tick)
- Implement pool compaction (remove stale/low-weight entries from bedrock)
- Add memmap2 rescue/repair on file corruption
- Add agent-to-agent communication (inter-agent messages)
- Add `/plan` command for plan lifecycle management
- Wire agent spawn result back into TUI (currently silent on approval)
- Add agent health dashboard in TUI sidebar

### Relevant Files
- /home/user/Code/workflow/AGENTS.md: instruction file
- /home/user/Code/workflow/src/main.rs: entry point
- /home/user/Code/workflow/src/lib.rs: module declarations
- /home/user/Code/workflow/src/tui/: Terminal UI dashboard (state.rs, render.rs, handler.rs, chat.rs, commands.rs, effect.rs, controller.rs, popup.rs, keymap.rs, status.rs, style.rs, chat_lines.rs)
- /home/user/Code/workflow/src/runtime/: AgentRuntime (runtime.rs, pipeline.rs, config.rs, optimizer.rs)
- /home/user/Code/workflow/src/agent/: Agent lifecycle (agent.rs, plan.rs, suspend.rs)
- /home/user/Code/workflow/src/tools/: MCP tool system (mod.rs, builtin.rs, agent.rs, memo.rs)
- /home/user/Code/workflow/src/llm/: LLM abstraction using rig (mod.rs, types.rs, factory.rs, chat.rs, embed.rs, embedding.rs)
- /home/user/Code/workflow/src/experience/: Experience pool (pool.rs, dual_track.rs, clustering.rs, role_template_store.rs, simple_retriever.rs)
- /home/user/Code/workflow/src/core/: Core types (types.rs, conflict.rs, simd.rs, constants.rs)
- /home/user/Code/workflow/src/models.rs: model registry from models.dev/api.json
- /home/user/Code/workflow/src/config.rs: unified provider configuration
- /home/user/Code/workflow/src/provider.rs: provider client pool with health tracking
- /home/user/Code/workflow/src/persistence.rs: state persistence with atomic writes
- /home/user/Code/workflow/src/reflection.rs: structured reflection pipeline
- /home/user/Code/workflow/src/l0.rs: L0 circuit breaker
- /home/user/Code/workflow/src/admission.rs: L-1 semaphore admission
- /home/user/Code/workflow/src/l1/: L1 retrieval and value classifier (mod.rs, classifier.rs, arbitration.rs)
- /home/user/Code/workflow/src/l2/: L2 audit engines (mod.rs, llm.rs)
- /home/user/Code/workflow/src/resource.rs: TaskResourceState + BudgetGuard
