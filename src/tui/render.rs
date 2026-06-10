//! Main UI rendering — layout, status bar.
//!
//! Auto-scrolls to bottom during streaming, drives thinking frame
//! animation, and renders chat with word-wrap enabled.

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::Tui;
use super::state::{AppState, Focus};
use crate::tui::chat_lines::{build_chat_lines, char_idx_to_byte_idx};
use crate::tui::dialogs;
use crate::tui::sidebar::render_sidebar;

/// Rough token estimation: ~4 chars per token (standard heuristic).
fn estimate_tokens(char_count: usize) -> usize {
    char_count / 4 + 1
}

impl Tui {
    pub(crate) async fn draw(&mut self) -> Result<()> {
        // Advance animation frame & auto-scroll (write lock).
        {
            let mut s = self.state.write().await;
            s.think_frame = s.think_frame.wrapping_add(1);

            // Auto-scroll to bottom when streaming and user hasn't scrolled up.
            if s.active_chat_requests > 0 && s.auto_scroll {
                let term_h = self.terminal.size().ok().map_or(20, |ts| ts.height);
                let input_lines = s.input.lines().count().clamp(1, 5) as u16;
                let visible_height = (term_h.saturating_sub(input_lines + 5)).max(1) as usize;
                let cache_len = self.chat_lines_cache.len();
                let max_scroll = cache_len.saturating_sub(visible_height);
                if s.chat_scroll < max_scroll {
                    s.chat_scroll = max_scroll;
                }
            }
        }

        let state = self.state.read().await;
        let term_size = self.terminal.size()?;
        let msg_count = state.messages.len();
        let is_streaming = state.active_chat_requests > 0;
        let input_lines = state.input.lines().count().clamp(1, 5) as u16;
        let sidebar_offset = if state.show_status_panel {
            crate::core::types::SIDEBAR_WIDTH as usize
        } else {
            0
        };
        // Content area: screen width - sidebar(if shown) - left padding(2) - block borders(2)
        let chat_width = (term_size.width.saturating_sub(sidebar_offset as u16 + 4)).max(1) as usize;

        // Rebuild cache when messages or streaming state change.
        if is_streaming || msg_count != self.chat_cache_msg_count || chat_width != self.chat_cache_width {
            self.chat_lines_cache = build_chat_lines(&state, chat_width);
            self.chat_cache_msg_count = msg_count;
            self.chat_cache_width = chat_width;
        }

        // Available lines for chat messages: total height - status bar(1) - input box(height + 2 borders) - chat borders(2)
        let visible_height = (term_size.height.saturating_sub(input_lines + 5)).max(1) as usize;
        let chat_scroll = state.chat_scroll.min(self.chat_lines_cache.len().saturating_sub(1));
        let visible_lines: Vec<_> = self
            .chat_lines_cache
            .iter()
            .skip(chat_scroll)
            .take(visible_height)
            .cloned()
            .collect();

        let show_provider = state.show_provider_dialog;
        let show_key = state.show_key_dialog;
        let show_picker = state.show_model_picker;
        let show_sidebar = state.show_status_panel;

        self.terminal.draw(|f| {
            let vert_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            let main_chunks = if show_sidebar {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(crate::core::types::SIDEBAR_WIDTH),
                        Constraint::Min(0),
                    ])
                    .split(vert_chunks[0])
            } else {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(0)])
                    .split(vert_chunks[0])
            };

            if show_sidebar {
                render_sidebar(f, main_chunks[0], &state);
            }

            let chat_area = if show_sidebar { main_chunks[1] } else { main_chunks[0] };
            Self::render_chat(f, chat_area, &state, &visible_lines);
            Self::render_status_bar(f, vert_chunks[1], &state);

            if state.show_custom_dialog {
                dialogs::render_custom_provider_dialog(f, vert_chunks[0], &state);
            } else if show_provider {
                dialogs::render_provider_dialog(f, vert_chunks[0], &state);
            } else if show_key {
                dialogs::render_key_dialog(f, vert_chunks[0], &state);
            } else if show_picker {
                dialogs::render_model_picker(f, vert_chunks[0], &state);
            } else {
                dialogs::render_command_popup(f, chat_area, &state);
            }
        })?;

        Ok(())
    }

    fn render_chat(f: &mut Frame, area: Rect, state: &AppState, visible_lines: &[Line<'static>]) {
        // Dynamic input height: grows for multi-line input (max 5 lines).
        let input_lines = state.input.lines().count().clamp(1, 5) as u16;
        let input_height = input_lines + 2; // borders

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(input_height)])
            .split(area);

        // ── Chat messages with word wrap ──
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Chat ")
            .style(Style::default().fg(Color::Blue));
        let inner = block.inner(chunks[0]);
        f.render_widget(block, chunks[0]);
        f.render_widget(
            Paragraph::new(ratatui::text::Text::from(visible_lines.to_vec())).wrap(Wrap { trim: false }),
            inner,
        );

        // ── Input box ──
        let input_style = if state.focus == Focus::Input {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let placeholder = "Type a message or /command… (Alt+Enter newline)";
        let input_display = if state.input.is_empty() {
            Paragraph::new(placeholder).style(input_style.fg(Color::DarkGray))
        } else {
            Paragraph::new(state.input.as_str()).style(input_style)
        };

        f.render_widget(
            input_display
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );

        // ── Cursor ──
        if state.focus == Focus::Input && !state.input.is_empty() {
            let prefix_width = crate::tui::chat_lines::display_width_up_to(&state.input, state.input_cursor);
            let cursor_x = chunks[1].x + prefix_width as u16 + 1;
            // Place cursor on the correct visual line.
            let line_no = state.input[..char_idx_to_byte_idx(&state.input, state.input_cursor)]
                .lines()
                .count()
                .saturating_sub(1);
            let cursor_y = chunks[1].y + 1 + line_no as u16;
            f.set_cursor_position((
                cursor_x.min(chunks[1].right().saturating_sub(1)),
                cursor_y.min(chunks[1].bottom().saturating_sub(1)),
            ));
        } else if state.focus == Focus::Input {
            f.set_cursor_position((chunks[1].x + 1, chunks[1].y + 1));
        }
    }

    fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
        let hint = if state.show_custom_dialog {
            "Enter continue · Esc cancel".to_string()
        } else if state.show_provider_dialog || state.show_key_dialog {
            "Esc cancel · Enter confirm".to_string()
        } else if state.show_model_picker {
            "Esc close · Enter toggle · Ctrl+A providers".to_string()
        } else if state.focus == Focus::Chat {
            "↑↓ scroll · g top · G bottom · Ctrl+P models · Ctrl+C quit".to_string()
        } else if state.active_chat_requests > 0 {
            "Ctrl+X stop · Ctrl+C quit".to_string()
        } else {
            "Enter send · Alt+Enter newline · ↑↓ history · /cmd · Ctrl+P models · Tab sidebar · Ctrl+C quit".to_string()
        };

        let total_chars: usize = state.messages.iter().map(|m| m.content.len()).sum();
        let ctx_tokens = estimate_tokens(total_chars);
        let ctx_info = if state.context_limit > 0 {
            format!("ctx: {:>4}/{}", ctx_tokens, state.context_limit)
        } else {
            format!("ctx: {:>4}", ctx_tokens)
        };

        let hint_width = hint.len() as u16;
        let ctx_width = ctx_info.len() as u16;
        let pad = area.width.saturating_sub(hint_width + ctx_width + 2);

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
                Span::raw(" ".repeat(pad as usize)),
                Span::styled(ctx_info, Style::default().fg(Color::Green)),
            ])),
            area,
        );
    }
}
