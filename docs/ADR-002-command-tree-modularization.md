# ADR-002: Command Tree Modularization

## Status

Accepted (2026-06-21)

## Context

Phase 2c completed the migration of all commands from `commands.rs` (legacy dispatch) into the command tree. As a result, `command_tree.rs` grew from ~940 lines (Phase 2a) to ~2087 lines (Phase 2c completion).

The file is now a monolith carrying 6 distinct responsibilities:

| Responsibility | Lines | Evidence |
|---------------|-------|----------|
| Type definitions | ~350 | Node, NodeKind, Provider, Handler, CommandInvocation |
| Parser | ~60 | ParsedCommand, parse, parse_tokens |
| Runtime | ~200 | CommandRuntime::execute, resolve, apply_effect |
| Providers | ~250 | role, think, pool, sessions, memo, agent, reflect providers |
| Handlers | ~900 | All handler implementations (help, status, clear, pool, memo, agent, reflect, etc.) |
| Coverage tooling | ~80 | count_tree_commands, legacy_command_names, resolve_dynamic_items |

The 2000-line threshold has been crossed, and — more importantly — responsibilities are starting to overlap. For example, both provider logic and handler logic live in the same file but serve different layers of the architecture (provider = tree projection, handler = command execution).

## Decision

Split `command_tree.rs` into a `command_tree/` directory with 6 files:

```
src/tui/command_tree/
├── mod.rs          # Re-exports + core types (Node, NodeKind, CommandInvocation, Handler, etc.)
│                     Also: CommandPalette, PaletteLevel, CommandContext, PathEntry, DisplayItem
│                     Also: PaletteAction → removed (no longer used)
├── parser.rs       # ParsedCommand, parse(), parse_tokens()
├── runtime.rs      # CommandRuntime (execute, resolve, apply_effect)
├── provider.rs     # All NodeProvider functions (static subtrees + dynamic providers)
├── handlers.rs     # All handler implementations
└── coverage.rs     # count_tree_commands(), legacy_command_names(), resolve_dynamic_items()
```

## Module boundaries

### `mod.rs` — Public API and types

Exports: `Node`, `NodeKind`, `CommandInvocation`, `CommandResult`, `CommandStatus`, `UiEffect`, `Handler`, `NodeProvider`, `CommandContext`, `PathEntry`, `PaletteLevel`, `CommandPalette`, `DisplayItem`, `CommandRuntime`, `ParsedCommand`, `parse`, `count_tree_commands`, `resolve_dynamic_items`.

No implementation other than type definitions, the `node!` macro, and the `ROOT` tree + all static subtrees.

### `parser.rs` — Input tokenization

Exports: `ParsedCommand`, `parse()`.

Single responsibility: convert CLI input string to tokenized form. No knowledge of the tree.

### `runtime.rs` — Execution engine

Exports: `CommandRuntime`.

Contains `execute()`, `resolve()`, `apply_effect()`. No knowledge of specific commands or providers.

### `provider.rs` — Dynamic tree structure

Contains all `NodeProvider` implementations. Each provider maps a data source (static, runtime, persistence) to a `Vec<Node>`.

This is the most likely file to grow as new data sources are added.

### `handlers.rs` — Command business logic

Contains all handler functions. Each handler is `fn(&CommandInvocation, &mut AppState) -> CommandResult`.

This is the largest file (~900 lines). If it exceeds 1000 LOC in the future, consider splitting into `handlers/role.rs`, `handlers/pool.rs`, etc.

### `coverage.rs` — Metrics and backwards compatibility

Exports: `count_tree_commands()`, `legacy_command_names()`, `resolve_dynamic_items()`.

`resolve_dynamic_items()` bridges the old `PopupMode::SubCommand` rendering system to tree-based providers. It is kept for backward compatibility and will be removed when the old popup system is fully decommissioned.

## Migration method

1. Create `src/tui/command_tree/` directory
2. Create all 6 files
3. Move the corresponding code sections from the monolithic `command_tree.rs`
4. Remove the old file
5. Add `pub mod command_tree;` in `tui/mod.rs` (already exists — just verify it references the directory)

No behavior changes. Pure structural split.

## Consequences

### Positive

- Each file has a single responsibility
- Maximum file size drops from ~2087 to ~900 lines (handlers.rs)
- New providers and handlers can be added without touching unrelated code
- The `ROOT` tree + static subtrees live in `mod.rs`, making the command topology visible at a glance

### Negative

- Increased number of files (6 vs 1)
- Some intra-package visibility adjustments needed (private functions that were in the same file now need `pub(crate)`)

### Future

If handlers.rs exceeds 1000 LOC, split into per-domain files:
```
handlers/
├── mod.rs
├── help_status.rs
├── role.rs
├── pool.rs
├── memo.rs
├── agent.rs
└── reflect.rs
```
