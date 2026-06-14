//! Inline popup rendering — all popups render above the input box.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Paragraph, Row, Table, TableState},
};
use std::sync::OnceLock;

use crate::models::filter_providers;
use crate::tui::commands::{COMMANDS, resolve_dynamic_items};
use crate::tui::state::{AppState, PopupMode};

use crate::tui::style;

/// Height reserved for the inline popup. 0 when no popup is shown.
pub(crate) fn popup_height(state: &AppState) -> u16 {
    match &state.popup_mode {
        PopupMode::None => 0,
        PopupMode::Commands => {
            let prefix = state.ui.input.trim().to_lowercase();
            let count = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).count();
            (count.min(6) as u16 + 2).min(8)
        }
        PopupMode::SubCommand { parent, items } => {
            let resolved = resolve_subcommand_items_owned(parent, items, state);
            let filter = filter_text_for_subcommand(&state.ui.input, parent);
            let count = if filter.is_empty() {
                resolved.len()
            } else {
                let fl = filter.to_lowercase();
                resolved
                    .iter()
                    .filter(|(name, _)| name.to_lowercase().contains(&fl))
                    .count()
            };
            (count.min(8) as u16 + 2).min(10)
        }
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
            let files = get_project_files();
            let query = file_picker_query(&state.ui.input);
            let count = if query.is_empty() {
                files.len()
            } else {
                let q = query.to_lowercase();
                files.iter().filter(|f| f.to_lowercase().contains(&q)).count()
            };
            (count.min(8) as u16 + 2).min(10)
        }
    }
}

/// Render the appropriate inline popup.
pub(crate) fn render_popup(f: &mut Frame, area: Rect, state: &AppState) {
    match &state.popup_mode {
        PopupMode::None => {}
        PopupMode::Commands => render_command_popup(f, area, state),
        PopupMode::SubCommand { parent, items } => render_subcommand_popup(f, area, state, parent, items),
        PopupMode::Providers => render_provider_popup(f, area, state),
        PopupMode::KeyInput => render_key_popup(f, area, state),
        PopupMode::ModelPicker => render_model_popup(f, area, state),
        PopupMode::FilePicker { .. } => render_file_popup(f, area, state),
    }
}

/// Resolve owned items for popup_height / filtering when items are empty.
fn resolve_subcommand_items_owned(parent: &str, items: &[(String, String)], state: &AppState) -> Vec<(String, String)> {
    if !items.is_empty() {
        return items.to_vec();
    }
    resolve_dynamic_items(parent, &state.core)
}

/// Extract filter text for subcommand popup.
/// If input starts with parent+space, extract the suffix.
/// Otherwise use the whole input (dispatch cleared the parent).
fn filter_text_for_subcommand<'a>(input: &'a str, parent: &str) -> &'a str {
    if input.starts_with(parent) {
        let after = &input[parent.len()..];
        if after.starts_with(' ') { &after[1..] } else { after }
    } else {
        input
    }
}

// ── Command popup ──

