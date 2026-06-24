//! Inline popup rendering — all popups render above the input box.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Paragraph, Row, Table, TableState},
};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use crate::agent::AgentStatus;
use crate::core::types::AgentId;
use crate::models::filter_providers;
use crate::tui::agent_tree::build_tool_trace_preview;

use crate::tui::state::{AppState, PopupMode};

use crate::tui::style;

/// Height reserved for the inline popup. 0 when no popup is shown.
pub(crate) fn popup_height(state: &AppState) -> u16 {
    match &state.popup_mode {
        PopupMode::None => 0,
        PopupMode::Providers => {
            let count = filter_providers(state.core.models.providers(), &state.ui.input).len();
            ((count.min(8) as u16 + 1) + 1).min(12)
        }
        PopupMode::KeyInput => 5,
        PopupMode::ModelPicker => {
            let count = state
                .core
                .models
                .search_configured_models(&state.ui.input, &state.core.configured_providers)
                .len();
            ((count.min(8) as u16) + 1).min(10)
        }
        PopupMode::FilePicker { query: _ } => {
            let files = get_project_files_cached();
            let query = file_picker_query(&state.ui.input);
            let count = if query.is_empty() {
                files.len()
            } else {
                let q = query.to_lowercase();
                files
                    .iter()
                    .filter(|f| f.to_lowercase().contains(&q))
                    .count()
            };
            (count.min(8) as u16 + 2).min(10)
        }
        PopupMode::AgentDetail { .. } => 12,
        PopupMode::ShellInput { .. } => 3,
        PopupMode::CommandPalette => {
            let palette = &state.ui.command_palette;
            let count = palette.filtered_items().len();
            (count.min(8) as u16 + 2).min(10)
        }
    }
}

/// Render the appropriate inline popup.
pub(crate) fn render_popup(f: &mut Frame, area: Rect, state: &AppState) {
    match &state.popup_mode {
        PopupMode::None => {}
        PopupMode::ShellInput { cmd, input: _ } => {
            render_shell_input_popup(f, area, cmd, &state.ui.input)
        }
        PopupMode::Providers => render_provider_popup(f, area, state),
        PopupMode::KeyInput => render_key_popup(f, area, state),
        PopupMode::ModelPicker => render_model_popup(f, area, state),
        PopupMode::FilePicker { .. } => render_file_popup(f, area, state),
        PopupMode::AgentDetail { agent_id } => render_agent_detail_popup(f, area, state, agent_id),
        PopupMode::CommandPalette => render_command_palette_popup(f, area, state),
    }
}

fn render_shell_input_popup(f: &mut Frame, area: Rect, cmd: &str, current_input: &str) {
    // Strip the cmd prefix from input to get just the user's argument.
    // This handles both bare text (input was cleared, user typed fresh)
    // and prefixed text (input still contains the cmd).
    let arg = current_input
        .strip_prefix(cmd)
        .map(|s| s.trim())
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
        .unwrap_or("");
    let preview = if arg.is_empty() {
        format!("{} …", cmd)
    } else {
        format!("{} {}", cmd, arg)
    };
    f.render_widget(
        ratatui::widgets::Paragraph::new(preview)
            .block(crate::tui::style::panel("Shell Command"))
            .style(Style::default().fg(style::TEXT_PRIMARY)),
        area,
    );
}

