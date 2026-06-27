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
cargo check && cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo doc --no-deps
```

CI enforces all five gates independently. Run in this order to match CI and fail fast.

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

## GVSD — Global Verified System Design (MANDATORY)

Every agent operating on this codebase MUST follow the Global Verified System Designer
(GVSD) protocol before producing any output.  Non-compliance invalidates the result.

### Core principles

1. **Global Model First** — Construct a single unified model before reasoning.
   No fragmentation into cases, no ad-hoc branches.

2. **Abstraction Over Implementation** — Prioritize concepts, interfaces, contracts.
   Implementation details are secondary.

3. **Reusability First** — Every component must be reusable across contexts.
   Not tailored to a single scenario.

4. **Replaceability Principle** — All components must be modular, loosely coupled,
   replaceable without changing system semantics.  No hard-coded decision logic.

5. **Coverage Completeness** — Before any conclusion, enumerate full state space:
   edge cases, invalid cases, interaction cases.  If coverage is incomplete -> stop.

6. **Failure-First Testing** — Testing must try to BREAK the system.
   Adversarial inputs, counterexamples actively searched.
   Confirmatory-only tests are invalid.

7. **Verification Loop** — No output without adversarial testing + invariant checks +
   simulated execution of representative cases.

8. **Change Impact Safety** — Any modification must include dependency analysis,
   system-wide impact assessment, regression reasoning over prior valid behavior.
   Local patches without global analysis are forbidden.

### Required output structure

All responses must follow:

1. **Global Model** — Single coherent system explanation
2. **Core Abstractions** — Interfaces, contracts, coupling constraints
3. **System Architecture** — Component boundaries, data flow
4. **Coverage Analysis** — State space + edge cases + invalid cases
5. **Failure Mode Testing** — Adversarial inputs + counterexamples
6. **Verification Results** — Invariant checks + simulated execution
7. **Final Answer** — Only if fully verified

### Hard constraints (non-negotiable)

- No local / case-by-case reasoning as final logic
- No special-case rules or exception-based logic
- No toy code, minimal examples, or demo snippets as substitutes for design
- No one-off, non-reusable solutions
- No conclusions without full coverage analysis
- No results without adversarial testing
- No tests derived from the solution itself
- No fixes without system-wide impact analysis
- No local patches that risk regression elsewhere

### Violation handling

If an agent detects that a proposed solution violates GVSD, it MUST:
1. Raise the violation immediately
2. Identify which principle(s) are violated
3. Block output until resolved

No agent may bypass GVSD constraints for any reason.

## Gotchas

- TUI will not start in non-interactive environments (CI, pipes). Use `--cli` for headless.
- fastembed ONNX model downloads on first use and caches in `.fastembed_cache/`.
- The model registry (`models.dev/api.json`) is fetched lazily on `/connect`, not at startup.
- Edition 2024 requires Rust 1.85+ — older toolchains will fail with edition-related errors.
