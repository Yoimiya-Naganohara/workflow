//! Node trait and CommandStack for the command tree.

// ═══════════════════════════════════════════════════════════════
//  Node trait
// ═══════════════════════════════════════════════════════════════

/// Command tree node. All commands implement this trait.
pub trait Node: Send + Sync {
    /// Display name (e.g. "show", "default").
    fn name(&self) -> &str;

    /// One-line description.
    fn desc(&self) -> &str;

    /// Get child nodes (branch nodes only).
    fn children(&self) -> Vec<Box<dyn Node>> {
        vec![]
    }

    /// Execute the command (leaf nodes only).
    fn execute(
        &self,
        _args: &[String],
        _state: &mut crate::tui::state::AppState,
        _now: &str,
    ) -> bool {
        false
    }
}

// ═══════════════════════════════════════════════════════════════
//  CommandStack
// ═══════════════════════════════════════════════════════════════

/// Navigation stack for the command tree.
pub struct CommandStack {
    stack: Vec<Vec<Box<dyn Node>>>,
    selected: Vec<usize>,
}

impl CommandStack {
    pub fn new() -> Self {
        let root = super::RootNode;
        let children = root.children();
        Self {
            stack: vec![children],
            selected: vec![0],
        }
    }

    /// Current layer.
    pub fn top(&self) -> &[Box<dyn Node>] {
        self.stack.last().map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Currently selected node.
    pub fn current(&self) -> Option<&dyn Node> {
        let idx = self.selected.last().copied().unwrap_or(0);
        self.top().get(idx).map(|n| n.as_ref())
    }

    /// Execute the current node (leaf command).
    pub fn execute_current(
        &mut self,
        args: &[String],
        state: &mut crate::tui::state::AppState,
        now: &str,
    ) -> bool {
        let idx = self.selected.last().copied().unwrap_or(0);
        if let Some(node) = self.top().get(idx) {
            node.execute(args, state, now)
        } else {
            false
        }
    }

    /// Get children of the current node (branch command).
    pub fn current_children(&self) -> Vec<Box<dyn Node>> {
        let idx = self.selected.last().copied().unwrap_or(0);
        if let Some(node) = self.top().get(idx) {
            node.children()
        } else {
            vec![]
        }
    }

    /// Push children onto the stack (enter sub-layer).
    pub fn push(&mut self, children: Vec<Box<dyn Node>>) {
        self.stack.push(children);
        self.selected.push(0);
    }

    /// Pop one layer (go back).
    pub fn pop(&mut self) -> bool {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.selected.pop();
            true
        } else {
            false
        }
    }

    /// Current layer depth.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Select next item.
    pub fn next(&mut self) {
        let len = self.top().len();
        if len > 0 {
            if let Some(idx) = self.selected.last_mut() {
                *idx = (*idx + 1) % len;
            }
        }
    }

    /// Select previous item.
    pub fn prev(&mut self) {
        let len = self.top().len();
        if len > 0 {
            if let Some(idx) = self.selected.last_mut() {
                *idx = (*idx + len - 1) % len;
            }
        }
    }

    /// Get the currently selected index.
    pub fn selected_index(&self) -> usize {
        self.selected.last().copied().unwrap_or(0)
    }

    /// Filter current layer by query.
    pub fn filter(&self, query: &str) -> Vec<(usize, &dyn Node)> {
        self.top()
            .iter()
            .enumerate()
            .filter(|(_, node)| node.name().contains(query) || node.desc().contains(query))
            .map(|(i, node)| (i, node.as_ref()))
            .collect()
    }
}

impl Default for CommandStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stack_push_pop() {
        let mut stack = CommandStack::new();
        assert_eq!(stack.depth(), 1);

        // Find the RoleGroup node
        let top = stack.top();
        let role_idx = top.iter().position(|n| n.name() == "role").unwrap();
        *stack.selected.last_mut().unwrap() = role_idx;

        // Get children of RoleGroup
        let root_children = stack.current_children();
        assert!(!root_children.is_empty());
        stack.push(root_children);
        assert_eq!(stack.depth(), 2);

        assert!(stack.pop());
        assert_eq!(stack.depth(), 1);

        assert!(!stack.pop()); // can't pop root
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn test_stack_next_prev() {
        let mut stack = CommandStack::new();
        assert_eq!(stack.selected_index(), 0);

        stack.next();
        assert_eq!(stack.selected_index(), 1);

        stack.prev();
        assert_eq!(stack.selected_index(), 0);
    }

    #[test]
    fn test_stack_filter() {
        let stack = CommandStack::new();
        let matches = stack.filter("role");
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|(_, n)| n.name() == "role"));
    }
}
