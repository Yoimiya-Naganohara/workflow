use std::collections::HashMap;
use std::sync::Arc;

use futures::future::AbortHandle;
use tokio::sync::RwLock;

use crate::core::types::AgentId;
use crate::models::{ModelCapabilities, ModelRegistry};
use crate::provider::ProviderClient;
use crate::reflection::ReflectionConfig;
use crate::runtime::AgentRuntime;
use crate::tools::ToolServerHandle;
use crate::tui::effect::AppEvent;

// ── Shared types ──

/// Statistics about the dual-track experience pool.
#[derive(Debug, Clone, Default)]
pub struct ExperiencePoolStats {
    pub total: usize,
    pub bedrock: usize,
    pub fluid: usize,
    pub last_flush_result: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum Focus {
    Sidebar,
    Chat,
    Input,
}

#[derive(Clone, PartialEq)]
pub enum Panel {
    Chat,
}

#[derive(Clone, PartialEq)]
pub enum AppMode {
    Plan,
    Build,
}

#[derive(Clone, PartialEq)]
pub enum MessageStatus {
    Thinking,
    Streaming,
    Completed,
    Error,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    pub status: MessageStatus,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        Self {
            role: MessageRole::System,
            content: content.into(),
            timestamp: now,
            status: MessageStatus::Completed,
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Agent,
    Decision,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SelectedModel {
    pub provider_id: String,
    pub model_id: String,
    pub provider_name: String,
    pub model_name: String,
}

#[derive(Clone)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub budget: u64,
}

#[derive(Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Suspended,
    Completed,
    Failed,
}

// ── Grouped state structs ──

/// Business / domain state — persisted or long-lived.
pub struct CoreState {
    pub messages: Vec<ChatMessage>,
    pub models: ModelRegistry,
    pub configured_providers: Vec<String>,
    pub api_keys: HashMap<String, String>,
    pub provider_clients: HashMap<String, Arc<ProviderClient>>,
    pub selected_models: Vec<SelectedModel>,
    pub agents: Vec<AgentEntry>,
    pub agent_pool: Arc<RwLock<crate::agent::AgentPool>>,
    pub runtime: Option<Arc<RwLock<AgentRuntime>>>,
    pub tool_server: ToolServerHandle,
    pub responsible_agent_id: Option<AgentId>,
    pub default_role: String,
    pub reflection: ReflectionConfig,
    /// Track the last chat context for retry.
    pub last_chat_request_id: u64,
    /// Channel sender to the background [`RuntimeEventLoop`].
    /// Tools use this to dispatch async work without blocking the LLM stream.
    pub runtime_event_tx: Option<tokio::sync::mpsc::Sender<crate::runtime::RuntimeEvent>>,
    /// Saved conversation context from the previous session.
    /// Loaded on startup by `load_initial_state`, cleared on restore or new task.
    pub saved_context: Option<Vec<crate::llm::types::Message>>,
}

/// Transient UI state — reset on restart.
pub struct UiState {
    pub panel: Panel,
    pub focus: Focus,
    pub mode: AppMode,
    pub input: String,
    pub input_cursor: usize,
    pub sidebar_scroll: usize,
    pub chat_scroll: usize,
    pub auto_scroll: bool,
    pub think_frame: u8,
    pub command_popup_selection: usize,
    pub input_history: Vec<String>,
    pub input_history_idx: Option<usize>,
    pub active_chat_request_id: u64,
    pub active_chat_abort: Option<AbortHandle>,
    pub active_chat_requests: usize,
    pub budget_used: u64,
    pub budget_total: u64,
    pub permits_available: usize,
    pub permits_total: usize,
    pub context_limit: u64,
    /// Cached token counts (recalculated on message changes).
    pub cached_input_tokens: u32,
    pub cached_output_tokens: u32,
    /// Number of messages at last token recalc (to detect new messages).
    pub cached_message_count: usize,
    /// Whether the tiktoken BPE file has been downloaded.
    pub tokenizer_initialized: bool,
    /// Whether `ChatTokenUsage` events have been received in the current stream.
    /// When `true`, API‑reported token counts take precedence over local estimates.
    pub has_api_tokens: bool,
    /// When a paste is pending, stores the full text to be sent on submit.
    /// The input shows a summary marker instead of the raw text.
    pub pending_paste: Option<String>,
    pub current_plan: Option<crate::agent::plan::Plan>,

