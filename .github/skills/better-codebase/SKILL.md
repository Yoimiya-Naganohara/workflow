---
name: better-codebase
description: 'Refactoring and code quality improvement for Rust codebases. Use when: cleaning up lints, restructuring code, improving architecture, applying GVSD, or enforcing CI gates.'
argument-hint: 'What part of the codebase needs refactoring?'
---

# Better Codebase — Rust Refactoring Guide

## When to Use
- Clippy lint cleanup (`collapsible_if`, `items_after_test_module`, dead code, etc.)
- Restructuring modules or crates (splitting, merging, extracting)
- Applying architectural patterns (builder, DI, RAII, phase-based evolution)
- Improving test coverage or test structure
- Enforcing CI gate compliance before committing
- Removing technical debt (unused code, TODO follow-ups, phase migrations)
- Any change requiring GVSD (Global Verified System Design) protocol

## Workflow Overview

```
1. Assess ──▶ 2. Plan ──▶ 3. Execute ──▶ 4. Verify ──▶ 5. Review
                     │                        │
                     └── backtrack ────────────┘
```

---

## Step 1 — Assess Current State

### 1.1 Run full CI gate suite
```bash
./ci.sh
```
This runs: `cargo check` → `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test`.

If `--fix` mode: `./ci.sh --fix` auto-formats but still reports clippy and test failures.

### 1.2 Collect all lint issues
```bash
cargo clippy --all-targets -- -D warnings 2>&1
```
Note each warning's file, line, and clippy lint name.

### 1.3 Check for dead code
```bash
cargo build --all-targets 2>&1 | grep "dead_code"
```
Also search for `#[allow(dead_code)]` annotations and verify they're still needed:
```bash
grep -rn "allow(dead_code)" crates/ --include="*.rs"
```

### 1.4 Identify phase-related TODOs
Search for Phase markers and TODO comments:
```bash
grep -rn "Phase [0-9]" crates/ --include="*.rs"
grep -rn "TODO\|FIXME\|HACK\|XXX" crates/ --include="*.rs"
```

### 1.5 Check for structural issues
- Items after `#[cfg(test)] mod tests` blocks (clippy `items_after_test_module`)
- Circular dependency risk between crates (dependency direction is bottom-up)
- `RwLock` held across `.await` points (deadlock risk)
- Missing doc comments on public items

---

## Step 2 — Plan the Refactoring

### 2.1 Apply GVSD before any change

Before touching code, produce a GVSD analysis:

1. **Global Model** — Unified explanation of the component and its role
2. **Core Abstractions** — Interfaces, contracts, coupling constraints
3. **System Architecture** — Component boundaries, data flow
4. **Coverage Analysis** — State space + edge cases + invalid cases
5. **Failure Mode Testing** — Adversarial inputs + counterexamples
6. **Verification Results** — Invariant checks + simulated execution
7. **Final Answer** — Only if fully verified

See `AGENTS.md` for full GVSD protocol. **No output without adversarial testing.**

### 2.2 Prioritize changes

Order by risk-to-reward ratio:

| Priority | Category | Examples |
|----------|----------|----------|
| P0 | Compiler-blocking | Edition migration, broken deps, type errors |
| P1 | Correctness | Deadlock risks, RAII violations, unsound `unsafe` |
| P2 | Lint hygiene | `collapsible_if`, `items_after_test_module`, unused imports |
| P3 | Architecture | Module extraction, phase migration, deduplication |
| P4 | Style / docs | Missing doc comments, formatting, naming consistency |

### 2.3 Create a change plan

For each change, document:
- **File(s)** affected
- **What** changes (specific code)
- **Why** (which principle/rule it serves)
- **Risk** of regression (low/medium/high)
- **Test coverage** that protects this area

### 2.4 Use `jj new` before making edits

This repo uses `jj` (not git). Always create a new change:
```bash
jj status
jj new
```

---

## Step 3 — Execute Changes

