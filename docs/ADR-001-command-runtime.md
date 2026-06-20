# ADR-001: Command Runtime Architecture

## Status

Accepted (2026-06-20), updated (2026-06-21) with Phase 2c completion and architectural conclusions.

## Context

Phase 1 introduced a tree-based command navigation model (`command_tree.rs`) alongside the existing string dispatch (`commands.rs`). The tree model proved viable — it handles static branches, dynamic node providers, and palette-based navigation.

However, Phase 1 left three systems coexisting without clear boundaries:

| System | Role | Problem |
|--------|------|---------|
| `dispatch()` | String → bool | Mixed parsing + routing + execution |
| `command_tree` | Node / NodeKind / Palette | No formal execution layer |
| Handler | `fn(&[PathEntry], &mut AppState) -> bool` | Weak ABI (bool, path parsing leak) |

The key architectural insight from Phase 1 is:

> CLI (`/role show xxx`) and Palette (`role → show → xxx`) are two input modalities that should converge into **one execution engine**, not just one entry function.

## Decision

### 1. CommandInvocation — The universal input unit

```rust
pub struct CommandInvocation {
    pub command_path: Vec<String>,
    pub args: Vec<String>,
    pub flags: HashMap<String, String>,
}
```

### 2. Handler — The execution unit

```rust
pub type Handler = fn(&CommandInvocation, &mut AppState) -> CommandResult;
```

### 3. CommandResult — The execution outcome

```rust
pub struct CommandResult {
    pub status: CommandStatus,
    pub effects: SmallVec<[UiEffect; 2]>,
}
```

### 4. CommandRuntime — The execution engine

```rust
pub struct CommandRuntime;
impl CommandRuntime {
    pub fn execute(&self, parsed: &ParsedCommand, state: &mut AppState) -> CommandResult;
}
```

## Architecture (Phase 2c conclusion)

After Phase 2c (Legacy Extraction), the system has stabilized into four layers:

```
[1] Provider Layer
    Determines tree structure at navigation time.
    Sources: static data, runtime state, persistence IO.
    Contract: fn(&CommandContext) -> Vec<Node>
    Independence: Provider does NOT require AppState — only CommandContext (read-only core).

[2] Tree Layer
    Declarative command structure: Node / Branch / Execute.
    No business logic — pure structure.
    Both static (ROOT, POOL_NODES) and dynamic (sessions_provider) nodes use the same Node type.

[3] Runtime Layer
    resolve(): walks tree given parsed tokens, separates command_path from args.
    execute(): calls handler, applies UiEffect.
    No business logic — pure orchestration.

[4] Handler Layer
    Business logic for each command.
    Receives CommandInvocation (command_path + args), never the tree itself.
```

### Key architectural conclusion: Command Tree = Projection Layer

The command tree is NOT a command registry. It is a projection layer — a dynamic, runtime-generated view of available commands.

Evidence:
- `sessions_provider` reads from persistence, not from runtime state
- `role_names_provider` reads from runtime role templates
- `pool_provider` returns a fixed static subtree
- All three use the same `Node` type and `NodeProvider` signature

This means: the command tree is data-source agnostic. It projects whatever structure its providers return, regardless of origin (static code, in-memory state, file system).

### Provider independence (critical)

```
Old model:    Provider = Runtime subsystem
New model:    Provider = Independent data-source → Node mapper
```

Consequence: `CommandContext` (the provider's read-only input) is intentionally minimal — only `&[PathEntry]` and `&CoreState`. Providers that don't need runtime state (like sessions_provider) simply ignore it.

## Consequences

### Positive

1. **ABI stability**: Handler signature is frozen.
2. **Input modality independence**: CLI, Palette, Agent, Macro all converge through runtime.
3. **Provider independence**: Tree is a projection layer, not a registry.
4. **Phase 2b migration**: All execution paths now converge to `CommandRuntime::execute()`.
5. **Coverage progress**: Phase 2c reached 83% coverage (19 tree-backed commands, 4 legacy).

### Negative

1. Command Tree Modularization is approaching threshold (~1500 LOC). See ADR-002.

## Migration plan (Phase 2c remaining)

```
Batch 4: memo      — dynamic key tree + parameterized operations
Batch 5: connect   — static config tree (verified pattern)
Batch 6: agent     — complex state, list/inspect
Batch 7: reflect   — workflow composition, rule toggles
```

Termination condition: `legacy_command_names()` returns empty → delete `legacy_dispatch()`.

## Phase 3 (future)

- Aliases and macros via expanded resolve() layer
- RuntimeEffect split from UiEffect
- Async node providers