    // ── Phase 1: Agent diagnostic tree ──
    /// Bumped every time the agent tree topology or statuses change.
    /// Used by `render.rs` to decide whether to rebuild tree lines.
    pub agent_tree_version: u64,
    /// Cached tree lines from last render (avoids rebuilding every 50 ms tick).
    pub cached_tree_lines: Vec<String>,
    /// Currently selected agent index within the diagnostic tree.
    /// Used for keyboard navigation and detail popup (Phase 3).
    pub selected_agent_idx: usize,
    /// Agent IDs in display order of the diagnostic tree.
    /// Updated atomically with cached_tree_lines when version changes.
    pub tree_agent_ids: Vec<crate::core::types::AgentId>,
    /// When true, the input field is greyed out and keyboard input is
    /// discarded.  Set when the root agent transitions to AwaitingChildren.
    pub input_disabled: bool,
    /// Last known total chat lines (updated each render for scroll clamping).
    pub total_chat_lines: usize,
}

// ── AppState ──

#[derive(Default)]
pub struct AppState {
    pub ui: UiState,
    pub core: CoreState,
    pub popup_mode: PopupMode,
    pub popup_selected: usize,
    pub popup_key_provider: Option<String>,
    pub keymap: super::keymap::Keymap,
    pub pool_stats: ExperiencePoolStats,
    /// Effects queued for async execution (drained by event loop).
    pub effects: Vec<crate::tui::effect::Effect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PopupMode {
    #[default]
    None,
    Commands,
    SubCommand {
        parent: String,
        items: Vec<(String, String)>,
    },
    Providers,
    KeyInput,
    ModelPicker,
    FilePicker {
        query: String,
    },
    AgentDetail {
        agent_id: crate::core::types::AgentId,
    },
    /// Popup asking the user to type argument(s) for a command.
    /// `cmd` is the base command (e.g. `/sh`), and once Enter is pressed
    /// the full line `/sh <input>` is dispatched.
    ShellInput {
        cmd: String,
        input: String,
    },
}

impl AppState {
    /// Apply an async result to the state.
    pub fn handle_event(&mut self, event: AppEvent) {
        use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
        match event {
            AppEvent::ModelRegistryFetched { count } => {
                if let Some(cached) = crate::persistence::load_provider_cache() {
                    self.core.models = cached;
                }
                self.core
                    .messages
                    .push(ChatMessage::system(format!("Loaded {} providers", count)));
            }
            AppEvent::ModelRegistryFailed { error, is_empty } => {
                let status = if is_empty {
                    MessageStatus::Error
                } else {
                    MessageStatus::Completed
                };
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: error,
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status,
                });
            }
            AppEvent::ShellOutput { content, timestamp } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    timestamp,
                    status: MessageStatus::Completed,
                });
            }
            AppEvent::ShellError { error, timestamp } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: error,
                    timestamp,
                    status: MessageStatus::Error,
                });
            }
            AppEvent::PoolQueryResult {
                content,
                timestamp,
                is_error,
            } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    timestamp,
                    status: if is_error {
                        MessageStatus::Error
                    } else {
                        MessageStatus::Completed
                    },
                });
            }
            AppEvent::ChatToken { response_index, text } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        let was_thinking = msg.status == MessageStatus::Thinking;
                        msg.content.push_str(&text);
                        msg.status = MessageStatus::Streaming;
                        // On first token, recalc local estimate only if no API token
                        // data has arrived yet.  Once ChatTokenUsage events are flowing,
                        // those per-tick accumulations are more accurate and cheaper.
                        if was_thinking && !self.ui.has_api_tokens {
                            self.recalc_tokens();
                        }
                    }
                }
            }
            AppEvent::ChatCompleted {
                response_index,
                request_id,
                full_response,
                input,
                runtime,
            } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        // Only overwrite if the message hasn't already been completed.
                        // Prevents duplicate events from overwriting completed content.
                        if matches!(msg.status, MessageStatus::Thinking | MessageStatus::Streaming) {
                            msg.content = full_response.clone();
                        }
                        msg.status = MessageStatus::Completed;
                    }
                }
                if self.ui.active_chat_requests > 0 {
                    self.ui.active_chat_requests -= 1;
                }
                self.ui.active_chat_abort = None;

                // ── Trigger self-check reflection if enabled ──
                if self.core.reflection.auto_reflect {
                    self.trigger_self_check(response_index, request_id, full_response.clone(), &input, &runtime);
                }

                // ── Save exchange to responsible agent's context ──
                if let Some(agent_id) = self.core.responsible_agent_id {
                    if let Ok(mut pool) = self.core.agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.context.push(crate::llm::types::Message {
                                role: "user".to_string(),
                                content: input.clone(),
                            });
                            agent.context.push(crate::llm::types::Message {
                                role: "assistant".to_string(),
                                content: full_response.clone(),
                            });
                            // Prevent unbounded growth: keep last 100 exchanges
                            if agent.context.len() > 200 {
                                agent.context.drain(0..agent.context.len() - 200);
                            }
                            agent.last_active_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                        }
                    }
                }

                // ── Sync budget from runtime ──
                if let Some(rt) = &runtime {
                    if let Ok(r) = rt.try_read() {
                        let remaining = r.remaining_budget() as u64;
                        self.ui.budget_used = self.ui.budget_total.saturating_sub(remaining);
                    }
                }

                // ── Recalculate token cache (only if no API tokens received) ──
                if !self.ui.has_api_tokens {
                    self.recalc_tokens();
                }
            }
            AppEvent::ChatError {
                response_index,
                request_id: _,
                error,
            } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content = error;
                        msg.status = MessageStatus::Error;
                    }
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
                self.recalc_tokens();
            }
            AppEvent::ChatCancelled {
                response_index,
                request_id: _,
            } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content += " (cancelled)";
                        msg.status = MessageStatus::Completed;
                    }
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
                self.recalc_tokens();
            }
            AppEvent::ChatTokenUsage {
                response_index: _,
                input,
                output,
            } => {
                // On first API token report, clear the local‑estimate baseline
                // so we only accumulate actual API‑reported tokens.
                if !self.ui.has_api_tokens {
                    self.ui.cached_input_tokens = 0;
                    self.ui.cached_output_tokens = 0;
                    self.ui.has_api_tokens = true;
                }
                // API reports cumulative token totals — use max() to capture the
                // latest cumulative value without double-counting.
                self.ui.cached_input_tokens = self.ui.cached_input_tokens.max(input);
                self.ui.cached_output_tokens = self.ui.cached_output_tokens.max(output);
            }
            AppEvent::OptimizationResult {
                role_name,
                original: _,
                improved,
                summary,
                stats: _,
            } => {
                // Auto-apply: update system_prompt, increment version, recompute embedding.
                if let Some(runtime) = &self.core.runtime {
                    if let Ok(rt) = runtime.try_read() {
                        if let Some(mut tpl) = rt.get_role_template(&role_name) {
                            tpl.system_prompt = improved.clone();
                            tpl.version += 1;
                            tpl.updated_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            rt.save_role_template(tpl);
                            // Recompute embedding in background.
                            rt.compute_role_embeddings_async();
                        }
                    }
                }
                self.core.messages.push(ChatMessage::system(format!(
                    "Role '{}' optimization complete.\n\n{}  \n\nNew prompt applied (version ++).",
                    role_name, summary
                )));
            }
            AppEvent::OptimizationError { role_name, error } => {
                self.core.messages.push(ChatMessage::system(format!(
                    "Role '{}' optimization failed: {}",
                    role_name, error
                )));
            }
            AppEvent::ChatToolCall {
                response_index,
                name,
                args,
                timestamp: _,
            } => {
                // Tool call: "name — args" format.
                // Truncate long args at a safe char boundary (preserve original newlines).
                let args_trunc = if args.len() > 200 {
                    let end = args.char_indices().nth(197).map(|(i, _)| i).unwrap_or(args.len());
                    format!("{}…", &args[..end])
                } else {
                    args
                };
                let line = if args_trunc.is_empty() {
                    name.clone()
                } else {
                    format!("{} — {}", name, args_trunc)
                };

                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        if !msg.content.is_empty() {
                            msg.content.push('\n');
                        }
                        msg.content.push_str(&line);
                    }
                } else {
                    self.core.messages.push(ChatMessage {
                        role: MessageRole::Decision,
                        content: line,
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    });
                }
                self.recalc_tokens();
            }
            AppEvent::SelfCheckResult {
                response_index: _,
                request_id,
                passed,
                attempt,
                input: _,
                system_prompt,
                history,
                tool_server,
                provider,
                model_id,
                runtime,
                abort_registration: _,
                feedback,
            } => {
                if passed {
                    self.core.last_chat_request_id = 0;
                    return;
                }

                let max_attempts = self.core.reflection.max_attempts;
                if attempt >= max_attempts {
                    self.core.messages.push(ChatMessage::system(format!(
                        "⚠️ Reflection: max retries ({}) reached. Final result shown above.",
                        max_attempts
                    )));
                    self.core.last_chat_request_id = 0;
                    return;
                }

                let new_attempt = attempt + 1;
                let now = chrono::Local::now().format("%H:%M:%S").to_string();
                let feedback_msg = format!("🔄 Reflection #{} — revisiting response\n\n{}", new_attempt, feedback);
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: feedback_msg,
                    timestamp: now,
                    status: MessageStatus::Completed,
                });

                let mut new_history = history.clone();
                // Reset API token tracking for the retry stream
                self.ui.has_api_tokens = false;
                new_history.push(("user".to_string(), feedback.clone()));

                let new_response_index = self.core.messages.len();
                self.core.messages.push(ChatMessage {
                    role: MessageRole::Agent,
                    content: String::new(),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Thinking,
                });

                let new_request_id = self.ui.active_chat_request_id.wrapping_add(1);
                self.ui.active_chat_request_id = new_request_id;
                self.ui.active_chat_requests = 1;
                self.ui.auto_scroll = true;

                let (abort_handle, new_abort_registration) = futures::future::AbortHandle::new_pair();
                self.ui.active_chat_abort = Some(abort_handle);

                self.core.last_chat_request_id = request_id;

                let new_history_clone = new_history.clone();
                self.effects.push(crate::tui::effect::Effect::StartChat {
                    input: feedback.clone(),
                    response_index: new_response_index,
                    request_id: new_request_id,
                    model_id,
                    system_prompt,
                    history: new_history_clone,
                    tool_server,
                    provider,
                    runtime,
                    abort_registration: new_abort_registration,
                });
            }
            AppEvent::SystemLog { content } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Completed,
                });
            }
            AppEvent::AggregationStarting { agent_id: _ } => {
                self.core.messages.push(ChatMessage::system(
                    "All sub-tasks completed. Synthesising final result…",
                ));
            }
        }
    }

    /// Recalculate cached token counts from all messages.
    /// Uses the tiktoken-based tokenizer; falls back to char/4 estimate.
    pub fn recalc_tokens(&mut self) {
        use crate::tui::tokenizer;
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        for msg in &self.core.messages {
            let tokens = tokenizer::count_tokens(&msg.content);
            match msg.role {
                MessageRole::User | MessageRole::System => {
                    input_tokens = input_tokens.saturating_add(tokens);
                }
                MessageRole::Agent | MessageRole::Decision => {
                    output_tokens = output_tokens.saturating_add(tokens);
                }
            }
        }
        self.ui.cached_input_tokens = input_tokens;
        self.ui.cached_output_tokens = output_tokens;
        self.ui.cached_message_count = self.core.messages.len();
        self.ui.tokenizer_initialized = tokenizer::is_initialised();
    }

    /// Get capabilities for the currently selected model (if any).
    pub fn model_capabilities(&self) -> Option<ModelCapabilities> {
        let sel = self.core.selected_models.first()?;
        let model = self.core.models.get_model(&sel.provider_id, &sel.model_id)?;
        Some(model.capabilities())
    }

    /// Trigger a self-check reflection after a completed chat.
    fn trigger_self_check(
        &mut self,
        response_index: usize,
        request_id: u64,
        full_response: String,
        input: &str,
        runtime: &Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
    ) {
        // Don't reflect on reflection-triggered retries
        if request_id > 0 && request_id == self.core.last_chat_request_id {
            return;
        }

        // Get provider and model_id from the current selection
        let (provider, model_id, system_prompt) = self.get_chat_context(runtime, input);
        let system_prompt = format!(
            "{}\n\n{}\n\n{}",
            system_prompt,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
        );

        let provider = match provider {
            Some(p) => p,
            None => return,
        };

        let tool_server = self.core.tool_server.clone();

        // Build history from messages up to the response
        let mut history: Vec<(String, String)> = Vec::new();
        for (i, msg) in self.core.messages.iter().enumerate() {
            if i >= response_index.saturating_sub(1) {
                break;
            }
            match msg.role {
                MessageRole::User => history.push(("user".to_string(), msg.content.clone())),
                MessageRole::Agent => history.push(("assistant".to_string(), msg.content.clone())),
                _ => {}
            }
        }

        let (abort_handle, abort_registration) = futures::future::AbortHandle::new_pair();
        self.ui.active_chat_abort = Some(abort_handle);

        self.effects.push(crate::tui::effect::Effect::SelfCheck {
            response_index,
            request_id,
            attempt: 0,
            full_response,
            input: input.to_string(),
            system_prompt,
            history,
            tool_server,
            provider,
            model_id,
            runtime: runtime.clone(),
            abort_registration,
        });
    }

    /// Get the chat context (provider, model_id, system_prompt) from current state.
    fn get_chat_context(
        &self,
        _runtime: &Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
        _input: &str,
    ) -> (Option<std::sync::Arc<crate::llm::LlmProvider>>, String, String) {
        let default_tool_prompt = "Must follow user instructions and use available tools. Remember preferences";

        let provider = self
            .core
            .runtime
            .as_ref()
            .and_then(|rt| rt.try_read().ok().and_then(|r| r.provider.clone()));

        let model_id = provider
            .as_ref()
            .and_then(|_| {
                self.core
                    .runtime
                    .as_ref()
                    .and_then(|rt| rt.try_read().ok().map(|r| r.model_id.clone()))
            })
            .unwrap_or_default();

        // Fallback: build from selected_models if runtime provider unavailable
        let provider = provider.or_else(|| {
            self.core.selected_models.first().and_then(|sel| {
                if self.core.configured_providers.iter().any(|id| id == &sel.provider_id) {
                    // Try to get existing client
                    self.core
                        .provider_clients
                        .get(&sel.provider_id)
                        .map(|pc| Arc::new(pc.inner.clone()))
                } else {
                    None
                }
            })
        });

        let agent_prompt = self
            .core
            .responsible_agent_id
            .as_ref()
            .and_then(|aid| {
                let pool = self.core.agent_pool.try_read().ok()?;
                let role = pool.get_agent(aid)?.role.clone();
                drop(pool);
                let rt = self.core.runtime.as_ref()?.try_read().ok()?;
                rt.get_role_template(&role).map(|t| t.system_prompt.clone())
            })
            .unwrap_or_else(|| default_tool_prompt.to_string());

        (provider, model_id, agent_prompt)
    }
}

