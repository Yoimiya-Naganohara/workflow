//! Command Tree — 基于树导航的命令面板状态机。
//!
//! 不再是字符串 dispatch，而是：
//!
//! ```text
//! Node → Node → Node → Execute(handler)
//! ```
//!
//! 静态和动态节点共用同一 [`Node`] 类型，区别仅在于谁生产它。
//! 静态节点由 [`ROOT`] 等 `&'static [Node]` 定义；
//! 动态节点由 [`NodeProvider`] 在运行时生成。
//!
//! 旧 `dispatch()` 保留了字符串路径兼容层 [`execute_path`]。

use std::borrow::Cow;

use crate::tui::state::{AppState, CoreState};

// ═══════════════════════════════════════════════════════════════
//  Core types
// ═══════════════════════════════════════════════════════════════

/// 一个命令树节点。静态和动态共用同一类型。
#[derive(Clone)]
pub struct Node {
    /// 路由标识（如 `"default"`、`"claude-sonnet-4"`）
    pub id: Cow<'static, str>,
    /// 弹窗显示文本（如 `" role"`、`"🤖 Assistant Role"`）
    pub display: Cow<'static, str>,
    /// 帮助文本
    pub help: Cow<'static, str>,
    /// 节点类型
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
    /// 子节点列表由 provider 在导航时生成
    Branch { provider: NodeProvider },
    /// 叶子节点 — 最终执行
    Execute { handler: Handler },
}

/// 运行时节点提供者：根据上下文生成子节点列表。
pub type NodeProvider = fn(&CommandContext) -> Vec<Node>;

/// 执行器：收到完整路径后执行操作。
/// 不传 `CommandContext` 以避免与 `&mut AppState` 的借用冲突。
/// handler 内部通过 `state.core` 直接访问所需数据。
pub type Handler = fn(path: &[PathEntry], state: &mut AppState) -> bool;

/// 提供给 [`NodeProvider`] 和 [`Handler`] 的只读上下文。
pub struct CommandContext<'a> {
    /// 从根到当前节点的完整路径（不含当前节点自身）
    pub path: &'a [PathEntry],
    /// AppState 的核心状态（只读）
    pub core: &'a CoreState,
}

/// 路径中的一段。存 ID，不存 display（display 只用于渲染）。
#[derive(Clone, Debug, PartialEq)]
pub struct PathEntry {
    pub id: String,
}

/// 路径 = 一串 ID，从根到当前节点
pub type Path = Vec<PathEntry>;

/// 弹窗渲染用的显示项（过滤后的产物）。
pub struct DisplayItem {
    pub id: String,
    pub display: String,
    pub help: String,
    pub has_children: bool,
}

// ═══════════════════════════════════════════════════════════════
//  Palette state machine
// ═══════════════════════════════════════════════════════════════

/// 命令面板状态机。
#[derive(Default)]
pub struct CommandPalette {
    /// 当前导航路径。例如 `["role", "default"]`
    pub path: Path,
    /// 当前层级的节点列表（静态引用或动态缓存）
    pub level: PaletteLevel,
    /// 当前选中的索引（在过滤后的列表中）
    pub selected: usize,
    /// 用户输入的过滤文本
    pub filter: String,
}

impl CommandPalette {
    /// 激活：重置到树根。
    pub fn activate(&mut self) {
        self.path.clear();
        self.level = PaletteLevel::Static(ROOT);
        self.selected = 0;
        self.filter.clear();
    }

    /// 当前层级的原始节点列表。
    pub fn current_nodes(&self) -> &[Node] {
        match &self.level {
            PaletteLevel::Static(nodes) => nodes,
            PaletteLevel::Dynamic(nodes) => nodes.as_slice(),
        }
    }

