//! Command Tree — types, tree topology, and module wiring.

mod handlers;
pub mod parser;
mod provider;
mod runtime;

// Re-exports
pub use handlers::*;
pub use parser::{ParsedCommand, parse};

pub use runtime::CommandRuntime;

use crate::tui::effect::Effect;
use crate::tui::state::{AppState, CoreState, PopupMode};
use smallvec::SmallVec;
use std::borrow::Cow;

// ═══════════════════════════════════════════════════════════════
//  Node
// ═══════════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct Node {
    pub id: Cow<'static, str>,
    pub display: Cow<'static, str>,
    pub help: Cow<'static, str>,
    pub kind: NodeKind,
}

impl Node {
    pub fn display_id(&self) -> &str {
        self.id.as_ref()
    }
    pub fn display_text(&self) -> &str {
        self.display.as_ref()
    }
    pub fn help_text(&self) -> &str {
        self.help.as_ref()
    }
}

#[derive(Clone)]
pub enum NodeKind {
    Branch { provider: NodeProvider },
    Execute { handler: Handler },
}

pub type NodeProvider = fn(&CommandContext) -> Vec<Node>;
pub type Handler = fn(&CommandInvocation, &mut AppState) -> CommandResult;

// ═══════════════════════════════════════════════════════════════
//  CommandContext
// ═══════════════════════════════════════════════════════════════

pub struct CommandContext<'a> {
    pub path: &'a [PathEntry],
    pub core: &'a CoreState,
}

// ═══════════════════════════════════════════════════════════════
//  CommandInvocation
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct CommandInvocation {
    pub command_path: Vec<String>,
    pub args: Vec<String>,
    pub flags: std::collections::HashMap<String, String>,
}

