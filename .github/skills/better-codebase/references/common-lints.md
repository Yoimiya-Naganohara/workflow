# Common Clippy Lints in This Codebase

Reference patterns for lints that appear frequently in this workspace.

---

## `collapsible_if`

Nested `if` statements that can be combined with `&&` or `&& let`.

```rust
// ❌ Before
if let Some(node) = self.nodes.get_mut(&id) {
    if !node.dependencies.contains(&dep) {
        node.dependencies.push(dep);
    }
}

// ✅ After
if let Some(node) = self.nodes.get_mut(&id)
    && !node.dependencies.contains(&dep)
{
    node.dependencies.push(dep);
}
```

For deeper nesting, chain with `&& let`:

```rust
if child_done_result.is_some()
    && let Some(pid) = parent_id
    && let Some(parent) = self.nodes.get_mut(&pid)
    && !parent.completed_children.contains(&id)
{
    parent.completed_children.push(id);
}
```

---

## `items_after_test_module`

Any non-test item (impl blocks, `Default`, constants, etc.) placed **below** a `#[cfg(test)] mod tests` block.

```rust
// ❌ Before
#[cfg(test)]
mod tests {
    // ...
}

impl Default for ChatMessage {
    // ...
}

// ✅ After — move impl above the test module
impl Default for ChatMessage {
    // ...
}

#[cfg(test)]
mod tests {
    // ...
}
// ^ No items after this point
```

---

## `dead_code`

Unused functions, types, or methods.

| Situation | Action |
|-----------|--------|
| Truly unused, no future plan | Remove it |
| Reserved for Phase N+1 | `#[expect(dead_code)]` + doc comment explaining planned use |
| Conditionally compiled | Verify `#[cfg()]` gate scope |
| Public API surface | Check if callers exist elsewhere; if not, consider if pub is needed |

Prefer `#[expect]` over `#[allow]` — `#[expect]` will produce a warning if the lint **no longer applies**, alerting you to remove it.

```rust
/// Reserved for Phase 2 delegation routing.
#[expect(dead_code)]
fn can_reach_via_parent(&self, from: TaskId, to: TaskId) -> bool {
    // ...
}
```

---

## `unused_imports`

Often happens after refactoring or module extraction.

```bash
# Auto-fix
cargo fix --bin workflow --allow-dirty
```

---

## `module_inception`

Modules that re-export a module with the same name.

This project uses `#![allow(clippy::module_inception)]` at the crate level for runtime sub-modules (`runtime::runtime`). This is intentional — do not suppress locally.

---

## `empty_line_after_doc_comments`

Doc comments followed by a blank line before the documented item.

```rust
// ❌ Before
/// This is a doc comment.

fn foo() {}

// ✅ After
/// This is a doc comment.
fn foo() {}
```

Currently allowed in `orchestration.rs` — fix if encountered in other files.

---

## `needless_pass_by_value`

Functions taking `Vec`, `String`, or other owned types when a reference suffices.

```rust
// ❌ Before
fn process(items: Vec<Item>) { ... }

// ✅ After
fn process(items: &[Item]) { ... }
```

Be careful with this lint — if the function needs ownership (e.g., to store in a struct), keep the owned parameter but add `#[allow(clippy::needless_pass_by_value)]` with a comment.