    /// 当前层级的可显示项（应用 filter 后）。
    pub fn filtered_items(&self) -> Vec<DisplayItem> {
        let nodes = self.current_nodes();
        if self.filter.is_empty() {
            return nodes
                .iter()
                .map(|n| DisplayItem {
                    id: n.id.to_string(),
                    display: n.display.to_string(),
                    help: n.help.to_string(),
                    has_children: matches!(n.kind, NodeKind::Branch { .. }),
                })
                .collect();
        }

        let fl = self.filter.to_lowercase();
        nodes
            .iter()
            .filter(|n| {
                n.display_id().to_lowercase().contains(&fl)
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

    /// 生成当前路径的可读字符串，用于输入框显示。
    pub fn display_path(&self) -> String {
        if self.path.is_empty() {
            return "/".to_string();
        }
        let mut s = String::from("/");
        for (i, entry) in self.path.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&entry.id);
        }
        s
    }
}

/// 内部辅助：Enter 时从节点提取的操作信息。
pub enum PaletteAction {
    Branch(NodeProvider),
    Execute(Handler),
}

/// 树层级：静态引用或动态缓存。
pub enum PaletteLevel {
    Static(&'static [Node]),
    Dynamic(Vec<Node>),
}

impl Default for PaletteLevel {
    fn default() -> Self {
        PaletteLevel::Static(&[])
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tree navigation
// ═══════════════════════════════════════════════════════════════

/// 根据路径从根导航，返回目标层级的节点列表。
///
/// 用于回溯（Backspace）时重建层级。
pub fn navigate_to(
    root: &'static [Node],
    path: &[PathEntry],
    ctx: &CommandContext,
) -> PaletteLevel {
    let mut level: PaletteLevel = PaletteLevel::Static(root);

    for entry in path {
        let nodes = match &level {
            PaletteLevel::Static(nodes) => *nodes,
            PaletteLevel::Dynamic(nodes) => nodes.as_slice(),
        };

        let Some(node) = nodes.iter().find(|n| n.display_id() == entry.id) else {
            return PaletteLevel::Static(&[]);
        };

        match &node.kind {
            NodeKind::Branch { provider } => {
                let children = provider(ctx);
                level = PaletteLevel::Dynamic(children);
            }
            NodeKind::Execute { .. } => {
                // 不应深入执行节点
                return PaletteLevel::Static(&[]);
            }
        }
    }

    level
}

// ═══════════════════════════════════════════════════════════════
//  Compatibility: execute from string path
// ═══════════════════════════════════════════════════════════════

/// 从字符串路径执行命令。CLI（`/role show xxx`）和 Palette 都收敛至此。
///
/// `path` 是已分割的路径段，如 `["role", "show", "role_name"]`。
///
/// Phase 1 中 fallback 到旧 `dispatch()`。后续迁移完成后，
/// 此函数直接遍历树找到 `Execute` 节点并调用 handler。
pub fn execute_path(path: &[&str], state: &mut AppState, now: &str) -> bool {
    let full_cmd = format!("/{}", path.join(" "));
    crate::tui::commands::dispatch(&full_cmd, state, now)
}

// ═══════════════════════════════════════════════════════════════
//  Helper macros
// ═══════════════════════════════════════════════════════════════

/// 构造一个静态 [`Node`]。
macro_rules! node {
    ($id:expr, $display:expr, $help:expr, Branch($provider:path)) => {
        crate::tui::command_tree::Node {
            id: Cow::Borrowed($id),
            display: Cow::Borrowed($display),
            help: Cow::Borrowed($help),
            kind: crate::tui::command_tree::NodeKind::Branch {
                provider: $provider,
            },
        }
    };
    ($id:expr, $display:expr, $help:expr, Execute($handler:path)) => {
        crate::tui::command_tree::Node {
            id: Cow::Borrowed($id),
            display: Cow::Borrowed($display),
            help: Cow::Borrowed($help),
            kind: crate::tui::command_tree::NodeKind::Execute { handler: $handler },
        }
    };
}

// ═══════════════════════════════════════════════════════════════
//  ROOT tree
// ═══════════════════════════════════════════════════════════════

/// 树根。所有命令的入口。
///
/// Phase 1 只包含少量节点用于验证导航。
/// 后续逐步添加 `/memo`、`/pool`、`/reflect`、`/agent`、`/sessions` 等。
pub static ROOT: &[Node] = &[
    node!(
        "role",
        " role",
        "Role template management",
        Branch(role_provider)
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
];

/// Role 子命令（静态）
static ROLE_NODES: &[Node] = &[
    node!(
        "list",
        "list",
        "List all role templates",
        Execute(handlers::role_list)
    ),
    node!(
        "show",
        "show",
        "Show role template detail",
        Branch(role_names_provider)
    ),
    node!(
        "default",
        "default",
        "Set default bootstrap role",
        Branch(role_names_provider)
    ),
    node!(
        "create",
        "create",
        "Create a new role template",
        Execute(handlers::role_create)
    ),
    node!(
        "embed",
        "embed",
        "Compute embeddings for all roles",
        Execute(handlers::role_embed)
    ),
    node!(
        "delete",
        "delete",
        "Delete a role template",
        Branch(role_names_provider)
    ),
];

/// Provider: 返回 ROLE 静态子树
fn role_provider(_ctx: &CommandContext) -> Vec<Node> {
    ROLE_NODES.to_vec()
}

/// Provider: 运行时加载角色名称列表
fn role_names_provider(ctx: &CommandContext) -> Vec<Node> {
    let Some(runtime) = ctx.core.runtime.as_ref() else {
        return vec![];
    };
    let Ok(guard) = runtime.try_read() else {
        return vec![];
    };
    let templates = guard.all_role_templates();
    if templates.is_empty() {
        return vec![];
    }
    let current_default = &ctx.core.default_role;
    templates
        .iter()
        .map(|t| {
            let label = if t.role == *current_default {
                format!("{} (current)", t.label)
            } else {
                t.label.clone()
            };
            Node {
                id: Cow::Owned(t.role.clone()),
                display: Cow::Owned(label),
                help: Cow::Owned(format!("Role #{}", t.template_id)),
                kind: NodeKind::Execute {
                    handler: handlers::role_action,
                },
            }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════
//  Handlers (Phase 1 stubs)
// ═══════════════════════════════════════════════════════════════
//
//  Phase 1 的 handler 只做验证性操作（打印 + 推消息）。
//  Phase 2 逐步替换为真正的业务逻辑。

mod handlers {
    use super::{AppState, PathEntry};
    use crate::tui::state::{ChatMessage, MessageRole};

    pub fn help(_path: &[PathEntry], state: &mut AppState) -> bool {
        let help_text = crate::tui::commands::COMMANDS
            .iter()
            .map(|(cmd, desc)| format!("  {:20} {}", cmd, desc))
            .collect::<Vec<_>>()
            .join("\n");
        state.core.messages.push(ChatMessage::system(help_text));
        true
    }

    pub fn status(_path: &[PathEntry], state: &mut AppState) -> bool {
        let core = &state.core;
        let provider_count = core.configured_providers.len();
        let model_count = core.selected_models.len();
        let agent_count = core.agents.len();
        let reflection = if core.reflection.auto_reflect {
            "🟢 on"
        } else {
            "🔴 off"
        };
        let lines = [
            format!("Providers:       {}", provider_count),
            format!("Selected models: {}", model_count),
            format!("Active agents:   {}", agent_count),
            format!("Default role:    {}", core.default_role),
            format!("Reflection:      {}", reflection),
            format!(
                "Messages:        {}",
                core.messages
                    .iter()
                    .filter(|m| m.role == MessageRole::User)
                    .count()
            ),
        ];
        state.core.messages.push(ChatMessage::system(format!(
            "System Status:\n{}",
            lines.join("\n")
        )));
        true
    }

    pub fn clear(_path: &[PathEntry], state: &mut AppState) -> bool {
        let core = &mut state.core;
        let ui = &mut state.ui;
        core.messages.clear();
        core.messages.push(ChatMessage::system(
            "Workflow Agent — conversation cleared. Describe your goal and I'll help.",
        ));
        core.responsible_agent_id = None;
        core.agents.clear();
        ui.input.clear();
        ui.input_cursor = 0;
        ui.chat_scroll = 0;
        ui.input_history.clear();
        ui.input_history_idx = None;
        ui.active_chat_abort = None;
        ui.active_chat_requests = 0;
        ui.cached_system_prompt = None;
        ui.cached_prompt_role.clear();
        true
    }

    pub fn shell(_path: &[PathEntry], state: &mut AppState) -> bool {
        // 打开子命令输入弹窗 — 使用旧的 ShellInput 弹窗
        state.popup_mode = crate::tui::state::PopupMode::ShellInput {
            cmd: "/sh".to_string(),
            input: String::new(),
        };
        state.popup_selected = 0;
        true
    }

    pub fn models(_path: &[PathEntry], state: &mut AppState) -> bool {
        if state.core.configured_providers.is_empty() {
            state.core.messages.push(ChatMessage::system(
                "No providers configured. Use `/connect` first.",
            ));
        } else {
            state.popup_mode = crate::tui::state::PopupMode::ModelPicker;
            state.popup_selected = 0;
            state
                .core
                .messages
                .push(ChatMessage::system("Select models to add to your pool"));
        }
        true
    }

    pub fn role_list(_path: &[PathEntry], state: &mut AppState) -> bool {
        // Extract data before any mutable access
        let msg = (|| -> Option<String> {
            let runtime = state.core.runtime.as_ref()?;
            let guard = runtime.try_read().ok()?;
            let templates = guard.all_role_templates();
            if templates.is_empty() {
                return Some("No role templates found.".to_string());
            }
            let mut lines = vec!["Role Templates:".to_string()];
            for t in &templates {
                let embedded = if t.embedding.is_some() { "✓" } else { "✗" };
                lines.push(format!(
                    "  id={:<3}  {:<30}  label={:<20}  embedded={}",
                    t.template_id, t.role, t.label, embedded
                ));
            }
            Some(lines.join("\n"))
        })();

        state
            .core
            .messages
            .push(ChatMessage::system(msg.unwrap_or_else(|| {
                if state.core.runtime.is_some() {
                    "Runtime locked".to_string()
                } else {
                    "Runtime not available".to_string()
                }
            })));
        true
    }

    pub fn role_create(_path: &[PathEntry], state: &mut AppState) -> bool {
        state.core.messages.push(ChatMessage::system(
            "Role creation — edit role templates in ~/.workflow/role_templates.json",
        ));
        true
    }

    pub fn role_embed(_path: &[PathEntry], state: &mut AppState) -> bool {
        let n = (|| -> Option<usize> {
            let runtime = state.core.runtime.as_ref()?;
            let guard = runtime.try_read().ok()?;
            let n = guard.all_role_templates().len();
            guard.compute_role_embeddings_async();
            Some(n)
        })();

        match n {
            Some(count) => {
                state.core.messages.push(ChatMessage::system(format!(
                    "Computing embeddings for {} role template(s)...",
                    count
                )));
            }
            None => {
                state
                    .core
                    .messages
                    .push(ChatMessage::system(if state.core.runtime.is_some() {
                        "Runtime locked"
                    } else {
                        "Runtime not available"
                    }));
            }
        }
        true
    }

    /// 通用角色动作。路径最后一段是角色 ID。
    /// 由 `role_names_provider` 生成的所有 Execute 节点调用。
    ///
    /// 根据父节点区分操作（show / default / delete）：
    /// - `role show xxx` → 显示角色详情
    /// - `role default xxx` → 设为默认
    /// - `role delete xxx` → 删除角色
    pub fn role_action(path: &[PathEntry], state: &mut AppState) -> bool {
        let role_id = path.last().map(|e| e.id.as_str()).unwrap_or("?");
        let action = path
            .get(path.len().wrapping_sub(2))
            .map(|e| e.id.as_str())
            .unwrap_or("?");

        match action {
            "show" => {
                // Extract data first, then mutate
                let msg = (|| -> Option<String> {
                    let runtime = state.core.runtime.as_ref()?;
                    let guard = runtime.try_read().ok()?;
                    let t = guard.get_role_template(role_id)?;
                    let embedded = if t.embedding.is_some() { "yes" } else { "no" };
                    Some(format!(
                        "Role: {}\n  Label:        {}\n  ID:           {}\n  Embedded:     {}\n  Prompt ({}):\n{}\n{}\n{}",
                        t.role,
                        t.label,
                        t.template_id,
                        embedded,
                        t.system_prompt.len(),
                        "─".repeat(36),
                        t.system_prompt,
                        "─".repeat(36)
                    ))
                })();

                let msg = msg.unwrap_or_else(|| {
                    let has_runtime = state.core.runtime.is_some();
                    if !has_runtime {
                        "Runtime not available".to_string()
                    } else {
                        // Try to distinguish locked vs not-found
                        if let Some(rt) = state.core.runtime.as_ref() {
                            if rt.try_read().is_err() {
                                "Runtime locked".to_string()
                            } else {
                                format!("Role '{}' not found.", role_id)
                            }
                        } else {
                            "Runtime not available".to_string()
                        }
                    }
                });
                state.core.messages.push(ChatMessage::system(msg));
            }
            "default" => {
                state.core.default_role = role_id.to_string();
                state.core.responsible_agent_id = None;
                state.core.agents.clear();
                state.core.messages.push(ChatMessage::system(format!(
                    "Default bootstrap role set to `{}`. Next chat message will use this role.",
                    role_id
                )));
            }
            "delete" => {
                let result = (|| -> Result<(), String> {
                    let runtime = state
                        .core
                        .runtime
                        .as_ref()
                        .ok_or_else(|| "Runtime not available".to_string())?;
                    let guard = runtime
                        .try_read()
                        .map_err(|_| "Runtime locked".to_string())?;
                    let t = guard
                        .get_role_template(role_id)
                        .ok_or_else(|| format!("Role '{}' not found.", role_id))?;
                    guard.delete_role_template(t.template_id);
                    Ok(())
                })();

                match result {
                    Ok(()) => {
                        state
                            .core
                            .messages
                            .push(ChatMessage::system(format!("Role '{}' deleted.", role_id)));
                    }
                    Err(e) => {
                        state.core.messages.push(ChatMessage::system(e));
                    }
                }
            }
            _ => {
                state.core.messages.push(ChatMessage::system(format!(
                    "Unknown role action '{}' for role '{}'",
                    action, role_id
                )));
            }
        }
        true
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Node / DisplayItem ──

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
        let items = palette.filtered_items();
        assert_eq!(items.len(), ROOT.len());
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
        let items = palette.filtered_items();
        assert!(items.is_empty());
    }

    #[test]
    fn test_display_path_empty() {
        let palette = CommandPalette::default();
        assert_eq!(palette.display_path(), "/");
    }

    #[test]
    fn test_display_path_with_segments() {
        let mut palette = CommandPalette::default();
        palette.path.push(PathEntry {
            id: "role".to_string(),
        });
        palette.path.push(PathEntry {
            id: "default".to_string(),
        });
        assert_eq!(palette.display_path(), "/role default");
    }

    // ── navigate_to ──

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
            PaletteLevel::Dynamic(nodes) => {
                assert!(nodes.iter().any(|n| n.display_id() == "list"));
                assert!(nodes.iter().any(|n| n.display_id() == "show"));
                assert!(nodes.iter().any(|n| n.display_id() == "default"));
            }
            _ => panic!("expected Dynamic for role branch"),
        }
    }

    #[test]
    fn test_navigate_to_execute_returns_empty() {
        let ctx = CommandContext {
            path: &[],
            core: &CoreState::default(),
        };
        let path = vec![PathEntry {
            id: "help".to_string(),
        }];
        match navigate_to(ROOT, &path, &ctx) {
            PaletteLevel::Static(nodes) => assert!(nodes.is_empty()),
            PaletteLevel::Dynamic(nodes) => assert!(nodes.is_empty()),
        }
    }

    #[test]
    fn test_navigate_to_nonexistent_returns_empty() {
        let ctx = CommandContext {
            path: &[],
            core: &CoreState::default(),
        };
        let path = vec![PathEntry {
            id: "nonexistent".to_string(),
        }];
        match navigate_to(ROOT, &path, &ctx) {
            PaletteLevel::Static(nodes) => assert!(nodes.is_empty()),
            PaletteLevel::Dynamic(nodes) => assert!(nodes.is_empty()),
        }
    }

    // ── DisplayItem fields ──

    #[test]
    fn test_display_item_has_children_for_branch() {
        let mut palette = CommandPalette::default();
        palette.level = PaletteLevel::Static(ROOT);
        let role_item = palette
            .filtered_items()
            .into_iter()
            .find(|i| i.id == "role")
            .unwrap();
        assert!(role_item.has_children, "role should have children");
    }

    #[test]
    fn test_display_item_no_children_for_execute() {
        let mut palette = CommandPalette::default();
        palette.level = PaletteLevel::Static(ROOT);
        let help_item = palette
            .filtered_items()
            .into_iter()
            .find(|i| i.id == "help")
            .unwrap();
        assert!(!help_item.has_children, "help should not have children");
    }
}
