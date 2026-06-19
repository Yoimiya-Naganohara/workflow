# AGENTS.md

## Version control: jj (Jujutsu)

This repo uses **jj** (not git) for version control. Multiple agents work concurrently — jj's change-based model supports this natively.

### Critical jj workflow

```bash
jj status                  # always check before starting work
jj new                     # create a NEW change before editing files
jj describe -m "message"   # describe the current change AFTER editing
jj log --limit 5           # see recent history
```

**Rules:**
- **Always run `jj new` before making changes.** jj auto-commits working copy edits to the current change. Without `jj new`, you modify someone else's in-progress change.
- **Describe every change** with `jj describe -m "..."` before moving on.
- **Never work on `@` directly** if it already has a description — that's another agent's change.
- **Don't use `git` commands.** jj manages its own state; git commands bypass or conflict with it.
- **Resolve conflicts** with `jj resolve` — jj uses a 3-way merge model.

### Multi-agent coordination

- Each agent should own its own change (created via `jj new`).
- Use `jj log` to see what other agents are doing.
- `jj squash` can fold sub-changes into a parent when done.
- `jj abandon` discards a change that's no longer needed.

## Quick verification

```bash
cargo check && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

CI enforces all four gates independently. Run them in this order to match CI and fail fast.

## Build & run

```bash
cargo build --release        # LTO enabled, single codegen unit
cargo run --release          # TUI mode (requires interactive terminal)
cargo run --release -- --cli # headless CLI mode
```

Debug build (`cargo build`) works but TUI requires a real terminal.

## Single-test execution

```bash
cargo test <test_name>                  # by exact name
cargo test <module>::<test_name>        # scoped to a module
cargo test -- --nocapture               # with stdout
```

Tests live as `#[cfg(test)] mod tests` blocks inside their source files (45 modules). There are no separate `tests/` integration dirs.

## Codebase structure

Single Rust crate (edition 2024, MSRV 1.85). Not a workspace.

| Directory | Purpose |
|-----------|---------|
| `src/main.rs` | Entry — `--cli` flag selects CLI vs TUI |
| `src/core/` | Shared types, SIMD cosine similarity, conflict types |
| `src/admission.rs` | L-1: tokio semaphore concurrency gate |
| `src/l0.rs` | L0: circuit breaker, CAS budget, `BudgetGuard` RAII |
| `src/l1/` | L1: experience retrieval, value classifier, arbitration |
| `src/l2/` | L2: rule-based audit + LLM judge |
| `src/experience/` | Dual-track memory (mmap bedrock + fluid Vec), clustering |
| `src/llm/` | Provider abstraction (rig 0.38), embeddings (fastembed ONNX) |
| `src/runtime/` | Agent runtime, decision pipeline orchestration |
| `src/agent/` | Agent lifecycle, plans, suspend queue |
| `src/tools/` | MCP tool system (rmcp), built-in file/shell tools |
| `src/tui/` | ratatui terminal UI — events, rendering, dialogs |
| `src/config.rs` | Unified provider config layer |
| `src/persistence.rs` | `~/.workflow/` state (JSON + mmap) |

## Key conventions

- **Clippy:** `clippy.toml` sets `ignore-interior-mutability = ["tokio::sync::RwLock"]` — don't suppress that lint manually.
- **Formatting:** No `rustfmt.toml` — uses rustfmt defaults.
- **Release profile:** LTO + `codegen-units = 1` + symbol stripping. Release builds are significantly faster at runtime.
- **SIMD:** `src/core/simd.rs` uses AVX2+FMA intrinsics. Tests verify scalar fallback alignment.
- **Persistence:** State at `~/.workflow/state.json` (API keys in plaintext). Experience pool mmap at `~/.workflow/experience_a.bin`.
- **No workspace/cargo features** — everything compiles as one flat crate.

## Testing notes

- L0 tests spin 100 concurrent threads for CAS stress testing.
- SIMD tests validate error < 1e-5 against scalar reference.
- L2 tests use 50 adversarial samples — approval rate must stay < 15%.
- Use `tempfile` crate (dev-dependency) for tests needing filesystem.
- Use `tokio-test` crate (dev-dependency) for async test helpers.

## Gotchas

- TUI will not start in non-interactive environments (CI, pipes). Use `--cli` for headless.
- fastembed ONNX model downloads on first use and caches in `.fastembed_cache/`.
- The model registry (`models.dev/api.json`) is fetched lazily on `/connect`, not at startup.
- Edition 2024 requires Rust 1.85+ — older toolchains will fail with edition-related errors.