fn render_command_palette_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let palette = &state.ui.command_palette;
    let items = palette.filtered_items();

    if items.is_empty() {
        return;
    }

    let path_str = palette.display_path();
    let max_item_id = items.iter().map(|i| i.display.len()).max().unwrap_or(10);

    let rows: Vec<Row> = items
        .iter()
        .map(|item| {
            // 用 path + id 作为过滤匹配，后续支持 highlight
            let display_text = if item.has_children {
                format!("> {}", item.display)
            } else {
                item.display.clone()
            };

            Row::new(vec![
                ratatui::text::Line::from(Span::styled(
                    display_text,
                    Style::default().fg(style::ACTIVE),
                )),
                ratatui::text::Line::from(Span::styled(&item.help, style::hint_style())),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    let sel = palette.selected.min(items.len().saturating_sub(1));
    table_state.select(Some(sel));

    let title = if path_str.len() > 30 {
        format!(
            "Command …{}",
            &path_str[path_str.len().saturating_sub(27)..]
        )
    } else {
        format!("Command {}", path_str)
    };

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(max_item_id as u16 + 2),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .block(crate::tui::style::panel(&title))
        .row_highlight_style(
            Style::default()
                .fg(crate::tui::style::HIGHLIGHT_FG)
                .bg(crate::tui::style::HIGHLIGHT_BG),
        ),
        area,
        &mut table_state,
    );
}

fn render_provider_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered = filter_providers(state.core.models.providers(), &state.ui.input);
    let total_items = filtered.len() + 1;

    if total_items == 0 {
        let block = style::panel("Providers");
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new("No matching providers.").style(style::hint_style()),
            inner,
        );
        return;
    }

    let max_name = filtered.iter().map(|p| p.name.len()).max().unwrap_or(0);
    let max_count = filtered
        .iter()
        .map(|p| format!("{} models", p.models.len()).len())
        .max()
        .unwrap_or(8);

    let mut rows: Vec<Row> = filtered
        .iter()
        .map(|p| {
            let is_configured = state.core.configured_providers.iter().any(|id| id == &p.id);
            let needs_key = !p.env.is_empty();
            let (icon, status_label, status_style) = if is_configured {
                (
                    "\u{2713}",
                    "configured",
                    Style::default().fg(style::SUCCESS),
                )
            } else if needs_key {
                ("", "needs key", style::hint_style())
            } else {
                ("", "no auth", Style::default().fg(style::ACTIVE))
            };
            Row::new(vec![
                Span::styled(icon, Style::default().fg(style::SUCCESS)),
                Span::styled(&p.name, style::value_style()),
                Span::styled(format!("{} models", p.models.len()), style::hint_style()),
                Span::styled(status_label, status_style),
            ])
        })
        .collect();

    rows.push(Row::new(vec![
        Span::raw(""),
        Span::styled("+ Add Custom Provider", Style::default().fg(style::ACTIVE)),
        Span::raw(""),
        Span::raw(""),
    ]));

    let mut table_state = TableState::default();
    table_state.select(Some(
        state.popup_selected.min(total_items.saturating_sub(1)),
    ));

    let block = style::panel("Providers");
    let inner = block.inner(area);
    f.render_widget(block, area);

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(max_name as u16 + 1),
                ratatui::layout::Constraint::Length(max_count as u16),
                ratatui::layout::Constraint::Length(12),
            ],
        )
        .row_highlight_style(
            Style::default()
                .fg(style::HIGHLIGHT_FG)
                .bg(style::HIGHLIGHT_BG),
        ),
        inner,
        &mut table_state,
    );
}

// ── Key input popup ──

fn render_key_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let provider_name = state
        .popup_key_provider
        .as_ref()
        .and_then(|pid| state.core.models.providers().iter().find(|p| p.id == *pid))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let block = style::panel(&format!("API Key \u{2014} {}", provider_name));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(inner);

    // Masked input
    let masked: String = state.ui.input.chars().map(|_| '\u{2022}').collect();
    let has_input = !state.ui.input.is_empty();
    let display = if has_input {
        masked.as_str()
    } else {
        "Type or paste your API key\u{2026}"
    };
    let input_style = if has_input {
        Style::default().fg(style::WARNING)
    } else {
        style::hint_style()
    };
    f.render_widget(
        Paragraph::new(Span::styled(display, input_style)).block(style::input_box(has_input)),
        chunks[0],
    );

    // Cursor
    let cursor_x = chunks[0].x + state.ui.input.chars().count() as u16 + 1;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor_position((cursor_x.min(chunks[0].right().saturating_sub(2)), cursor_y));

    // Hint
    style::render_hint(f, chunks[1], "Enter to confirm  \u{00b7}  Esc to cancel");
}

// ── Model picker popup ──

