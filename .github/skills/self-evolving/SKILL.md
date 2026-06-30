---
name: self-evolving
description: 'Systematic codebase evolution loop — observe, hypothesize, evolve, validate, and measure. Use when: evolving architecture, adding new capabilities, phasing in features, improving system behavior, or conducting iterative improvements across the self-evolving multi-agent system.'
argument-hint: 'What part of the system or capability needs evolution?'
---

# Self-Evolving Codebase — Evolutionary Improvement Loop

## When to Use
- Adding a new capability or phase to the multi-agent system
- Evolving architectural boundaries between crates
- Improving system-level behavior (throughput, latency, safety)
- Phasing in a staged feature across multiple iterations
- Conducting a systematic improvement cycle end-to-end
- Any change that benefits from a hypothesis→measure→learn loop

## Core Principle

This codebase is a **holographic self-evolving multi-agent system**. Every evolutionary change should follow a closed-loop cycle: **Observe → Hypothesize → Design → Implement → Validate → Measure → Learn**.

The GVSD protocol (see `AGENTS.md`) applies to every evolutionary step — no local patches without global model analysis.

---

## Workflow

```
┌──────────────────────────────────────────────────────────┐
│                    EVOLUTION LOOP                         │
│                                                          │
│  Observe ──▶ Hypothesize ──▶ Design ──▶ Implement        │
│     ▲                                        │           │
│     │                                        ▼           │
│     │                                  Validate          │
│     │                                        │           │
│     └────────── Learn ◀── Measure ◀──────────┘           │
└──────────────────────────────────────────────────────────┘
```

---

## Step 1 — Observe

Collect data to understand the current state of the system before proposing changes.

### 1.1 Establish baselines

Run the CI suite and capture results:
```bash
./ci.sh
```

Benchmark current performance if relevant:
```bash
cargo bench   # if benchmarks exist
```

### 1.2 Analyze system behavior

| Observation | Method |
|-------------|--------|
| Compile-time issues | `cargo check` errors, clippy warnings |
| Test coverage gaps | `cargo test` failures, missing test modules |
| Architectural drift | Circular deps, misplaced types, leaking abstractions |
| Performance hotspots | SIMD usage, lock contention, clone-heavy paths |
| Safety violations | `unsafe` without safety docs, `RwLock` across `.await` |
| Dead code / phase residue | `#[allow(dead_code)]` from previous phases |
| Event/log patterns | RuntimeEvent variants, pipeline decision distribution |

### 1.3 Map the current architecture

Identify the crate boundaries involved in the evolution target. Reference the dependency graph:

```
wf-core ← wf-llm ← wf-models
wf-core ← wf-experience ← wf-l1
wf-core ← wf-l2
wf-core ← wf-agent
wf-core ← wf-tools
wf-core ← wf-persistence
wf-core ← wf-reflection
wf-core ← wf-runtime ← wf-tui ← wf-workflow
```

All crates depend on `wf-core` and **no circular deps** are allowed.

### 1.4 Search for evolution signals

Look for markers that indicate staged/planned evolution:

```bash
# Phase markers — code staged for future extension
grep -rn "Phase [0-9]" crates/ --include="*.rs"

# TODO/FIXME — known incomplete areas
grep -rn "TODO\|FIXME\|HACK\|XXX" crates/ --include="*.rs"

# Dead-code allowances — likely phase residue
grep -rn "allow(dead_code)" crates/ --include="*.rs"
```

---

## Step 2 — Hypothesize

Formulate a clear, testable hypothesis for the evolution.

### 2.1 Hypothesis template

```
If we [change], then [observable effect] as measured by [metric],
because [rationale rooted in system model].
```

**Examples:**
- *"If we split `wf-runtime` into pipeline + lifecycle crates, then compile time decreases by 15% because the two halves change independently."*
- *"If we add SIMD fallback alignment tests, then portability increases without regression risk because the scalar path is verified against the AVX2 path."*
- *"If we replace the RwLock in experience retrieval with a CAS-based approach, then L1 latency drops by 20% because contention disappears."*

### 2.2 Identify risk category

| Risk | Criteria | Validation Required |
|------|----------|-------------------|
| Low | Pure additive (new type, new fn, new tests) | CI gates only |
| Medium | Refactors existing code (moves, renames, restructuring) | CI + GVSD coverage analysis |
| High | Changes core abstractions or pipeline semantics | Full GVSD + adversarial testing + behavior invariants |

### 2.3 Determine evolution strategy