### 3.1 Apply lint fixes (most common)

#### Collapsible `if` statements
```rust
// Before
if let Some(node) = self.nodes.get_mut(&task_id) {
    if !node.dependencies.contains(&depends_on_id) {
        node.dependencies.push(depends_on_id);
    }
}

// After
if let Some(node) = self.nodes.get_mut(&task_id)
    && !node.dependencies.contains(&depends_on_id)
{
    node.dependencies.push(depends_on_id);
}
```
Use `let`-chains (`&& let Some(x) = expr`) for nested `if let`.

#### Items after test module
Move non-test items (impl blocks, trait impls, constants) **above** the `#[cfg(test)] mod tests` block. The test module should be the **last** item in the file.

#### Dead code handling
- If the code is truly unused → remove it
- If reserved for future use (Phase 2, etc.) → add `#[expect(dead_code)]` with a doc comment explaining the planned use
- If conditionally compiled → verify `#[cfg()]` gates are correct

#### Unused imports
```bash
cargo fix --bin workflow --allow-dirty  # auto-fix where safe
```

### 3.2 Structural refactoring

#### Module extraction (splitting large files)
1. Create new module file(s) in the appropriate crate
2. Move types, impls, and functions — keep `pub` visibility minimal
3. Re-export from the parent module for backward compatibility
4. Update `use` paths across the codebase
5. Add doc comments with `//!` module-level docs

#### Re-export pattern (backward compat)
When moving code to another crate, keep a thin re-export:
```rust
// Old location — kept for backward compatibility
pub use wf_core::guard::AdmissionControl;
```

### 3.3 Lock strategy enforcement

- **Never** hold `std::sync::Mutex` or `tokio::sync::RwLock` across `.await` points
- Extract data first (clone or copy), then call async
- Use `Arc<RwLock<T>>` for shared state, `Mutex` for short critical sections
- Document lock ordering if multiple locks are acquired

### 3.4 Phase-based evolution

When implementing Phase N+1 of a previously staged feature:
1. Remove old `#[allow(dead_code)]` / `#[expect(dead_code)]` annotations
2. Wire up the new code into the call sites
3. Update or remove transitional comments ("Phase N only")
4. Add diagnostic assertions that catch incomplete migration

---

## Step 4 — Verify

### 4.1 Run CI gates (in order, fail fast)
```bash
cargo check && \
cargo fmt --check && \
cargo clippy --all-targets -- -D warnings && \
cargo test && \
cargo doc --no-deps
```

Each gate must pass independently. This mirrors CI exactly (see `ci.sh`).

### 4.2 Verify no regressions
- Run tests that exercise the changed area specifically:
  ```bash
  cargo test -p <crate> <test_name>
  ```
- Check that no new warnings were introduced
- For structural changes, verify downstream crates still compile

### 4.3 Run single-test verification
```bash
cargo test -p <crate> <test_name>        # scoped
cargo test -p <crate> -- --nocapture     # with stdout
```

### 4.4 Check for clippy regressions
```bash
cargo clippy --all-targets -- -D warnings 2>&1
```
Expect **zero** warnings. `-D warnings` means any lint is an error.

### 4.5 Describe the change in jj
```bash
jj describe -m "crate: concise summary of changes

- Bullet list of specific changes
- Closes: #issue (if applicable)"
```

---

## Step 5 — Review & Iterate

### 5.1 Self-review checklist

| Check | Criterion |
|-------|-----------|
| GVSD compliance | Was GVSD followed for non-trivial changes? |
| CI clean | All 5 gates pass? |
| No dead code | Unused code removed or annotated with `#[expect]`? |
| No `RwLock` across `.await` | Verified in changed files? |
| Doc comments | Public items documented? |
| Visibility | `pub` on minimum needed surface? |
| Phase markers | Updated if a new phase was implemented? |
| Backward compat | Re-exports kept if code was moved? |
| Test coverage | Existing tests still pass? New tests for new logic? |
| No clippy suppressions | Prefer fixing over `#[allow]`? |