fn render_model_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let results = state
        .core
        .models
        .search_configured_models(&state.ui.input, &state.core.configured_providers);

    if results.is_empty() {
        let block = style::panel("Models");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if state.core.configured_providers.is_empty() {
            "No providers configured. Use /connect first."
        } else {
            "No matching models."
        };
        f.render_widget(Paragraph::new(msg).style(style::hint_style()), inner);
        return;
    }

    let max_name = results.iter().map(|(_, m)| m.name.len()).max().unwrap_or(0);

    let rows: Vec<Row> = results
        .iter()
        .map(|(p, m)| {
            let is_selected = state
                .core
                .selected_models
                .iter()
                .any(|sm| sm.provider_id == p.id && sm.model_id == m.id);
            Row::new(vec![
                if is_selected {
                    Span::styled("\u{2713}", Style::default().fg(style::SUCCESS))
                } else {
                    Span::raw("")
                },
                Span::styled(&m.name, style::value_style()),
                Span::styled(&p.name, style::hint_style()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    table_state.select(Some(
        state.popup_selected.min(results.len().saturating_sub(1)),
    ));

    let block = style::panel("Models");
    let inner = block.inner(area);
    f.render_widget(block, area);

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(max_name as u16 + 1),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .row_highlight_style(
            Style::default()
                .fg(style::HIGHLIGHT_FG)
                .bg(style::HIGHLIGHT_BG),
        ),
        inner,
        &mut table_state,
    );
}

// ── File picker popup ──

fn render_file_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let files = get_project_files_cached();
    let query = file_picker_query(&state.ui.input);
    let q = query.to_lowercase();
    let filtered: Vec<&str> = if q.is_empty() {
        files.iter().map(String::as_str).collect()
    } else {
        files
            .iter()
            .filter(|f| f.to_lowercase().contains(&q))
            .map(String::as_str)
            .collect()
    };

    if filtered.is_empty() {
        let block = style::panel("Files");
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new("No matching files.").style(style::hint_style()),
            inner,
        );
        return;
    }

    let max_path_len = filtered.iter().map(|f| f.len()).max().unwrap_or(20);
    let rows: Vec<Row> = filtered
        .iter()
        .map(|path| {
            let path_text = *path;
            // Show file icon based on extension
            let icon = file_icon(path_text);
            Row::new(vec![
                Span::styled(icon, Style::default().fg(style::TEXT_MUTED)),
                Span::styled(path_text.to_string(), style::value_style()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    let sel = state.popup_selected.min(filtered.len().saturating_sub(1));
    table_state.select(Some(sel));

    let block = style::panel("Files  (type to filter)");
    let inner = block.inner(area);
    f.render_widget(block, area);

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(2),
                ratatui::layout::Constraint::Length(max_path_len as u16 + 2),
            ],
        )
        .row_highlight_style(
            Style::default()
                .fg(style::HIGHLIGHT_FG)
                .bg(style::HIGHLIGHT_BG),
        ),
        inner,
        &mut table_state,
    );
}

// ── Agent detail popup (Phase 3) ──

fn render_agent_detail_popup(f: &mut Frame, area: Rect, state: &AppState, agent_id: &AgentId) {
    // Read agent data from pool (try_read — never block the render thread).
    let (name, role, status, depth, goal, result, trace) = match state.core.agent_pool.try_read() {
        Ok(pool) => {
            let Some(agent) = pool.agents().iter().find(|a| a.id == *agent_id) else {
                return;
            };
            let trace = build_tool_trace_preview(&pool, agent_id);
            (
                agent.name.clone(),
                agent.role.clone(),
                agent.status.clone(),
                agent.depth,
                agent.goal.clone(),
                agent.result.clone().unwrap_or_default(),
                trace,
            )
        }
        Err(_) => return,
    };

    let status_str = match status {
        AgentStatus::Idle => "Idle",
        AgentStatus::Planning => "Planning",
        AgentStatus::AwaitingChildren => "Awaiting Children",
        AgentStatus::Aggregating => "Aggregating",
        AgentStatus::Completed => "Completed",
        AgentStatus::Failed => "Failed",
    };

    // Truncate goal for display (char-safe).
    let goal_display = if goal.chars().count() > 60 {
        format!("{}…", goal.chars().take(60).collect::<String>())
    } else {
        goal
    };

    // Truncate result for display (char-safe).
    let result_display = if result.chars().count() > 80 {
        format!("{}…", result.chars().take(80).collect::<String>())
    } else {
        result
    };

    let mut lines = vec![
        format!(" Agent:  {}", name),
        format!(" Role:   {}", role),
        format!(" Status: {}", status_str),
        format!(" Depth:  {}", depth),
        format!(" Goal:   {}", goal_display),
    ];

    if !result_display.is_empty() {
        lines.push(format!(" Result: {}", result_display));
    }

    if !trace.is_empty() {
        lines.push(" Tools:".to_string());
        lines.extend(trace);
    }

    lines.push(String::new());
    lines.push("  [Esc] close".to_string());

    let text = ratatui::text::Text::from(ratatui::text::Line::from(
        lines
            .iter()
            .map(|l| Span::raw(l.as_str()))
            .collect::<Vec<_>>(),
    ));

    let popup = Paragraph::new(text)
        .block(style::panel("Agent Detail"))
        .style(Style::default().fg(style::TEXT_PRIMARY));

    f.render_widget(popup, area);
}

/// Return a simple file icon based on file extension.
fn file_icon(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "[rs]"
    } else if path.ends_with(".md") || path.ends_with(".txt") {
        "[doc]"
    } else if path.ends_with(".toml")
        || path.ends_with(".json")
        || path.ends_with(".yaml")
        || path.ends_with(".yml")
    {
        "[cfg]"
    } else if path.ends_with(".html")
        || path.ends_with(".css")
        || path.ends_with(".js")
        || path.ends_with(".ts")
    {
        "[web]"
    } else if path.ends_with(".py") {
        "[py]"
    } else {
        "[file]"
    }
}

/// Get the query text after the last `@` in the input.
pub(crate) fn file_picker_query(input: &str) -> &str {
    if let Some(pos) = input.rfind('@') {
        &input[pos + 1..]
    } else {
        ""
    }
}

/// Scan the project directory for files, with caching.
static FILE_CACHE: OnceLock<RwLock<ProjectFileCache>> = OnceLock::new();
const FILE_CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct ProjectFileCache {
    files: Vec<String>,
    refreshed_at: Instant,
}

pub(crate) fn get_project_files_cached() -> Vec<String> {
    let cache = FILE_CACHE.get_or_init(|| {
        RwLock::new(ProjectFileCache {
            files: scan_project_files(),
            refreshed_at: Instant::now(),
        })
    });

    if let Ok(guard) = cache.read() {
        if guard.refreshed_at.elapsed() < FILE_CACHE_TTL {
            return guard.files.clone();
        }
    }

    let refreshed = scan_project_files();
    if let Ok(mut guard) = cache.write() {
        guard.files = refreshed.clone();
        guard.refreshed_at = Instant::now();
    }
    refreshed
}

fn scan_project_files() -> Vec<String> {
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(_) => return Vec::new(),
    };
    if cwd.as_os_str().is_empty() {
        return Vec::new();
    }

    // Use `git ls-files` to respect .gitignore and avoid hardcoded skip lists.
    // This includes both tracked and untracked (non-ignored) files.
    let mut files: Vec<String> = match std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(&cwd)
        .output()
    {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().map(|s| s.to_string()).collect()
        }
        _ => {
            let mut f = Vec::new();
            let skip_dirs: &[&str] = &[".git", "target", "node_modules"];
            let walker = walkdir::WalkDir::new(&cwd).into_iter().filter_entry(|e| {
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    !skip_dirs.contains(&name.as_ref())
                } else {
                    true
                }
            });
            for entry in walker.flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                if let Ok(rel) = entry.path().strip_prefix(&cwd) {
                    f.push(rel.display().to_string());
                }
            }
            f
        }
    };

    files.sort_by(|a: &String, b: &String| {
        let a_is_src = a.starts_with("src") || a.starts_with("workflow/src");
        let b_is_src = b.starts_with("src") || b.starts_with("workflow/src");
        a_is_src.cmp(&b_is_src).reverse().then(a.cmp(b))
    });
    files
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use crate::agent::{Agent, AgentConfig, AgentStatus, ToolCallRecord, ToolStatus};
    use crate::core::types::AgentId;
    use crate::tui::agent_tree::build_tool_trace_preview;
    use crate::tui::state::{AppState, PopupMode};

    use super::{popup_height, render_agent_detail_popup};

    /// AgentDetail popup reserves 12 lines.
    #[test]
    fn test_agent_detail_popup_height() {
        let app = AppState::default();
        let agent_id: AgentId = [42u8; 16];
        // popup_height reads popup_mode, not pool, so it's fine if
        // the agent doesn't exist yet.
        let mut app = app;
        app.popup_mode = PopupMode::AgentDetail { agent_id };
        assert_eq!(popup_height(&app), 12);
    }

    /// Verify AgentDetail data extraction does not panic with
    /// a fully populated agent (tool_trace + child_results).
    #[test]
    fn test_agent_detail_data_extraction() {
        use std::collections::VecDeque;

        let mut app = AppState::default();
        let agent_id: AgentId = [42u8; 16];

        {
            let mut pool = app.core.agent_pool.try_write().unwrap();
            pool.add_agent(Agent {
                id: agent_id,
                name: "dev-test".into(),
                role: "developer".into(),
                status: AgentStatus::Completed,
                depth: 1,
                goal: "Implement auth".into(),
                result: Some("Done".into()),
                role_template_id: None,
                parent_id: None,
                children: Vec::new(),
                config: AgentConfig::default(),
                context: Vec::new(),
                child_results: vec![([1u8; 16], "sub-result".into())],
                last_active_at: 0,
                tokens_input: 0,
                tokens_output: 0,
                tool_trace: VecDeque::from(vec![
                    ToolCallRecord {
                        name: "grep".into(),
                        args_preview: "pattern=fn auth".into(),
                        status: ToolStatus::Success,
                    },
                    ToolCallRecord {
                        name: "read_file".into(),
                        args_preview: "path=auth.rs".into(),
                        status: ToolStatus::Success,
                    },
                ]),
                inbox: VecDeque::new(),
                task_id: None,
                sandbox: None,
                retry_count: 0,
            });
        }

        app.popup_mode = PopupMode::AgentDetail { agent_id };

        // Data extraction (same as render_agent_detail_popup).
        let (_name, _role, trace) = match app.core.agent_pool.try_read() {
            Ok(pool) => {
                let agent = pool.agents().iter().find(|a| a.id == agent_id).unwrap();
                let trace = build_tool_trace_preview(&pool, &agent_id);
                (agent.name.clone(), agent.role.clone(), trace)
            }
            Err(_) => panic!("lock"),
        };
        assert_eq!(trace.len(), 2);
        assert!(trace[0].contains("read_file"));
        assert!(trace[1].contains("grep"));
    }

    /// Verify that [`render_agent_detail_popup`] does not panic for a
    /// well-formed agent, even with an empty tool_trace.
    #[test]
    fn test_agent_detail_popup_render_does_not_panic() {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        let agent_id: AgentId = [42u8; 16];
        let mut app_state = AppState::default();

        // Populate pool (sync try_write — safe in single-threaded test).
        {
            let mut pool = app_state.core.agent_pool.try_write().unwrap();
            pool.add_agent(Agent {
                id: agent_id,
                name: "stub-agent".into(),
                role: "stub".into(),
                role_template_id: None,
                parent_id: None,
                children: Vec::new(),
                depth: 0,
                goal: "Test goal for render".into(),
                config: AgentConfig::default(),
                status: AgentStatus::Planning,
                result: None,
                child_results: Vec::new(),
                context: Vec::new(),
                last_active_at: 0,
                tokens_input: 0,
                tokens_output: 0,
                tool_trace: std::collections::VecDeque::new(),
                inbox: std::collections::VecDeque::new(),
                task_id: None,
                sandbox: None,
                retry_count: 0,
            });
        }

        app_state.popup_mode = PopupMode::AgentDetail { agent_id };
        app_state.ui.input_disabled = true;

        // Render one frame — if this doesn't panic, the popup is sound.
        let result = terminal.draw(|f| {
            let area = Rect::new(0, 0, 58, 10);
            render_agent_detail_popup(f, area, &app_state, &agent_id);
        });

        assert!(result.is_ok(), "AgentDetail popup render should not panic");
    }
}