fn render_command_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let prefix = state.ui.input.trim().to_lowercase();
    let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
    if matches.is_empty() {
        return;
    }

    let max_cmd_len = matches.iter().map(|(cmd, _)| cmd.len()).max().unwrap_or(10);
    let rows: Vec<Row> = matches
        .iter()
        .map(|(cmd, desc)| {
            Row::new(vec![
                Span::styled(*cmd, Style::default().fg(style::ACTIVE)),
                Span::styled(*desc, style::hint_style()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    let sel = state.popup_selected.min(matches.len().saturating_sub(1));
    table_state.select(Some(sel));

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(max_cmd_len as u16),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .block(style::panel("Commands"))
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        area,
        &mut table_state,
    );
}

fn render_subcommand_popup(f: &mut Frame, area: Rect, state: &AppState, parent: &str, items: &[(String, String)]) {
    // Resolve items: use stored items, or fetch dynamic items
    let resolved = if items.is_empty() {
        resolve_dynamic_items(parent, &state.core)
    } else {
        items.to_vec()
    };

    if resolved.is_empty() {
        return;
    }

    let filter = filter_text_for_subcommand(&state.ui.input, parent);
    let fl = filter.to_lowercase();
    let filtered: Vec<_> = if filter.is_empty() {
        resolved.iter().collect()
    } else {
        resolved
            .iter()
            .filter(|(name, _)| name.to_lowercase().contains(&fl))
            .collect()
    };

    if filtered.is_empty() {
        return;
    }

    let max_name_len = filtered.iter().map(|(name, _)| name.len()).max().unwrap_or(10);
    let rows: Vec<Row> = filtered
        .iter()
        .map(|(name, desc)| {
            Row::new(vec![
                Span::styled(name.as_str(), Style::default().fg(style::ACTIVE)),
                Span::styled(desc.as_str(), style::hint_style()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    let sel = state.popup_selected.min(filtered.len().saturating_sub(1));
    table_state.select(Some(sel));

    let title = if items.is_empty() && !resolved.is_empty() {
        // Dynamic items — derive title from parent
        let parts: Vec<&str> = parent.rsplitn(2, ' ').collect();
        let sub = parts[0];
        let capitalized = sub[..1].to_uppercase() + &sub[1..];
        format!("{} · {}", parts.get(1).unwrap_or(&""), capitalized)
    } else {
        format!("{} · sub-commands", parent)
    };
    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(max_name_len as u16),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .block(style::panel(&title))
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        area,
        &mut table_state,
    );
}

// ── Provider popup ──

fn render_provider_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered = filter_providers(state.core.models.providers(), &state.ui.input);
    // Always show "Add Custom Provider" row at the bottom regardless of filter.
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
                ("\u{2713}", "configured", Style::default().fg(style::SUCCESS))
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
    table_state.select(Some(state.popup_selected.min(total_items.saturating_sub(1))));

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
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
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
    table_state.select(Some(state.popup_selected.min(results.len().saturating_sub(1))));

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
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        inner,
        &mut table_state,
    );
}

// ── File picker popup ──

fn render_file_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let files = get_project_files();
    let query = file_picker_query(&state.ui.input);
    let q = query.to_lowercase();
    let filtered: Vec<&String> = if q.is_empty() {
        files.iter().collect()
    } else {
        files.iter().filter(|f| f.to_lowercase().contains(&q)).collect()
    };

    if filtered.is_empty() {
        let block = style::panel("Files");
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(Paragraph::new("No matching files.").style(style::hint_style()), inner);
        return;
    }

    let max_path_len = filtered.iter().map(|f| f.len()).max().unwrap_or(20);
    let rows: Vec<Row> = filtered
        .iter()
        .map(|path| {
            // Show file icon based on extension
            let icon = file_icon(path);
            Row::new(vec![
                Span::styled(icon, Style::default().fg(style::TEXT_MUTED)),
                Span::styled(path.as_str(), style::value_style()),
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
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        inner,
        &mut table_state,
    );
}

/// Return a simple file icon based on file extension.
fn file_icon(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "\u{1f99b}" // 🦀 rust
    } else if path.ends_with(".md") || path.ends_with(".txt") {
        "\u{1f4c4}" // 📄 document
    } else if path.ends_with(".toml") || path.ends_with(".json") || path.ends_with(".yaml") || path.ends_with(".yml") {
        "\u{2699}" // ⚙ config
    } else if path.ends_with(".html") || path.ends_with(".css") || path.ends_with(".js") || path.ends_with(".ts") {
        "\u{1f310}" // 🌐 web
    } else if path.ends_with(".py") {
        "\u{1f40d}" // 🐍 python
    } else {
        "\u{1f4c1}" // 📁 file
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
static FILE_CACHE: OnceLock<Vec<String>> = OnceLock::new();

pub(crate) fn get_project_files_cached() -> &'static Vec<String> {
    get_project_files()
}

fn get_project_files() -> &'static Vec<String> {
    FILE_CACHE.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_default();
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
                // Fallback: walk directory when git is not available
                // with minimal skips for performance
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
    })
}