### 5.2 Iteration loop

If any verification step fails:
1. Diagnose the failure (is it a false positive in clippy, or real issue?)
2. Fix the root cause — never suppress lints without justification
3. Re-run from Step 4.1
4. Only suppress a lint with a documented `#[allow]` + comment when:
   - The lint is a known false positive (link to upstream issue)
   - The code is intentionally atypical for performance/safety reasons
   - The item is reserved for a planned phase (use `#[expect]` instead)

### 5.3 Final sign-off

When all gates pass and the self-review is clean, the change is ready.

---

## Decision Points & Branching

```
What type of change is this?
│
├── Lint fix (clippy warning/error)
│   ├── Can cargo fix handle it?
│   │   ├── Yes → Run cargo fix, verify
│   │   └── No  → Manual edit per common-lints.md, verify
│   └── Is it a false positive?
│       ├── Yes → #[allow] with comment + upstream issue link
│       └── No  → Fix it
│
├── Structural refactor (module split, extraction, re-org)
│   ├── GVSD required? → Yes, always
│   ├── Backward compat needed?
│   │   ├── Yes → Use re-export pattern
│   │   └── No  → Update all call sites
│   └── Test coverage adequate?
│       ├── Yes → Proceed
│       └── No  → Add tests first
│
├── TODO / Phase migration (implementing staged feature)
│   ├── Dead_code allow to remove?
│   │   ├── Yes → Remove #[expect]/#[allow], wire up
│   │   └── No  → Just update phase markers
│   └── Old path still needed as fallback?
│       ├── Yes → Keep both, add dispatch
│       └── No  → Remove old path
│
├── Bug fix
│   ├── GVSD required? → Yes
│   ├── Regression test added?
│   │   ├── Yes → Proceed
│   │   └── No  → Write test reproducing the bug first
│   └── Does it affect other crates?
│       ├── Yes → Full CI suite on workspace
│       └── No  → Scoped test
│
├── Dependency upgrade
│   ├── Semver breaking?
│   │   ├── Yes → Full audit of changed API surface
│   │   └── No  → Verify compiles, run tests
│   ├── Feature flags changed?
│   │   ├── Yes → Verify no loss of functionality
│   │   └── No  → OK
│   └── Run cargo update + cargo check
│
├── Unsafe audit
│   ├── New unsafe block?
│   │   ├── Yes → Safety comment required (SAFETY: ...)
│   │   └── No  → Verify existing safety invariants
│   ├── Miri check? (if available)
│   │   ├── Yes → Run cargo miri
│   │   └── No  → Document why not
│   └── SIMD intrinsics affected?
│       ├── Yes → Test scalar fallback alignment
│       └── No  → OK
│
└── Documentation / comments only
    ├── Public API doc?
    │   ├── Yes → /// doc comments on all public items
    │   └── No  → Internal // comments
    └── Phase markers updated?
        ├── Yes → Done
        └── No  → Update to reflect current state
```

---

## Quality Criteria

- **Zero** clippy warnings with `-D warnings`
- **Zero** compiler warnings (including `dead_code`, `unused_imports`)
- All CI gates pass on first attempt after fix
- No regression in test count or coverage
- GVSD document produced for non-trivial changes
- `jj change` described with meaningful message
- No `#[allow]` without a justifying comment
- No `RwLock` held across `.await` (deadlock prevention)
- Public API documented with `///` doc comments
- Phase comments updated to reflect current state

## References

- `AGENTS.md` — GVSD protocol, jj workflow, CI gates, conventions
- `ARCHITECTURE.md` — system architecture, pipeline layers, lock strategy
- `ci.sh` — CI gate script
- `Cargo.toml` — workspace config, dependency versions
- [GVSD Template](./references/gvsd-template.md) — Mandatory design template for non-trivial changes
- [Common Lints](./references/common-lints.md) — Clippy fix patterns for this codebase
