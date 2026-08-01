<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://img.shields.io/badge/workflow-runtime-8B5CF6?style=for-the-badge&logo=rust&logoColor=white">
    <img alt="workflow" src="https://img.shields.io/badge/workflow-runtime-8B5CF6?style=for-the-badge&logo=rust&logoColor=white">
  </picture>
</p>

<p align="center">
  <b>Multi-agent orchestration runtime with hierarchical delegation,<br>role-based agent pools, MCP tool integration, and a Tauri desktop UI.</b>
</p>

<p align="center">
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.96%2B-orange?style=flat&logo=rust" alt="Rust"></a>
  <a href="https://github.com/WorkflowTeam/workflow/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat" alt="License"></a>
  <img src="https://img.shields.io/badge/edition-2024-important?style=flat" alt="Edition 2024">
  <img src="https://img.shields.io/badge/status-alpha-yellow?style=flat" alt="Alpha">
</p>

---

## Why Workflow?

LLM agents today are isolated — each one operates alone, unaware of other agents, unable to delegate, and blind to the bigger picture. As tasks grow complex, a single agent hits context limits, loses focus, and produces shallow results.

**Workflow** turns that around. Instead of one agent doing everything, you get a **swarm of specialized agents** that:

- **Decompose** complex missions into smaller, focused tasks
- **Delegate** subtasks to the right agent for each job (planner, researcher, coder, reviewer…)
- **Orchestrate** execution in dependency-respecting waves — no step starts before its inputs are ready
- **Extend** their capabilities through MCP servers — filesystem, database, browser, or any MCP-compatible tool
- **Stay visible** — every agent streams text, reasoning, and tool calls to a desktop UI or CLI in real time

The result: each agent does one thing well, within its context window, and the runtime handles the coordination.

## How It Works

### Core Loop

1. **Create roles** — define agent personalities with system prompts (`planner`, `coder`, `reviewer`, ...)
2. **Send a mission** — the root agent receives your prompt and decomposes it into a DAG of tasks
3. **Tasks execute in waves** — each wave runs independent tasks in parallel; dependent tasks wait for their inputs
4. **Agents use tools** — built-in tools (`send_message`, `orchestrate`, `list_agents`) plus any MCP server tools you connect
5. **Results stream back** — text, reasoning, tool calls, and tool results appear in real time in the UI

### Quick Start

```bash
# 1. Set your API key
export OPENCODE_API_KEY="oc_..."

# 2. Build and run the CLI
cargo build --release
cargo run --release

# 3. Open the desktop UI (requires Node.js + pnpm)
cd crates/workflow-ui
pnpm install
pnpm tauri dev
```

Type a mission into the CLI or desktop UI, and watch agents decompose, delegate, and execute.

### Configuration

Provider settings live in `~/.workflow/provider_config.json`:

```json
{
  "id": "opencode",
  "name": "OpenCode AI",
  "protocol": "OpenAiCompatible",
  "base_url": "https://opencode.ai/zen/v1",
  "api_key": "sk-...",
  "models": ["big-pickle"]
}
```

Connect MCP servers (filesystem, browser, database, …) via `~/.workflow/mcp_servers.json`:

```json
[
  {
    "name": "filesystem",
    "transport": "stdio",
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
  }
]
```

### Prerequisites

- **Rust 1.96+** — for the CLI runtime
- **An API key** — any OpenAI-compatible provider (OpenCode AI, OpenAI, Anthropic, others)
- **Node.js 20+ and pnpm** — only needed for the desktop UI
- **MCP servers** — optional, for extended tool capabilities

## Project Structure

```
crates/
  workflow/          CLI binary
  workflow-core/     Runtime, event bus, agent factory
  workflow-agent/    Agent lifecycle, agent pool, A2A protocol
  workflow-tool/     Built-in tools (orchestrate, send_message, ...)
  workflow-mcp/      MCP client manager and server config
  workflow-config/   Provider configuration, XOR secret storage
  workflow-providers/ Model registry and cache
  workflow-role/     Role definitions and role pool
  workflow-ui/       Tauri 2 desktop app with Svelte 5 frontend
  playground/        Standalone test binary
```

For detailed architecture, crate map, and design principles, see [ARCHITECTURE.md](ARCHITECTURE.md).

## CI Gates

```bash
./ci.sh              # check, fmt, clippy, test, doc
./ci.sh --fix        # auto-fix formatting
```

## License

MIT — see [LICENSE](LICENSE).

---

<p align="center">
  <sub>Built with Rust, Tokio, rig, and rmcp.</sub>
</p>