| Strategy | When | Approach |
|----------|------|----------|
| **Refactor** | Same behavior, better structure | Extract, rename, deduplicate |
| **Phase** | Staged feature, incrementally enabled | Add behind gate, then wire up |
| **Replace** | Component swap, same interface | Build new, route traffic, remove old |
| **Extend** | Add capability without changing existing | New variants, new rules, new tools |

---

## Step 3 — Design (GVSD)

Apply the Global Verified System Design protocol before implementing. This is **mandatory** for medium- and high-risk changes.

### 3.1 Global Model

Produce a single unified explanation of the system component being evolved:
- What is its role in the broader system?
- What are its inputs, outputs, and invariants?
- How does it interact with adjacent components?
- What state does it manage and how?

### 3.2 Core Abstractions

Define the interfaces and contracts:
- Trait definitions with doc comments specifying pre/post conditions
- Type invariants (what makes a valid instance?)
- Error types and recovery semantics
- Dependency injection points (trait objects in builder pattern)

### 3.3 System Architecture

Map component boundaries and data flow:
- Where does this component sit in the pipeline?
- What events does it emit/receive (`RuntimeEvent` variants)?
- What data crosses module boundaries?
- Is the change within a single crate or does it cross crate boundaries?

### 3.4 Coverage Analysis

Enumerate the full state space before implementing:

| Dimension | Questions |
|-----------|-----------|
| Normal cases | Happy path, typical inputs, expected outputs |
| Edge cases | Boundary values, empty inputs, singleton/max |
| Error cases | Invalid inputs, resource exhaustion, timeouts |
| Concurrency | Race conditions, lock ordering, CAS retries |
| Composition | Combined with other features, pipeline stages |
| Degradation | Graceful fallback, partial failure, circuit breaking |

### 3.5 Failure Mode Testing (Adversarial)

For each hypothesis, design tests that try to **break** the system:
- What counterexample would disprove the hypothesis?
- What adversarial input would cause incorrect behavior?
- What concurrent interleaving would trigger a race?
- What resource limit would cause unexpected failure?

### 3.6 Invariant Verification

Identify invariants that must hold **before and after** the change:
- RAII gurantees (`BudgetGuard` drop safety)
- Lock ordering across the system
- No `RwLock` held across `.await`
- Bottom-up dependency direction
- Event sequence ordering guarantees

### 3.7 Generate `jj` change

```bash
jj status    # verify current state
jj new       # create a new change for this evolution step
```

---

## Step 4 — Implement

### 4.1 Follow codebase conventions

| Convention | Rule |
|------------|------|
| Edition | 2024, MSRV 1.85 |
| Formatting | rustfmt defaults (no config file) |
| Clippy | `clippy.toml` suppresses `ignore-interior-mutability` only |
| Unsafe | Every `unsafe` block needs a safety comment |
| Locks | Never hold `RwLock` across `.await` — extract data first |
| RAII | Use guard types for paired acquire/release (see `BudgetGuard`) |
| Visibility | Minimize `pub` — prefer `pub(crate)` |
| Errors | Use `anyhow::Result` for fallible functions |
| Events | Pipeline decisions emit `RuntimeEvent` variants |
| Tests | `#[cfg(test)] mod tests` as **last** item in each file |

### 4.2 Evolution-specific patterns

#### Phasing a new feature
```rust
// Phase 1: Add behind expect(dead_code) with doc comment
#[expect(dead_code)]
/// Reserved for Phase 2: [description of planned use]
fn new_capability() { /* ... */ }

// Phase 2: Wire into call sites, remove annotation
fn new_capability() { /* ... upgraded ... */ }
```

#### Extending an enum
```rust
#[non_exhaustive]  // already on public enums
pub enum RuntimeEvent {
    // existing variants...
    NewVariant(NewVariantData),  // ADD
}
```

Check all `match` statements — the compiler will flag unhandled arms with `#[non_exhaustive]` or you can add a `// TODO phase N` note.

#### Adding a new rule to the reflection engine
1. Add a new variant to `RuleId` in `wf-reflection`
2. Implement the rule function with the `fn check(...) -> RuleVerdict` signature
3. Register it in the `RulesEngine` builder
4. Add a test that exercises both pass and fail cases

#### Adding a new tool to the MCP system
1. Define the tool struct and implement the tool trait in `wf-tools`
2. Register in the tool registry (builder pattern)
3. Add to the agent's available tools list
4. Test via the sandbox executor

### 4.3 Cross-cutting changes

When an evolution affects multiple crates:

1. **Start from leaves** — Modify `wf-core` first, then walk up the dependency graph
2. **Commit per crate** — Make all changes to one crate before moving to the next
3. **Verify after each** — Run `cargo check -p <crate>` after modifying each crate
4. **Re-exports** — Use thin re-exports for backward compatibility when moving types:

