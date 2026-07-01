pub mod agent_tree;
pub mod chat;
pub mod chat_lines;
pub mod command_tree;
pub mod commands;
pub mod controller;
pub mod effect;
pub mod handler;
pub mod keymap;
pub mod popup;
pub mod render;
pub mod runtime_bridge;
pub mod state;
pub mod status;
pub mod style;
pub mod tokenizer;

use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, EventStream, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use self::state::Panel;
pub use self::state::{AppState, Focus};
use crate::tui::chat_lines::ChatRenderOutput;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<AppState>>,
    chat_lines_cache: ChatRenderOutput,
    chat_cache_key: (usize, usize, usize, bool, usize),
    app_event_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::effect::AppEvent>,
    app_event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::tui::effect::AppEvent>,
    last_session_save: std::time::Instant,
    last_session_message_count: usize,
    needs_redraw: bool,
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
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        let (app_event_tx, app_event_rx) = tokio::sync::mpsc::unbounded_channel();
        Ok(Self {
            terminal,
            state,
            chat_lines_cache: ChatRenderOutput {
                rendered: Vec::new(),
            },
            chat_cache_key: (0, 0, 0, true, 0),
            app_event_tx,
            app_event_rx,
            last_session_save: std::time::Instant::now(),
            last_session_message_count: 0,
            needs_redraw: true,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        {
            let mut state = self.state.write().await;
            crate::tui::controller::load_initial_state(&mut state).await;
        }

        // ── Wire runtime event channel and spawn event loop + broker ──
        // Channel 1: tools → event loop (ActivateAgent)
        let (runtime_tx, event_loop_rx) = tokio::sync::mpsc::channel::<wf_core::event::RuntimeEvent>(
            wf_core::RUNTIME_CHANNEL_CAPACITY,
        );
        {
            let mut s = self.state.write().await;
            s.core.runtime_event_tx = Some(runtime_tx);
        }
        // Channel 2: event loop → broker (ChildCompleted, AggregationCompleted, etc.)
        let (broker_tx, broker_rx) = tokio::sync::mpsc::channel::<wf_core::event::RuntimeEvent>(
            wf_core::RUNTIME_CHANNEL_CAPACITY,
        );

        let state_clone = self.state.clone();
        let pool = state_clone.read().await.core.agent_pool.clone();
        let runtime = state_clone.read().await.core.runtime.clone();
        let tool_server = state_clone.read().await.core.tool_server.clone();
        let runtime_tx = state_clone.read().await.core.runtime_event_tx.clone();
        if let Some(rt) = runtime {
            tokio::spawn(async move {
                use wf_runtime::runtime::runtime_loop::RuntimeEventLoop;
                let loop_ = RuntimeEventLoop::new(
                    rt,
                    pool,
                    event_loop_rx,
                    broker_tx,
                    tool_server,
                    runtime_tx,
                )
                .await;
                loop_.run().await;
            });
        } else {
            tracing::error!("Runtime not initialized — event loop not started");
        }

        let app_tx = self.app_event_tx.clone();
        let state_for_broker = self.state.clone();
        tokio::spawn(async move {
            crate::tui::runtime_bridge::runtime_event_broker(broker_rx, app_tx, state_for_broker)
                .await;
        });

        let mut event_stream = EventStream::new();
        let mut interval = tokio::time::interval(Duration::from_millis(50));

        // Initial draw: render the TUI immediately at startup so the user
        // sees the interface without having to press a key first.
        self.draw().await?;

        loop {
            self.needs_redraw = false;

            tokio::select! {
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            match event {
                                Event::Key(key) if key.kind == KeyEventKind::Press => {
                                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                                        && key.code == KeyCode::Char('c')
                                    {
                                        self.save_session().await;
                                        return Ok(());
                                    }

                                    let mut state = self.state.write().await;
                                    let should_quit = match state.ui.panel {
                                        Panel::Chat => !self.handle_chat_keys(&mut state, key),
                                    };
                                    let effects = std::mem::take(&mut state.effects);
                                    drop(state);

                                    if should_quit {
                                        self.save_session().await;
                                        return Ok(());
                                    }

                                    for effect in effects {
                                        let tx = self.app_event_tx.clone();
                                        tokio::spawn(async move {
                                            crate::tui::effect::execute_effect(effect, &tx).await;
                                        });
                                    }
                                    self.needs_redraw = true;
                                }
                                Event::Mouse(mouse) => {
                                    let mut state = self.state.write().await;
                                    match mouse.kind {
                                        MouseEventKind::ScrollDown => {
                                            state.ui.chat_scroll = state.ui.chat_scroll.saturating_add(3);
                                        }
                                        MouseEventKind::ScrollUp => {
                                            state.ui.chat_scroll = state.ui.chat_scroll.saturating_sub(3);
                                            state.ui.auto_scroll = false;
                                        }
                                        _ => {}
                                    }
                                    self.needs_redraw = true;
                                }
                                Event::Paste(text) => {
                                    let mut state = self.state.write().await;
                                    // 无弹窗 + 输入框焦点 → paste marker；其余情况都直接插入（弹窗过滤、KeyInput 等）
                                    if state.popup_mode == crate::tui::state::PopupMode::None
                                        && state.ui.focus == crate::tui::state::Focus::Input
                                    {
                                        state.ui.pending_paste = Some(text.clone());
                                        let summary = if text.lines().count() > 1 {
                                            format!("[Pasted {} chars / {} lines]", text.chars().count(), text.lines().count())
                                        } else {
                                            format!("[Pasted {} chars]", text.chars().count())
                                        };
                                        let byte_idx = crate::tui::chat_lines::char_idx_to_byte_idx(
                                            &state.ui.input,
                                            state.ui.input_cursor,
                                        );
                                        state.ui.input.insert_str(byte_idx, &summary);
                                        state.ui.input_cursor += summary.chars().count();
                                    } else {
                                        let byte_idx = crate::tui::chat_lines::char_idx_to_byte_idx(
                                            &state.ui.input,
                                            state.ui.input_cursor,
                                        );
                                        state.ui.input.insert_str(byte_idx, &text);
                                        state.ui.input_cursor += text.chars().count();
                                    }
                                    self.needs_redraw = true;
                                }
                                _ => {}
                            }
                        }
                        Some(Err(e)) => { eprintln!("Event stream error: {}", e); }
                        None => {
                            self.save_session().await;
                            return Ok(());
                        },
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
                    self.needs_redraw = true;
                }

                _ = interval.tick() => {
                    let is_streaming = self.state.read().await.ui.active_chat_requests > 0;
                    if is_streaming {
                        self.needs_redraw = true;
                    }
                    if self.last_session_save.elapsed() >= std::time::Duration::from_secs(30) {
                        self.save_session().await;
                        self.last_session_save = std::time::Instant::now();
                    }
                }
            }

            if self.needs_redraw {
                self.draw().await?;
            }
        }
    }

    /// Save conversation messages for the next session (opencode-style).
    /// Always overwrites `session.json` (crash recovery).
    /// Only creates a new timestamped session in `sessions/` when messages have
    /// actually changed — each entry in the sessions list should represent a truly
    /// distinct conversation state, not a periodic snapshot.
    async fn save_session(&mut self) {
        let state = self.state.read().await;
        if state.core.messages.is_empty() {
            return;
        }
        let msg_count = state.core.messages.len();
        let is_new = msg_count != self.last_session_message_count;
        let _ = wf_persistence::save_session(&state.core.messages);
        if is_new {
            let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            let _ = wf_persistence::save_session_as(&ts, &state.core.messages);
            if let Some(ref prompt) = state.ui.cached_system_prompt {
                let _ =
                    wf_persistence::save_session_prompt(&ts, prompt, &state.ui.cached_prompt_role);
            }
            self.last_session_message_count = msg_count;
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // ── Graceful shutdown: consolidate fluid experiences to bedrock ──
        // This ensures all accumulated experiences (even those below the
        // high-water mark) are preserved to disk before the process exits.
        if let Ok(state) = self.state.try_read()
            && let Some(runtime) = &state.core.runtime
            && let Ok(mut rt) = runtime.try_write()
        {
            rt.consolidate_experience_pool();
            let _ = rt.flush_experience_pool();
        }

        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste,
        );
    }
}