impl CommandInvocation {
    pub fn new(command_path: Vec<String>, args: Vec<String>) -> Self {
        Self {
            command_path,
            args,
            flags: std::collections::HashMap::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  CommandResult
// ═══════════════════════════════════════════════════════════════

pub struct CommandResult {
    pub status: CommandStatus,
    pub effects: SmallVec<[UiEffect; 2]>,
}

impl CommandResult {
    pub fn handled() -> Self {
        Self {
            status: CommandStatus::Handled,
            effects: SmallVec::new(),
        }
    }
    pub fn error(_msg: impl Into<String>) -> Self {
        Self {
            status: CommandStatus::Error,
            effects: SmallVec::new(),
        }
    }
    pub fn with_effect(mut self, effect: UiEffect) -> Self {
        self.effects.push(effect);
        self
    }
}

#[derive(Clone, PartialEq)]
pub enum CommandStatus {
    Handled,
    Error,
}

pub enum UiEffect {
    ClosePalette,
    KeepPalette,
    OpenPopup(PopupMode),
    SetInput(String),
    PushEffect(Effect),
}

// ═══════════════════════════════════════════════════════════════
//  PathEntry / DisplayItem
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug, PartialEq)]
pub struct PathEntry {
    pub id: String,
}
pub type Path = Vec<PathEntry>;

// ═══════════════════════════════════════════════════════════════
//  PaletteLevel
// ═══════════════════════════════════════════════════════════════

pub enum PaletteLevel {
    Static(&'static [Node]),
    Dynamic(Vec<Node>),
}

impl Default for PaletteLevel {
    fn default() -> Self {
        PaletteLevel::Static(&[])
    }
}

impl PaletteLevel {
    pub fn list(&self) -> &[Node] {
        match self {
            PaletteLevel::Static(nodes) => nodes,
            PaletteLevel::Dynamic(nodes) => nodes.as_slice(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  CommandPalette state machine
// ═══════════════════════════════════════════════════════════════

#[derive(Default)]
pub struct CommandPalette {
    pub path: Path,
    pub level: PaletteLevel,
    pub selected: usize,
    pub filter: String,
}

impl CommandPalette {
    pub fn activate(&mut self) {
        self.path.clear();
        self.level = PaletteLevel::Static(ROOT);
        self.selected = 0;
        self.filter.clear();
    }
    pub fn current_nodes(&self) -> &[Node] {
        match &self.level {
            PaletteLevel::Static(nodes) => nodes,
            PaletteLevel::Dynamic(nodes) => nodes.as_slice(),
        }
    }
    pub fn filtered_items(&self) -> Vec<DisplayItem> {
        let nodes = self.current_nodes();
        let fl = self.filter.to_lowercase();
        nodes
            .iter()
            .filter(|n| {
                self.filter.is_empty()
                    || n.display_id().to_lowercase().contains(&fl)
                    || n.display_text().to_lowercase().contains(&fl)
            })
            .map(|n| DisplayItem {
                id: n.id.to_string(),
                display: n.display.to_string(),
                help: n.help.to_string(),
                has_children: matches!(n.kind, NodeKind::Branch { .. }),
            })
            .collect()
    }
    pub fn display_path(&self) -> String {
        if self.path.is_empty() {
            return "/".to_string();
        }
        let mut s = String::from("/");
        for (i, e) in self.path.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&e.id);
        }
        s
    }
}

pub struct DisplayItem {
    pub id: String,
    pub display: String,
    pub help: String,
    pub has_children: bool,
}

// ═══════════════════════════════════════════════════════════════
//  navigate_to
// ═══════════════════════════════════════════════════════════════

pub fn navigate_to(
    root: &'static [Node],
    path: &[PathEntry],
    ctx: &CommandContext,
) -> PaletteLevel {
    let mut level: PaletteLevel = PaletteLevel::Static(root);
    for entry in path {
        let nodes = level.list();
        let Some(node) = nodes.iter().find(|n| n.display_id() == entry.id) else {
            return PaletteLevel::Static(&[]);
        };
        match &node.kind {
            NodeKind::Branch { provider } => {
                level = PaletteLevel::Dynamic(provider(ctx));
            }
            NodeKind::Execute { .. } => {
                return PaletteLevel::Static(&[]);
            }
        }
    }
    level
}

// ═══════════════════════════════════════════════════════════════
//  node! macro
// ═══════════════════════════════════════════════════════════════

#[macro_export]
macro_rules! node {
    ($id:expr, $display:expr, $help:expr, Branch($provider:path)) => {
        $crate::tui::command_tree::Node {
            id: Cow::Borrowed($id),
            display: Cow::Borrowed($display),
            help: Cow::Borrowed($help),
            kind: $crate::tui::command_tree::NodeKind::Branch {
                provider: $provider,
            },
        }
    };
    ($id:expr, $display:expr, $help:expr, Execute($handler:path)) => {
        $crate::tui::command_tree::Node {
            id: Cow::Borrowed($id),
            display: Cow::Borrowed($display),
            help: Cow::Borrowed($help),
            kind: $crate::tui::command_tree::NodeKind::Execute { handler: $handler },
        }
    };
}

// ═══════════════════════════════════════════════════════════════
//  ROOT tree + static subtrees
// ═══════════════════════════════════════════════════════════════

/// Tree root — all commands begin here.
pub static ROOT: &[Node] = &[
    node!(
        "role",
        " role",
        "Role template management",
        Branch(provider::role_provider)
    ),
    node!("help", "󰛨 help", "Show help", Execute(handlers::help)),
    node!(
        "status",
        " status",
        "Show system status",
        Execute(handlers::status)
    ),
    node!(
        "clear",
        "󰩈 clear",
        "Clear conversation",
        Execute(handlers::clear)
    ),
    node!(
        "sh",
        "$ sh",
        "Run a shell command",
        Execute(handlers::shell)
    ),
    node!(
        "models",
        " models",
        "Open model picker",
        Execute(handlers::models)
    ),
    node!(
        "connect",
        "󱚤 connect",
        "Configure a provider",
        Execute(handlers::connect)
    ),
    node!(
        "pool",
        " pool",
        "Experience pool management",
        Branch(provider::pool_provider)
    ),
    node!(
        "sessions",
        " sessions",
        "Switch to a saved session",
        Branch(provider::sessions_provider)
    ),
    node!(
        "memo",
        "󰊄 memo",
        "Role memo management",
        Branch(provider::memo_provider)
    ),
    node!(
        "agent",
        " agent",
        "Agent management (list/inspect)",
        Branch(provider::agent_provider)
    ),
    node!(
        "reflect",
        " reflect",
        "Reflection control",
        Branch(provider::reflect_provider)
    ),
    node!(
        "refresh",
        "󰔄 refresh",
        "Clear system prompt cache",
        Execute(handlers::refresh)
    ),
    node!(
        "think",
        " think",
        "Set reasoning display level",
        Branch(provider::think_provider)
    ),
];

// ═══════════════════════════════════════════════════════════════
//  Coverage tooling
// ═══════════════════════════════════════════════════════════════

pub fn count_tree_commands() -> usize {
    fn count_nodes(nodes: &[Node]) -> usize {
        let mut n = 0;
        for node in nodes {
            match &node.kind {
                NodeKind::Execute { .. } => n += 1,
                NodeKind::Branch { provider } => {
                    let ctx = CommandContext {
                        path: &[],
                        core: &CoreState::default(),
                    };
                    n += count_nodes(&provider(&ctx));
                }
            }
        }
        n
    }
    count_nodes(ROOT)
}

pub fn legacy_command_names() -> Vec<&'static str> {
    let tree_ids: std::collections::HashSet<&str> = ROOT.iter().map(|n| n.display_id()).collect();
    crate::tui::commands::COMMANDS
        .iter()
        .map(|(cmd, _)| cmd.strip_prefix('/').unwrap_or(cmd))
        .filter(|name| !tree_ids.contains(name))
        .collect()
}

pub fn resolve_dynamic_items(parent: &str, core: &CoreState) -> Vec<(String, String)> {
    let provider: Option<NodeProvider> = match parent {
        "/role default" | "/role show" | "/role edit" | "/role delete" | "/role optimize" => {
            Some(provider::role_names_show_provider)
        }
        "/sessions switch" => Some(provider::sessions_provider),
        _ => None,
    };
    let Some(provider) = provider else {
        return vec![];
    };
    let ctx = CommandContext { path: &[], core };
    provider(&ctx)
        .into_iter()
        .map(|n| (n.id.to_string(), n.help.to_string()))
        .collect()
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_has_nodes() {
        assert!(!ROOT.is_empty());
        assert!(ROOT.iter().any(|n| n.display_id() == "role"));
        assert!(ROOT.iter().any(|n| n.display_id() == "help"));
    }

    #[test]
    fn test_filtered_items_empty_filter() {
        let mut palette = CommandPalette::default();
        palette.level = PaletteLevel::Static(ROOT);
        assert_eq!(palette.filtered_items().len(), ROOT.len());
    }

    #[test]
    fn test_filtered_items_with_filter() {
        let mut palette = CommandPalette::default();
        palette.level = PaletteLevel::Static(ROOT);
        palette.filter = "role".to_string();
        let items = palette.filtered_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "role");
    }