/// Find the slot index of a streaming message.
/// Returns `None` if no streaming/thinking slot is found at the preferred index
/// or elsewhere, to prevent overwriting already-completed messages.
fn find_streaming_slot_response(messages: &[ChatMessage], preferred: usize) -> Option<usize> {
    // 1. Prefer the exact index if it's still in streaming/thinking state
    if messages
        .get(preferred)
        .is_some_and(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
    {
        return Some(preferred);
    }
    // 2. Fall back to the last streaming/thinking message anywhere
    messages
        .iter()
        .rposition(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
    // 3. Return None — don't overwrite completed messages
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            messages: vec![ChatMessage::system(
                "Workflow Agent — connected. Use `/connect` to configure a provider, then `/models` to add models to your pool.",
            )],
            models: ModelRegistry::new(),
            configured_providers: Vec::new(),
            api_keys: HashMap::new(),
            provider_clients: HashMap::new(),
            // provider_clients populated by load_initial_state
            selected_models: Vec::new(),
            agents: vec![AgentEntry {
                id: "agent-000".to_string(),
                name: "Planning Agent".to_string(),
                status: AgentStatus::Running,
                budget: 0,
            }],
            agent_pool: Arc::new(RwLock::new(crate::agent::AgentPool::new())),
            runtime: None,
            tool_server: crate::tools::create_tool_server(),
            default_role: "general_business_analyst".to_string(),
            responsible_agent_id: None,
            reflection: ReflectionConfig::default(),
            last_chat_request_id: 0,
            runtime_event_tx: None,
            saved_context: None,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            panel: Panel::Chat,
            focus: Focus::Input,
            mode: AppMode::Plan,
            input: String::new(),
            input_cursor: 0,
            sidebar_scroll: 0,
            chat_scroll: 0,
            auto_scroll: true,
            think_frame: 0,
            command_popup_selection: 0,
            input_history: Vec::new(),
            input_history_idx: None,
            active_chat_request_id: 0,
            cached_input_tokens: 0,
            cached_output_tokens: 0,
            cached_message_count: 0,
            tokenizer_initialized: false,
            has_api_tokens: false,
            pending_paste: None,
            active_chat_abort: None,
            active_chat_requests: 0,
            budget_used: 0,
            budget_total: 10000,
            permits_available: 10,
            permits_total: 10,
            context_limit: 0,
            current_plan: None,
            agent_tree_version: 0,
            cached_tree_lines: Vec::new(),
            selected_agent_idx: 0,
            tree_agent_ids: Vec::new(),
            input_disabled: false,
            total_chat_lines: 0,
            pending_shell_cmd: false,
        }
    }
}
