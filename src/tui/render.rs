//! Main UI layout — partitions the screen and delegates rendering
//! to component modules (chat, sidebar, status bar, dialogs).

use anyhow::Result;
use ratatui::layout::{Constraint, Direction, Layout};

use super::Tui;
use crate::tui::chat::render_chat;
use crate::tui::chat_lines::build_chat_lines;
use crate::tui::dialogs;
use crate::tui::sidebar::render_sidebar;
use crate::tui::status::render_status_bar;

impl Tui {
    pub(crate) async fn draw(&mut self) -> Result<()> {
        // Advance animation frame (write lock).
        {
            let mut s = self.state.write().await;
            s.ui.think_frame = s.ui.think_frame.wrapping_add(1);
        }

        let state = self.state.read().await;
        let term_size = self.terminal.size()?;

        let msg_count = state.core.messages.len();
        let is_streaming = state.ui.active_chat_requests > 0;
        let last_content_len = state.core.messages.last().map(|m| m.content.len()).unwrap_or(0);
        let input_lines = state.ui.input.lines().count().clamp(1, 5) as u16;
        let sidebar_offset = if state.ui.show_status_panel {
            crate::core::types::SIDEBAR_WIDTH as usize
        } else {
            0
        };
        let chat_width = (term_size.width.saturating_sub(sidebar_offset as u16 + 4)).max(1) as usize;

        // Content-hash cache key: msg count + last msg content length + streaming + width.
        // This catches the case where streaming completes (msg_count unchanged but content
        // length is now final) — otherwise the final message would be invisible until
        // the next user message.
        let cache_key = (msg_count, last_content_len, is_streaming, chat_width);
        if cache_key != self.chat_cache_key {
            self.chat_lines_cache = build_chat_lines(&state.core, chat_width);
            self.chat_cache_key = cache_key;
        }

        let visible_height = (term_size.height.saturating_sub(input_lines + 5)).max(1) as usize;

        let chat_scroll = if state.ui.auto_scroll {
            self.chat_lines_cache.len().saturating_sub(visible_height)
        } else {
            state.ui.chat_scroll.min(self.chat_lines_cache.len().saturating_sub(1))
        };
        let visible_lines: Vec<_> = self
            .chat_lines_cache
            .iter()
            .skip(chat_scroll)
            .take(visible_height)
            .cloned()
            .collect();

        let show_sidebar = state.ui.show_status_panel;

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
            render_chat(f, chat_area, &state, &visible_lines);
            render_status_bar(f, vert_chunks[1], &state);

            // Dialog overlay or command popup
            if let Some(dialog) = &state.active_dialog {
                dialog.render(f, vert_chunks[0], &state.core);
            } else {
                dialogs::command_popup::render_command_popup(f, chat_area, &state.ui, &state.core);
            }
        })?;

        Ok(())
    }
}