    #[test]
    fn test_filtered_items_no_match() {
        let mut palette = CommandPalette::default();
        palette.level = PaletteLevel::Static(ROOT);
        palette.filter = "zzznonexistent".to_string();
        assert!(palette.filtered_items().is_empty());
    }

    #[test]
    fn test_filtered_items_no_children_for_execute() {
        let mut palette = CommandPalette::default();
        palette.level = PaletteLevel::Static(ROOT);
        let help_item = palette
            .filtered_items()
            .into_iter()
            .find(|i| i.id == "help")
            .unwrap();
        assert!(!help_item.has_children);
    }

    #[test]
    fn test_navigate_to_empty_path_returns_root() {
        let ctx = CommandContext {
            path: &[],
            core: &CoreState::default(),
        };
        match navigate_to(ROOT, &[], &ctx) {
            PaletteLevel::Static(nodes) => assert!(std::ptr::eq(nodes, ROOT)),
            _ => panic!("expected Static(root)"),
        }
    }

    #[test]
    fn test_navigate_to_role_branch() {
        let ctx = CommandContext {
            path: &[],
            core: &CoreState::default(),
        };
        let path = vec![PathEntry {
            id: "role".to_string(),
        }];
        match navigate_to(ROOT, &path, &ctx) {
            PaletteLevel::Dynamic(nodes) => assert!(nodes.iter().any(|n| n.display_id() == "list")),
            _ => panic!("expected Dynamic for role branch"),
        }
    }

    #[cfg(test)]
    mod coverage_tests {
        use super::*;

        #[test]
        fn test_runtime_coverage() {
            let tree_count = count_tree_commands();
            let legacy: Vec<&str> = legacy_command_names();
            let total = tree_count + legacy.len();
            let pct = if total > 0 {
                (tree_count as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            eprintln!(
                "[coverage] tree={} legacy={} total={} coverage={:.0}%",
                tree_count,
                legacy.len(),
                total,
                pct
            );
            eprintln!("[coverage] legacy commands: {:?}", legacy);
            assert!(tree_count > 0);
        }
    }
}