```rust
// Old location — kept for backward compat during evolution
pub use wf_core::new_module::OldType;
```

---

## Step 5 — Validate

### 5.1 Run full CI gates
```bash
cargo check && \
cargo fmt --check && \
cargo clippy --all-targets -- -D warnings && \
cargo test && \
cargo doc --no-deps
```

Each gate must pass independently. Fail fast — don't proceed past a failing gate.

### 5.2 Run adversarial tests
```bash
cargo test -p wf-l2        # adversarial audit tests (50 samples, <15% approval)
cargo test -p wf-core      # SIMD alignment tests (<1e-5 error)
cargo test -p wf-runtime   # L0 CAS stress tests (100 concurrent threads)
```

### 5.3 Verify invariants

Checklist:

- [ ] No `RwLock` held across `.await` in changed code
- [ ] `jj status` shows only intended modifications
- [ ] No new `#[allow(dead_code)]` added without doc comment
- [ ] No new circular dependencies
- [ ] Public API additions are documented
- [ ] Match arms updated for all enum variants (if extended)
- [ ] `BudgetGuard` RAII pattern used for paired acquire/release

### 5.4 Behavioral validation

If the evolution changes observable system behavior:
- Run the TUI or CLI headless: `cargo run --release -- --cli`
- Verify the pipeline accepts/rejects expected inputs
- Check experience pool persistence: `~/.workflow/experience_a.bin`
- Validate model registry connectivity: `/connect` command

---

## Step 6 — Measure

Compare outcomes against the hypothesis.

### 6.1 Quantitative metrics

| Metric | Measurement | Compare Against |
|--------|-------------|-----------------|
| Compile time | `cargo build --release 2>&1 | tail` | Baseline from Step 1 |
| Test pass rate | `cargo test 2>&1 | tail -5` | Previous run |
| Binary size | `ls -lh target/release/workflow` | Baseline |
| Clippy count | `cargo clippy 2>&1 | grep "warning" | wc -l` | Baseline |
| Lint items | `grep -rn "allow(dead_code)" ... | wc -l` | Baseline |

### 6.2 Qualitative assessment

- Did the GVSD coverage analysis miss any edge cases?
- Are the abstractions cleaner or more complex than before?
- Is the system easier or harder to reason about?
- Did the change introduce new coupling?

### 6.3 Hypothesis validation

```
Hypothesis: [from Step 2]
Result:     [confirmed / refuted / inconclusive]
Evidence:   [metrics, test results, observations]
```

---

## Step 7 — Learn and Iterate

### 7.1 Describe the change
```bash
jj describe -m "evolve: <crate>: <summary of change>

- What: <what changed>
- Why: <rationale>
- Hypothesis: <original hypothesis>
- Result: <confirmed/refuted>
- Risk: <low/medium/high>
- Metrics: <before vs after>"
```

### 7.2 Decide next action

| Outcome | Action |
|---------|--------|
| Hypothesis confirmed | Squash change, move to next evolution target |
| Hypothesis refuted | Abandon change (`jj abandon`), document why, explore alternative |
| Inconclusive | Add more metrics, refine hypothesis, iterate again |
| Regression found | Fix regression (same change or `jj new` + fix) before proceeding |

### 7.3 Feed into next cycle

- Update phase markers and TODO comments to reflect new state
- If the evolution revealed deeper issues, create a follow-up hypothesis
- Update architecture documentation (`ARCHITECTURE.md` if relevant)
- If patterns emerge, consider updating `AGENTS.md` or creating a new skill

---

## Anti-patterns

- **Evolution without observation** — Changing code without understanding current behavior
- **Hypothesis-free changes** — Making edits without a clear prediction of outcome
- **Skipping GVSD on medium/high risk** — Local patches always cause regressions
- **Multiple evolutions in one `jj` change** — Keep each evolutionary step atomic
- **Measuring without baseline** — No way to know if things improved or regressed
- **Confirmation bias** — Only running tests that are expected to pass (run adversarial tests)
- **Gold-plating** — Evolving beyond what the hypothesis requires
- **Ignoring the loop** — Implementing without the observe→measure→learn cycle

## References

- `AGENTS.md` — GVSD protocol, jj workflow, build/test commands
- `ARCHITECTURE.md` — System architecture, pipeline layers, memory model
- `crates/wf-reflection/src/lib.rs` — Self-check rules engine
- `crates/wf-core/src/types.rs` — Core types and invariants
- `crates/wf-runtime/src/pipeline.rs` — Decision pipeline implementation
- `crates/wf-experience/src/dual_track.rs` — Two-tier memory model
