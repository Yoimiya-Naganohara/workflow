//! Main UI rendering — layout, status bar.

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::Tui;
use super::state::{AppMode, AppState, Focus};
use crate::tui::chat_lines::build_chat_lines;
use crate::tui::dialogs;
use crate::tui::sidebar::render_sidebar;

impl Tui {
    pub(crate) async fn draw(&mut self) -> Result<()> {
        let state = self.state.read().await;

        let term_size = self.terminal.size()?;
        let chat_width = (term_size.width.saturating_sub(4)).max(1) as usize;
        let msg_count = state.messages.len();
        let is_streaming = state.active_chat_requests > 0;

        if is_streaming || msg_count != self.chat_cache_msg_count || chat_width != self.chat_cache_width {
            self.chat_lines_cache = build_chat_lines(&state, chat_width);
            self.chat_cache_msg_count = msg_count;
            self.chat_cache_width = chat_width;
        }

        // Pre-compute chat scroll to avoid borrow conflict inside closure
        let chat_scroll = state.chat_scroll.min(self.chat_lines_cache.len().saturating_sub(1));
        let visible_lines: Vec<_> = self
            .chat_lines_cache
            .iter()
            .skip(chat_scroll)
            .take((term_size.height.saturating_sub(4)).max(1) as usize)
            .cloned()
            .collect();

        self.terminal.draw(|f| {
            let vert_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            let main_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(crate::core::types::SIDEBAR_WIDTH),
                    Constraint::Min(0),
                ])
                .split(vert_chunks[0]);

            render_sidebar(f, main_chunks[0], &state);

            let chat_area = main_chunks[1];
            Self::render_chat(f, chat_area, &state, &visible_lines);
            Self::render_status_bar(f, vert_chunks[1], &state);

            if state.show_provider_dialog {
                dialogs::render_provider_dialog(f, vert_chunks[0], &state);
            } else if state.show_key_dialog {
                dialogs::render_key_dialog(f, vert_chunks[0], &state);
            } else if state.show_model_picker {
                dialogs::render_model_picker(f, vert_chunks[0], &state);
            } else {
                dialogs::render_command_popup(f, chat_area, &state);
            }
        })?;

        Ok(())
    }

    fn render_chat(f: &mut Frame, area: Rect, state: &AppState, visible_lines: &[ratatui::text::Line<'static>]) {
        let chunks = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Min(0),
                ratatui::layout::Constraint::Length(3),
            ])
            .split(area);

        // Chat message area
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Chat ")
            .style(Style::default().fg(Color::Blue));
        let inner = block.inner(chunks[0]);
        f.render_widget(block, chunks[0]);
        f.render_widget(
            ratatui::widgets::Paragraph::new(ratatui::text::Text::from(visible_lines.to_vec())),
            inner,
        );

        // Input box
        let input_style = if state.focus == crate::tui::state::Focus::Input {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let input_text = if state.input.is_empty() {
            "Type a message or /command..."
        } else {
            state.input.as_str()
        };
        let input_display_style = if state.input.is_empty() {
            input_style.fg(Color::DarkGray)
        } else {
            input_style
        };
        f.render_widget(
            ratatui::widgets::Paragraph::new(input_text)
                .style(input_display_style)
                .block(Block::default().borders(Borders::ALL)),
            chunks[1],
        );

        // Cursor
        if state.focus == crate::tui::state::Focus::Input {
            let prefix_width = crate::tui::chat_lines::display_width_up_to(&state.input, state.input_cursor);
            let cursor_x = chunks[1].x + prefix_width as u16 + 1;
            let cursor_y = chunks[1].y + 1;
            f.set_cursor_position((cursor_x.min(chunks[1].right().saturating_sub(1)), cursor_y));
        }
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

        let hint = if state.show_provider_dialog || state.show_key_dialog {
            "Esc cancel · Enter confirm".to_string()
        } else if state.show_model_picker {
            "Esc close · Enter toggle · Ctrl+A providers".to_string()
        } else if state.focus == Focus::Chat {
            "Up/Down scroll · Ctrl+P models · Ctrl+C quit".to_string()
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
}
