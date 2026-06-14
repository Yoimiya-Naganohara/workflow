pub mod chat;
pub mod chat_lines;
pub mod commands;
pub mod controller;
pub mod effect;
pub mod handler;
pub mod keymap;
pub mod popup;
pub mod render;
pub mod state;
pub mod status;
pub mod style;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

pub use self::state::AppState;
use self::state::Panel;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<AppState>>,
    chat_lines_cache: Vec<Line<'static>>,
    chat_cache_key: (usize, usize, bool, usize, Option<u8>, bool, usize),
    app_event_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::effect::AppEvent>,
    app_event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::tui::effect::AppEvent>,
}

impl Tui {
    pub fn new(state: Arc<RwLock<AppState>>) -> Result<Self> {
        enable_raw_mode().map_err(|e| {
            anyhow::anyhow!(
                "Failed to enable raw mode: {}. Are you running in an interactive terminal?",
                e
            )
        })?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        let (app_event_tx, app_event_rx) = tokio::sync::mpsc::unbounded_channel();
        Ok(Self {
            terminal,
            state,
            chat_lines_cache: Vec::new(),
            chat_cache_key: (0, 0, false, 0, None, true, 0),
            app_event_tx,
            app_event_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        {
            let mut state = self.state.write().await;
            crate::tui::controller::load_initial_state(&mut state).await;
        }

        let mut event_stream = EventStream::new();
        let mut interval = tokio::time::interval(Duration::from_millis(50));

        loop {
            tokio::select! {
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            match event {
                                Event::Key(key) if key.kind == KeyEventKind::Press => {
                                    let mut state = self.state.write().await;

                                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                                        && key.code == KeyCode::Char('c')
                                    {
                                        return Ok(());
                                    }

                                    match state.ui.panel {
                                        Panel::Chat => {
                                            if !self.handle_chat_keys(&mut state, key) {
                                                return Ok(());
                                            }
                                        }
                                    }

                                    let effects = std::mem::take(&mut state.effects);
                                    drop(state);
                                    for effect in effects {
                                        let tx = self.app_event_tx.clone();
                                        tokio::spawn(async move {
                                            crate::tui::effect::execute_effect(effect, &tx).await;
                                        });
                                    }
                                }
                                Event::Mouse(mouse) => {
                                    let mut state = self.state.write().await;
                                    match mouse.kind {
                                        MouseEventKind::ScrollDown => {
                                            state.ui.chat_scroll = state.ui.chat_scroll.saturating_add(1);
                                        }
                                        MouseEventKind::ScrollUp => {
                                            state.ui.chat_scroll = state.ui.chat_scroll.saturating_sub(1);
                                            state.ui.auto_scroll = false;
                                        }
                                        _ => {}
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(Err(e)) => { eprintln!("Event stream error: {}", e); }
                        None => return Ok(()),
                    }
                }

                Some(app_event) = self.app_event_rx.recv() => {
                    let mut state = self.state.write().await;
                    state.handle_event(app_event);
                    let effects = std::mem::take(&mut state.effects);
                    drop(state);
                    for effect in effects {
                        let tx = self.app_event_tx.clone();
                        tokio::spawn(async move {
                            crate::tui::effect::execute_effect(effect, &tx).await;
                        });
                    }
                }

                _ = interval.tick() => {}
            }

            self.draw().await?;
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    }
}
