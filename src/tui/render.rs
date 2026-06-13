//! Main UI layout — full-width chat, opencode-style status bar.

use anyhow::Result;
use ratatui::layout::{Constraint, Direction, Layout};

use super::Tui;
use crate::tui::chat::render_chat;
use crate::tui::chat_lines::build_chat_lines;
use crate::tui::status::render_status_bar;

impl Tui {
    pub(crate) async fn draw(&mut self) -> Result<()> {
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
        let chat_width = term_size.width.saturating_sub(4).max(10) as usize;

        let cache_key = (
            msg_count,
            last_content_len,
            is_streaming,
            chat_width,
            is_streaming.then_some(state.ui.think_frame),
        );
        if cache_key != self.chat_cache_key {
            self.chat_lines_cache = build_chat_lines(&state.core, chat_width, state.ui.think_frame);
            self.chat_cache_key = cache_key;
        }

        let visible_height = (term_size.height.saturating_sub(input_lines + 3)).max(1) as usize;

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

        self.terminal.draw(|f| {
            let vert_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            let chat_border = crate::tui::style::panel_chat("");
            let chat_inner = chat_border.inner(vert_chunks[0]);
            f.render_widget(chat_border, vert_chunks[0]);
            render_chat(f, chat_inner, &state, &visible_lines);

            render_status_bar(f, vert_chunks[1], &state);
        })?;

        Ok(())
    }
}
