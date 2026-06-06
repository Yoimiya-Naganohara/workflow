use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use super::Tui;
use super::state::{AppMode, AppState, COMMANDS, Focus, MessageRole, MessageStatus, Panel};
use crate::models::{Model, Provider, filter_providers};

impl Tui {
    pub(crate) async fn draw(&mut self) -> Result<()> {
        let state = self.state.read().await;

        // Compute model picker results before draw to avoid borrow issues
        let model_picker_results = if state.show_model_picker {
            Some(state.models.search_models(&state.model_picker_search_query))
        } else {
            None
        };

        // Rebuild cached chat lines only when messages change or during streaming
        let term_size = self.terminal.size()?;
        let chat_width = (term_size.width.saturating_sub(4)).max(1) as usize;
        let msg_count = state.messages.len();
        let is_streaming = state.active_chat_requests > 0;
        if is_streaming
            || self.chat_lines_cache.is_empty()
            || self.chat_cache_msg_count != msg_count
            || self.chat_cache_width != chat_width
        {
            self.chat_lines_cache = Self::build_chat_lines(&state, chat_width);
            self.chat_cache_msg_count = msg_count;
            self.chat_cache_width = chat_width;
        }
        let cached_chat_lines = &self.chat_lines_cache;

        self.terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            // Header
            Self::render_header(f, chunks[0], &state);

            // Main content: chat area + optional status panel
            let (chat_area, main_chunks) = if state.show_status_panel {
                let c = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
                    .split(chunks[1]);
                (c[0], Some(c[1]))
            } else {
                (chunks[1], None)
            };
            match state.panel {
                Panel::Chat => Self::render_chat_panel(f, chat_area, &state, cached_chat_lines),
            }
            if let Some(area) = main_chunks {
                Self::render_status_panel(f, area, &state);
            }

            // Status bar
            Self::render_status_bar(f, chunks[2], &state);

            // Provider dialog overlay
            if state.show_provider_dialog {
                Self::render_provider_dialog(f, f.area(), &state);
            }

            // Key dialog overlay
            if state.show_key_dialog {
                Self::render_key_dialog(f, f.area(), &state);
            }

            // Model picker overlay
            if let Some(results) = &model_picker_results {
                Self::render_model_picker(f, f.area(), &state, results);
            }

            // Command popup
            if state.focus == Focus::Input && state.input.starts_with('/') && !state.input.trim().is_empty() {
                Self::render_command_popup(f, chat_area, &state);
            }
        })?;

        Ok(())
    }

    fn render_header(f: &mut Frame, area: Rect, state: &AppState) {
        let model_name = state
            .selected_models
            .first()
            .map(|sm| format!("{} / {}", sm.provider_name, sm.model_name));
        let model_info = model_name.as_deref().unwrap_or("no model");
        let thinking = if state.active_chat_requests > 0 {
            format!(" ◌ {}", state.active_chat_requests)
        } else {
            String::new()
        };

        let header = Line::from(vec![
            Span::styled(
                " workflow ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" │ "),
            Span::styled(model_info, Style::default().fg(Color::DarkGray)),
            Span::raw(&thinking),
        ]);

        f.render_widget(Paragraph::new(header), area);
    }

    fn render_chat_panel(f: &mut Frame, area: Rect, state: &AppState, chat_lines: &[Line<'static>]) {
        Self::render_chat(f, area, state, chat_lines);
    }

    fn render_chat(f: &mut Frame, area: Rect, state: &AppState, chat_lines: &[Line<'static>]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);

        let chat_height = chunks[0].height.saturating_sub(2) as usize;
        let total_lines = chat_lines.len();
        let max_scroll = total_lines.saturating_sub(chat_height);
        let chat_scroll = state.chat_scroll.min(max_scroll);

        // Only render visible lines
        let visible = if total_lines > chat_height {
            let start = max_scroll.saturating_sub(chat_scroll);
            let end = (start + chat_height).min(total_lines);
            &chat_lines[start..end]
        } else {
            chat_lines
        };

        let chat = Paragraph::new(Text::from(visible.to_vec())).wrap(Wrap { trim: false });
        f.render_widget(chat, chunks[0]);

        // Input
        let input_style = if state.focus == Focus::Input {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let input_text = if state.input.is_empty() {
            "Type a message or /command..."
        } else {
            state.input.as_str()
        };
        let input_style = if state.input.is_empty() {
            input_style.fg(Color::DarkGray)
        } else {
            input_style
        };

        f.render_widget(
            Paragraph::new(input_text)
                .style(input_style)
                .block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );

        if state.focus == Focus::Input {
            let prefix_width = Self::display_width_up_to(&state.input, state.input_cursor);
            let visible_cursor = prefix_width.min(chunks[1].width.saturating_sub(3) as usize);
            let cursor_x = chunks[1].x + visible_cursor as u16 + 1;
            let cursor_y = chunks[1].y + 1;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn render_status_panel(f: &mut Frame, area: Rect, state: &AppState) {
        if area.width < 2 {
            return;
        }
        let block = Block::default()
            .borders(Borders::LEFT)
            .style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.height < 3 {
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        // ── System ──
        lines.push(Line::from(Span::styled(
            " ── System ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));

        let mode_str = match state.mode {
            AppMode::Plan => "plan",
            AppMode::Build => "build",
        };
        lines.push(Line::from(vec![
            Span::styled("  mode    ", Style::default().fg(Color::DarkGray)),
            Span::styled(mode_str, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]));

        let focus_str = match state.focus {
            Focus::Sidebar => "sidebar",
            Focus::Chat => "chat",
            Focus::Input => "input",
        };
        lines.push(Line::from(vec![
            Span::styled("  focus   ", Style::default().fg(Color::DarkGray)),
            Span::styled(focus_str, Style::default().fg(Color::White)),
        ]));

        let panel_str = match state.panel {
            Panel::Chat => "chat",
        };
        lines.push(Line::from(vec![
            Span::styled("  panel   ", Style::default().fg(Color::DarkGray)),
            Span::styled(panel_str, Style::default().fg(Color::White)),
        ]));

        // ── Budget ──
        lines.push(Line::from(Span::styled(
            " ── Budget ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));

        let budget_pct = if state.budget_total > 0 {
            state.budget_used * 100 / state.budget_total
        } else {
            0
        };
        let budget_color = if budget_pct > 80 {
            Color::Red
        } else if budget_pct > 50 {
            Color::Yellow
        } else {
            Color::Green
        };
        lines.push(Line::from(vec![
            Span::styled("  used    ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", state.budget_used), Style::default().fg(budget_color)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  total   ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", state.budget_total), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  pct     ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}%", budget_pct), Style::default().fg(budget_color)),
        ]));

        // ── Permits ──
        lines.push(Line::from(Span::styled(
            " ── Permits ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));

        lines.push(Line::from(vec![
            Span::styled("  avail   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", state.permits_available),
                Style::default().fg(if state.permits_available > 0 {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  total   ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", state.permits_total), Style::default().fg(Color::White)),
        ]));

        // ── Model ──
        lines.push(Line::from(Span::styled(
            " ── Model ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        if let Some(sm) = state.selected_models.first() {
            lines.push(Line::from(vec![
                Span::styled("  name    ", Style::default().fg(Color::DarkGray)),
                Span::styled(&sm.model_name, Style::default().fg(Color::White)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  provider", Style::default().fg(Color::DarkGray)),
                Span::styled(&sm.provider_name, Style::default().fg(Color::Cyan)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  none    ", Style::default().fg(Color::DarkGray)),
                Span::styled("no model", Style::default().fg(Color::DarkGray)),
            ]));
        }

        // ── Network ──
        lines.push(Line::from(Span::styled(
            " ── Network ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        let configured = state.configured_providers.len();
        let clients = state.provider_clients.len();
        lines.push(Line::from(vec![
            Span::styled("  config  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} providers", configured),
                Style::default().fg(if configured > 0 { Color::Green } else { Color::DarkGray }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  clients ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} active", clients),
                Style::default().fg(if clients > 0 { Color::Cyan } else { Color::DarkGray }),
            ),
        ]));
        if state.active_chat_requests > 0 {
            lines.push(Line::from(vec![
                Span::styled("  request ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "streaming",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
                ),
            ]));
        }

        // ── Plan ──
        if let Some(plan) = &state.current_plan {
            lines.push(Line::from(Span::styled(
                " ── Plan ──",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
            )));

            let total = plan.tasks.len();
            let completed = plan
                .tasks
                .iter()
                .filter(|t| t.status == crate::plan::TaskStatus::Completed)
                .count();
            let running = plan
                .tasks
                .iter()
                .filter(|t| t.status == crate::plan::TaskStatus::Running)
                .count();
            let pending = plan
                .tasks
                .iter()
                .filter(|t| t.status == crate::plan::TaskStatus::Pending)
                .count();
            let failed = plan
                .tasks
                .iter()
                .filter(|t| t.status == crate::plan::TaskStatus::Failed)
                .count();

            let plan_color = if failed > 0 {
                Color::Red
            } else if completed == total {
                Color::Green
            } else {
                Color::Yellow
            };
            lines.push(Line::from(vec![
                Span::styled("  status  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    match state.mode {
                        AppMode::Plan => "planning",
                        AppMode::Build => "building",
                    },
                    Style::default().fg(plan_color),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  tasks   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}/{}", completed, total),
                    Style::default().fg(if completed == total { Color::Green } else { Color::White }),
                ),
            ]));
            if running > 0 {
                lines.push(Line::from(vec![
                    Span::styled("  running ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}", running), Style::default().fg(Color::Yellow)),
                ]));
            }
            if pending > 0 {
                lines.push(Line::from(vec![
                    Span::styled("  pending ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}", pending), Style::default().fg(Color::DarkGray)),
                ]));
            }
            if failed > 0 {
                lines.push(Line::from(vec![
                    Span::styled("  failed  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}", failed), Style::default().fg(Color::Red)),
                ]));
            }

            // Goal (truncated)
            if !plan.goal.is_empty() {
                let goal = if plan.goal.len() > (area.width as usize).saturating_sub(6) {
                    format!("{}…", &plan.goal[..(area.width as usize).saturating_sub(8)])
                } else {
                    plan.goal.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled("  goal    ", Style::default().fg(Color::DarkGray)),
                    Span::styled(goal, Style::default().fg(Color::White)),
                ]));
            }
        }

        // ── Messages ──
        lines.push(Line::from(Span::styled(
            " ── Messages ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        let user_msgs = state.messages.iter().filter(|m| m.role == MessageRole::User).count();
        let agent_msgs = state.messages.iter().filter(|m| m.role == MessageRole::Agent).count();
        let sys_msgs = state.messages.iter().filter(|m| m.role == MessageRole::System).count();
        lines.push(Line::from(vec![
            Span::styled("  total   ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", state.messages.len()), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  user    ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", user_msgs), Style::default().fg(Color::Cyan)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  agent   ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", agent_msgs), Style::default().fg(Color::Blue)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  system  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", sys_msgs), Style::default().fg(Color::DarkGray)),
        ]));

        // ── Queue ──
        lines.push(Line::from(Span::styled(
            " ── Queue ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", state.chat_scroll), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  history ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} entries", state.input_history.len()),
                Style::default().fg(Color::White),
            ),
        ]));

        // ── Experience ──
        if let Some(ref rt) = state.runtime {
            let rt = rt.try_read();
            if let Ok(rt) = rt {
                let exp_count = rt.experience_count();
                let pending = rt.pending_suspended();
                lines.push(Line::from(Span::styled(
                    " ── Experience ──",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(vec![
                    Span::styled("  pool    ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{} entries", exp_count),
                        Style::default().fg(if exp_count > 0 { Color::Green } else { Color::DarkGray }),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  suspend ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{} queued", pending),
                        Style::default().fg(if pending > 0 { Color::Yellow } else { Color::DarkGray }),
                    ),
                ]));
            }
        }

        // ── Runtime ──
        lines.push(Line::from(Span::styled(
            " ── Runtime ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  request ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("#{}", state.active_chat_request_id),
                Style::default().fg(Color::White),
            ),
        ]));
        if let Some(plan) = &state.current_plan {
            let plan_status = match plan.status {
                crate::plan::PlanStatus::Draft => "draft",
                crate::plan::PlanStatus::Approved => "approved",
                crate::plan::PlanStatus::Executing => "executing",
                crate::plan::PlanStatus::Completed => "completed",
                crate::plan::PlanStatus::Failed => "failed",
            };
            lines.push(Line::from(vec![
                Span::styled("  plan    ", Style::default().fg(Color::DarkGray)),
                Span::styled(plan_status, Style::default().fg(Color::Cyan)),
            ]));
        }
        let agents_running = state
            .agents
            .iter()
            .filter(|a| a.status == crate::tui::state::AgentStatus::Running)
            .count();
        let agents_total = state.agents.len();
        lines.push(Line::from(vec![
            Span::styled("  agents  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/{}", agents_running, agents_total),
                Style::default().fg(if agents_running > 0 {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
        ]));

        let status = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        f.render_widget(status, inner);
    }

    fn render_provider_dialog(f: &mut Frame, area: Rect, state: &AppState) {
        let providers = state.models.providers();
        if providers.is_empty() {
            return;
        }

        let filtered = filter_providers(providers, &state.provider_search_query);

        if filtered.is_empty() {
            let dialog_w = 30.min(area.width.saturating_sub(4));
            let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
            let y = area.y + 4;
            let no_match = Rect::new(x, y, dialog_w, 3);
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Select Provider ")
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(Paragraph::new("No matching providers").block(block), no_match);
            return;
        }

        let dialog_w = 44.min(area.width.saturating_sub(4));
        let list_h = filtered.len() as u16;
        let search_h = 3u16;
        let dialog_h = (list_h + 4 + search_h).min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(search_h), Constraint::Min(0)].as_ref())
            .split(dialog_area);

        let search_style = Style::default().fg(Color::Cyan);
        f.render_widget(
            Paragraph::new(state.provider_search_query.as_str())
                .style(search_style)
                .block(Block::default().borders(Borders::ALL).title("Search")),
            chunks[0],
        );
        let prefix_width = Self::display_width_up_to(&state.provider_search_query, state.provider_search_cursor);
        let cursor_x = chunks[0].x + prefix_width as u16 + 1;
        let cursor_y = chunks[0].y + 1;
        f.set_cursor_position((cursor_x, cursor_y));

        let items: Vec<ListItem> = filtered
            .iter()
            .map(|p| {
                let count = p.models.len();
                let env = p.env.first().map(|e| e.as_str()).unwrap_or("no key");
                ListItem::new(Line::from(vec![
                    Span::styled(&p.name, Style::default()),
                    Span::raw("  "),
                    Span::styled(format!("{} models", count), Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(format!("env: {}", env), Style::default().fg(Color::Yellow)),
                ]))
            })
            .collect();

        let list_area = chunks[1];

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select Provider ")
            .style(Style::default().fg(Color::Cyan));

        let mut list_state = ListState::default();
        list_state.select(Some(state.selected_provider_idx.min(filtered.len().saturating_sub(1))));
        f.render_stateful_widget(
            List::new(items)
                .block(block)
                .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .highlight_symbol("❯ "),
            list_area,
            &mut list_state,
        );
    }

    fn render_key_dialog(f: &mut Frame, area: Rect, state: &AppState) {
        let provider_name = state
            .key_provider_id
            .as_ref()
            .and_then(|id| state.models.providers().iter().find(|p| &p.id == id))
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown");

        let dialog_w = 50.min(area.width.saturating_sub(4));
        let dialog_h = 7;
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" API Key: {} ", provider_name))
            .style(Style::default().fg(Color::Yellow));

        let inner = block.inner(dialog_area);
        f.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(inner);

        // Masked input display
        let masked: String = "*".repeat(state.key_input.len());
        let input_text = if state.key_input.is_empty() {
            "Enter API key...".to_string()
        } else {
            masked
        };

        let input_style = if state.key_input.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        f.render_widget(
            Paragraph::new(input_text)
                .style(input_style)
                .block(Block::default().borders(Borders::ALL).title("Key")),
            chunks[0],
        );

        // Set cursor in input
        let prefix_width = Self::display_width_up_to(&state.key_input, state.key_cursor);
        let cursor_x = chunks[0].x + prefix_width as u16 + 1;
        let cursor_y = chunks[0].y + 1;
        f.set_cursor_position((cursor_x, cursor_y));

        // Hints
        let hint = Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(": confirm  "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(": cancel"),
        ]);
        f.render_widget(Paragraph::new(hint), chunks[1]);
    }

    fn render_model_picker(f: &mut Frame, area: Rect, state: &AppState, results: &[(&Provider, &Model)]) {
        if results.is_empty() {
            let dialog_w = 40.min(area.width.saturating_sub(4));
            let dialog_h = 5;
            let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
            let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
            let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Select Model ")
                .style(Style::default().fg(Color::Cyan));

            let inner = block.inner(dialog_area);
            f.render_widget(block, dialog_area);

            let msg = if state.models.providers().is_empty() {
                "No providers loaded. Type /connect to fetch."
            } else {
                "No matching models."
            };

            f.render_widget(Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)), inner);
            return;
        }

        let dialog_w = 60.min(area.width.saturating_sub(4));
        let search_h = 3u16;
        let list_h = results.len() as u16;
        let dialog_h = (list_h + 4 + search_h).min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select Model ")
            .style(Style::default().fg(Color::Cyan));

        let inner = block.inner(dialog_area);
        f.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(search_h), Constraint::Min(0)])
            .split(inner);

        // Search bar
        let search_style = Style::default().fg(Color::Cyan);
        f.render_widget(
            Paragraph::new(state.model_picker_search_query.as_str())
                .style(search_style)
                .block(Block::default().borders(Borders::ALL).title("Search")),
            chunks[0],
        );
        let prefix_width =
            Self::display_width_up_to(&state.model_picker_search_query, state.model_picker_search_cursor);
        let cursor_x = chunks[0].x + prefix_width as u16 + 1;
        let cursor_y = chunks[0].y + 1;
        f.set_cursor_position((cursor_x, cursor_y));

        // Model list
        let configured = &state.configured_providers;
        let items: Vec<ListItem> = results
            .iter()
            .map(|(p, m)| {
                let needs_key = !configured.contains(&p.id) && !crate::controller::is_no_auth_provider(&p.id);
                ListItem::new(Line::from(vec![
                    Span::styled(&p.name, Style::default().fg(Color::DarkGray)),
                    Span::raw(" / "),
                    Span::styled(&m.name, Style::default()),
                    Span::raw("  "),
                    Span::styled(
                        if let Some(pos) = state
                            .selected_models
                            .iter()
                            .position(|sm| sm.provider_id == p.id && sm.model_id == m.id)
                        {
                            format!("[{}]", pos + 1)
                        } else if needs_key {
                            " ⌁".to_string()
                        } else {
                            "   ".to_string()
                        },
                        if needs_key {
                            Style::default().fg(Color::Yellow)
                        } else {
                            Style::default().fg(Color::Green)
                        },
                    ),
                ]))
            })
            .collect();

        let list_area = chunks[1];

        let list_block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));

        let mut list_state = ListState::default();
        list_state.select(Some(
            state.selected_model_picker_idx.min(results.len().saturating_sub(1)),
        ));
        f.render_stateful_widget(
            List::new(items)
                .block(list_block)
                .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .highlight_symbol("❯ "),
            list_area,
            &mut list_state,
        );
    }

    fn render_command_popup(f: &mut Frame, chat_area: Rect, state: &AppState) {
        let prefix = state.input.trim().to_lowercase();
        let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();

        if matches.is_empty() {
            return;
        }

        let items: Vec<ListItem> = matches
            .iter()
            .map(|(cmd, desc)| {
                ListItem::new(Line::from(vec![
                    Span::styled(*cmd, Style::default()),
                    Span::styled(format!("  — {}", desc), Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect();

        let popup_h = (matches.len() as u16).clamp(3, 6) + 2;
        let popup_w = 50.min(chat_area.width.saturating_sub(4));
        let x = chat_area.x;
        let y = chat_area.y + chat_area.height.saturating_sub(popup_h + 3);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Commands ")
            .style(Style::default().fg(Color::Cyan));

        let mut list_state = ListState::default();
        list_state.select(Some(state.command_popup_selection.min(matches.len().saturating_sub(1))));
        f.render_stateful_widget(
            List::new(items)
                .block(block)
                .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .highlight_symbol("❯ "),
            Rect::new(x, y, popup_w, popup_h),
            &mut list_state,
        );
    }

    fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
        let mode_indicator = match state.mode {
            AppMode::Plan => Span::styled(
                " PLAN ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            AppMode::Build => Span::styled(
                " BUILD ",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        };

        let hint: String = match state.panel {
            Panel::Chat => {
                if state.show_provider_dialog {
                    "Type to filter | Up/Down navigate | Enter select | Esc cancel".to_string()
                } else if state.show_model_picker {
                    "Type to filter | Up/Down navigate | Ctrl+A provider | Enter toggle | Esc cancel".to_string()
                } else if state.focus == Focus::Chat {
                    "Up/Down scroll | Ctrl+P models | Ctrl+C quit".to_string()
                } else {
                    let panel_hint = if state.show_status_panel {
                        "Tab hide panel"
                    } else {
                        "Tab show panel"
                    };
                    let mode_hint = match state.mode {
                        AppMode::Plan => "Type a goal · /apply build · /connect provider · /models pick",
                        AppMode::Build => "/apply execute · /connect provider · Ctrl+P models",
                    };
                    format!("{} · {} · Ctrl+C quit", mode_hint, panel_hint)
                }
            }
        };

        f.render_widget(
            Paragraph::new(Line::from(vec![
                mode_indicator,
                Span::raw("  "),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ])),
            area,
        );
    }

    fn build_chat_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
        let content_width = width.max(1);
        let _body_width = content_width.saturating_sub(2).max(1);
        let mut lines = Vec::new();

        for message in &state.messages {
            let (label, color) = match message.role {
                MessageRole::System => ("system", Color::DarkGray),
                MessageRole::User => ("user", Color::Cyan),
                MessageRole::Agent => ("agent", Color::Blue),
                MessageRole::Decision => ("decision", Color::Green),
            };

            // Render state indicator
            let state_indicator = match message.status {
                MessageStatus::Thinking => Span::styled(
                    " ◌ ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
                ),
                MessageStatus::Streaming => Span::styled(
                    " ◉ ",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK),
                ),
                MessageStatus::Completed => Span::styled(" ✓ ", Style::default().fg(Color::Green)),
                MessageStatus::Error => Span::styled(" ✗ ", Style::default().fg(Color::Red)),
            };

            lines.push(Line::from(vec![
                state_indicator,
                Span::styled(
                    format!("[{}] ", message.timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            ]));

            // Render message content with syntax highlighting
            let mut in_code_block = false;
            let mut code_lang = String::new();
            let mut code_lines: Vec<String> = Vec::new();

            for line in message.content.lines() {
                if line.trim_start().starts_with("```") {
                    if in_code_block {
                        // End of code block
                        if !code_lang.is_empty() {
                            lines.push(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(
                                    format!("{} ", code_lang),
                                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                        for code_line in code_lines.iter() {
                            lines.push(Line::from(vec![
                                Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                                Span::styled(code_line.clone(), Style::default().fg(Color::Cyan)),
                            ]));
                        }
                        lines.push(Line::from(Span::styled("  └───", Style::default().fg(Color::DarkGray))));
                        code_lines.clear();
                        code_lang.clear();
                        in_code_block = false;
                    } else {
                        // Start of code block
                        in_code_block = true;
                        code_lang = line.trim_start().trim_start_matches("```").trim().to_string();
                    }
                } else if in_code_block {
                    code_lines.push(line.to_string());
                } else {
                    // Regular text with inline code highlighting
                    let spans = Self::render_text_line(line);
                    if line.is_empty() {
                        lines.push(Line::from(String::new()));
                    } else {
                        let mut line_spans = vec![Span::raw("  ")];
                        line_spans.extend(spans);
                        lines.push(Line::from(line_spans));
                    }
                }
            }

            // Handle unclosed code block
            if in_code_block && !code_lines.is_empty() {
                if !code_lang.is_empty() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("{} ", code_lang),
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                for code_line in code_lines.iter() {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                        Span::styled(code_line.clone(), Style::default().fg(Color::Cyan)),
                    ]));
                }
                lines.push(Line::from(Span::styled("  └───", Style::default().fg(Color::DarkGray))));
            }

            lines.push(Line::from(String::new()));
        }

        if lines.is_empty() {
            lines.push(Line::from("No messages yet."));
        }

        lines
    }

    fn render_text_line(line: &str) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut remaining = line.to_string();

        while !remaining.is_empty() {
            if let Some(start) = remaining.find('`') {
                // Text before inline code
                if start > 0 {
                    spans.push(Span::styled(remaining[..start].to_string(), Style::default()));
                }

                // Find closing backtick
                if let Some(end) = remaining[start + 1..].find('`') {
                    let code = &remaining[start + 1..start + 1 + end];
                    spans.push(Span::styled(
                        format!("`{}`", code),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC),
                    ));
                    remaining = remaining[start + 2 + end..].to_string();
                } else {
                    // No closing backtick
                    spans.push(Span::styled(
                        remaining[start..].to_string(),
                        Style::default().fg(Color::Cyan),
                    ));
                    remaining.clear();
                }
            } else {
                // No more inline code
                spans.push(Span::styled(remaining.clone(), Style::default()));
                remaining.clear();
            }
        }

        spans
    }

    pub(crate) fn display_width_up_to(s: &str, char_idx: usize) -> usize {
        s.chars()
            .take(char_idx)
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
            .sum()
    }
}

pub(crate) fn format_action(action: &super::keymap::Action) -> String {
    match action {
        super::keymap::Action::Quit => "Quit the application",
        super::keymap::Action::CancelResponse => "Cancel current response",
        super::keymap::Action::ToggleStatusPanel => "Show/hide status panel",
        super::keymap::Action::MoveUp => "Move up / Previous item",
        super::keymap::Action::MoveDown => "Move down / Next item",
        super::keymap::Action::MoveLeft => "Move cursor left",
        super::keymap::Action::MoveRight => "Move cursor right",
        super::keymap::Action::ScrollUp => "Scroll chat up",
        super::keymap::Action::ScrollDown => "Scroll chat down",
        super::keymap::Action::ScrollToTop => "Scroll to top",
        super::keymap::Action::ScrollToBottom => "Scroll to bottom",
        super::keymap::Action::Confirm => "Confirm selection",
        super::keymap::Action::Cancel => "Cancel / Close dialog",
        super::keymap::Action::OpenModelPicker => "Open model picker",
        super::keymap::Action::OpenProviderDialog => "Open provider dialog",
        super::keymap::Action::SwitchPanel => "Switch panel",
        super::keymap::Action::SendMessage => "Send message",
        super::keymap::Action::TypeChar(_) => "Type character",
        super::keymap::Action::DeleteChar => "Delete character",
        super::keymap::Action::HistoryPrev => "Previous input history",
        super::keymap::Action::HistoryNext => "Next input history",
        super::keymap::Action::CommandPrev => "Previous command",
        super::keymap::Action::CommandNext => "Next command",
        super::keymap::Action::None => "",
    }
    .to_string()
}
