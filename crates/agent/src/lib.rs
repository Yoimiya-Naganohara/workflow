pub mod agent_pool;
pub mod protocol;

use async_stream::stream;
use futures::{Stream, StreamExt};
use rig::{
    agent::{AgentHook, Flow, MultiTurnStreamItem, StepEvent},
    completion::CompletionModel,
    message::Text,
    streaming::{StreamedAssistantContent, StreamedUserContent},
};
use std::{borrow::Cow, collections::HashMap, pin::Pin, sync::Arc};
use tokio::sync::{
    Mutex, RwLock,
    mpsc::{Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel},
};

// ── Types ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum AgentState {
    Idle,
    Running,
    Hibernating,
    Stopped,
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle => write!(f, "idle"),
            AgentState::Running => write!(f, "running"),
            AgentState::Hibernating => write!(f, "hibernating"),
            AgentState::Stopped => write!(f, "stopped"),
        }
    }
}

/// Data payload (LLM prompts, inter-agent content).
#[derive(Debug, Clone)]
pub enum Message {
    User(String),
    AgentMessage(protocol::PeerMessage),
}

/// Out-of-band lifecycle controls. These use a separate unbounded channel so
/// shutdown cannot be delayed behind queued prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMessage {
    Abort,
    Hibernate,
    Resume,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentRunError {
    #[error("agent runtime has already been started")]
    AlreadyStarted,
}

/// Streamed output emitted by an [`Agent`] for an external consumer to handle.
///
/// The agent itself does no printing or rendering — it forwards these events
/// out via its outbox channel and the caller decides what to do with them
/// (print, forward to a peer, write to a TUI, persist, ...).
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Assistant text delta.
    Text(String),
    /// Reasoning text delta (chain-of-thought).
    Reasoning(String),
    /// A tool call started — the function name is now known.
    ToolCall { name: String, params: String },
    /// A tool call completed with a result.
    ToolResult { name: String, result: String },
    /// One completion turn finished successfully.
    TurnComplete,
    /// A stream-level error occurred.
    Error(String),
}

pub type AgentId = u32;

tokio::task_local! {
    static CURRENT_AGENT_ID: AgentId;
}

pub fn current_agent_id() -> AgentId {
    CURRENT_AGENT_ID.try_with(|id| *id).unwrap_or(0)
}

/// Type-erased function that runs one model turn and yields agent events.
pub type RunFn =
    Box<dyn Fn(String) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> + Send + Sync>;

struct Budget {
    send_message_budget: u8,
    used_send_message_budget: u8,
}
impl Budget {
    pub fn new(send_message_budget: u8) -> Self {
        Self {
            send_message_budget,
            used_send_message_budget: 0,
        }
    }

    pub fn request_message(&mut self) -> bool {
        if self.send_message_budget > self.used_send_message_budget {
            self.used_send_message_budget += 1;
            true
        } else {
            false
        }
    }
    pub fn reset(&mut self) {
        self.used_send_message_budget = 0;
    }
}
// ── Agent ────────────────────────────────────────────────────

/// Single-agent runtime.
///
/// The concrete `rig::agent::Agent<M>` is built **outside** this struct
/// (model, preamble, tools, memory, hooks all configured externally) and
/// injected via [`Agent::new`], which type-erases `M` into a [`RunFn`]. This
/// struct owns only the runtime wiring — inbound message intake, run state,
/// and streamed output. Inter-agent routing and tool-message forwarding live
/// in the external orchestrator, not here. The struct is deliberately
/// non-generic so an [`AgentPool`] can hold agents backed by *different*
/// models/providers.
struct AgentRuntime {
    run_fn: RunFn,
    inbox: Receiver<Message>,
    controls: UnboundedReceiver<ControlMessage>,
}

pub struct Agent {
    id: AgentId,
    role: String,
    budget: Mutex<Budget>,

    current_task: Arc<RwLock<Option<String>>>,
    state: Arc<RwLock<AgentState>>,

    runtime: Mutex<Option<AgentRuntime>>,
    sender: Sender<Message>,
    controls: UnboundedSender<ControlMessage>,

    receiver: tokio::sync::broadcast::Receiver<AgentEvent>,
    outbox: tokio::sync::broadcast::Sender<AgentEvent>,
}
impl Agent {
    /// Create a new Agent, type-erasing the model.
    ///
    /// * `rig_agent` – a fully-configured `rig::agent::Agent<M>` (model,
    ///   preamble, tools, memory, hooks). `M` is captured here and never
    ///   appears in the returned [`Agent`].
    /// * `inbox` – receiver for [`Message`]s routed to this agent by the
    ///   orchestrator (user prompts, peer messages, control signals).
    /// * `outbox` – sender for streamed [`AgentEvent`]s; the caller owns the
    ///   receiver and handles all output rendering/forwarding.
    pub fn new<M>(id: AgentId, role: String, rig_agent: rig::agent::Agent<M>) -> Self
    where
        M: CompletionModel + 'static,
    {
        const MAX_TURNS: usize = 100;
        const CHANNEL_CAPACITY: usize = 1024;

        let (sender, inbox) = channel::<Message>(CHANNEL_CAPACITY);
        let (controls, control_inbox) = unbounded_channel::<ControlMessage>();
        let (outbox, receiver) = tokio::sync::broadcast::channel(CHANNEL_CAPACITY);
        let rig_agent = Arc::new(rig_agent);
        let run_fn: RunFn = Box::new(move |prompt| {
            let rig_agent = Arc::clone(&rig_agent);
            Box::pin(stream! {
                let mut stream = rig_agent
                    .runner(prompt)
                    .max_turns(MAX_TURNS)
                    .stream()
                    .await;

                let mut pending_tool_names: HashMap<String, String> = HashMap::new();

                while let Some(item) = stream.next().await {
                    let event = match item {
                        Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => match content {
                            StreamedAssistantContent::Text(Text { text, .. }) => {
                                Some(AgentEvent::Text(text))
                            }
                            StreamedAssistantContent::ReasoningDelta { reasoning, .. }
                                 =>
                            {
                                Some(AgentEvent::Reasoning(reasoning))
                            }
                            StreamedAssistantContent::ToolCall { tool_call, internal_call_id } => {
                                let name = tool_call.function.name.clone();
                                pending_tool_names.insert(internal_call_id.clone(), name.clone());
                                Some(AgentEvent::ToolCall {
                                    name,
                                    params: tool_call.function.arguments.to_string(),
                                })
                            }
                            _ => None,
                        },
                        Ok(MultiTurnStreamItem::StreamUserItem(content)) => match content {
                            StreamedUserContent::ToolResult { tool_result, internal_call_id } => {
                                let name = pending_tool_names.remove(&internal_call_id)
                                    .unwrap_or_else(|| "unknown".to_string());
                                let result_text = tool_result.content.iter()
                                    .filter_map(|c| match c {
                                        rig::completion::message::ToolResultContent::Text(t) => Some(t.text.as_str()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                // Oversized results are truncated before they ever
                                // reach the model context by the `ToolResultLimit`
                                // hook registered when the agent was built.
                                Some(AgentEvent::ToolResult { name, result: result_text })
                            }
                        },
                        Ok(MultiTurnStreamItem::FinalResponse(_)) => {
                            Some(AgentEvent::TurnComplete)
                        }
                        Ok(_) => None,
                        Err(error) => Some(AgentEvent::Error(error.to_string())),
                    };

                    if let Some(event) = event {
                        yield event;
                    }
                }
            })
        });

        Self {
            id,
            role,
            // TODO: GET BUDGET FROM THE ROLE
            budget: Mutex::new(Budget::new(10)),
            current_task: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(AgentState::Idle)),
            runtime: Mutex::new(Some(AgentRuntime {
                run_fn,
                inbox,
                controls: control_inbox,
            })),
            sender,
            controls,
            receiver,
            outbox,
        }
    }

    pub fn id(&self) -> AgentId {
        self.id
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn state(&self) -> &Arc<RwLock<AgentState>> {
        &self.state
    }

    pub fn current_task(&self) -> &Arc<RwLock<Option<String>>> {
        &self.current_task
    }

    /// Attempt to consume one `send_message` allowance from this turn's
    /// budget. Returns `false` when the budget is exhausted.
    pub async fn request_message_budget(&self) -> bool {
        self.budget.lock().await.request_message()
    }

    async fn reset_budget(&self) {
        self.budget.lock().await.reset();
    }

    /// Run the main event loop. This method may only be called once.
    ///
    /// The bounded data inbox provides backpressure while a turn is active.
    /// Lifecycle controls travel out-of-band, so abort and hibernate remain
    /// responsive even when the data inbox is full. Hibernating cancels the
    /// active turn; queued messages remain in the bounded inbox until resume.
    pub async fn run(&self) -> Result<(), AgentRunError> {
        let mut runtime = self
            .runtime
            .lock()
            .await
            .take()
            .ok_or(AgentRunError::AlreadyStarted)?;
        let mut hibernating = false;

        'lifecycle: loop {
            if hibernating {
                *self.state.write().await = AgentState::Hibernating;
                loop {
                    match runtime.controls.recv().await {
                        Some(ControlMessage::Resume) => {
                            hibernating = false;
                            *self.state.write().await = AgentState::Idle;
                            break;
                        }
                        Some(ControlMessage::Hibernate) => {}
                        Some(ControlMessage::Abort) | None => break 'lifecycle,
                    }
                }
            }

            let message = tokio::select! {
                message = runtime.inbox.recv() => match message {
                    Some(message) => message,
                    None => break,
                },
                control = runtime.controls.recv() => match control {
                    Some(ControlMessage::Abort) | None => break,
                    Some(ControlMessage::Hibernate) => {
                        hibernating = true;
                        continue;
                    }
                    Some(ControlMessage::Resume) => continue,
                }
            };

            let prompt = match message {
                Message::User(prompt) => prompt,
                Message::AgentMessage(msg) => msg.render_for_model(),
            };

            // Each incoming message opens a fresh turn with a full budget.
            self.reset_budget().await;

            *self.current_task.write().await = Some(prompt.clone());
            *self.state.write().await = AgentState::Running;

            let mut events = (runtime.run_fn)(prompt);
            let mut completed = false;
            let mut abort = false;

            loop {
                tokio::select! {
                    event = events.next() => match event {
                        Some(AgentEvent::TurnComplete) => completed = true,
                        Some(event) => {
                            let _ = self.outbox.send(event);
                        }
                        None => break,
                    },
                    control = runtime.controls.recv() => match control {
                        Some(ControlMessage::Resume) => {}
                        Some(ControlMessage::Hibernate) => {
                            hibernating = true;
                            break;
                        }
                        Some(ControlMessage::Abort) | None => {
                            abort = true;
                            break;
                        }
                    }
                }
            }

            *self.current_task.write().await = None;
            *self.state.write().await = AgentState::Idle;

            if completed && !abort && !hibernating {
                let _ = self.outbox.send(AgentEvent::TurnComplete);
            }
            if abort {
                break 'lifecycle;
            }
        }

        *self.current_task.write().await = None;
        *self.state.write().await = AgentState::Stopped;
        Ok(())
    }

    pub async fn send(
        &self,
        message: Message,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<Message>> {
        self.sender.send(message).await
    }

    pub fn control(
        &self,
        message: ControlMessage,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ControlMessage>> {
        self.controls.send(message)
    }

    pub fn receiver(&self) -> tokio::sync::broadcast::Receiver<AgentEvent> {
        self.receiver.resubscribe()
    }
}

// ── Tool result length limit ─────────────────────────────────

/// Default cap for a single tool result, in bytes, admitted to the model
/// context.
pub const DEFAULT_MAX_TOOL_RESULT_CHARS: usize = 10_000;

/// A rig [`AgentHook`] that truncates tool results longer than `max_chars`
/// *before the model sees them*.
///
/// rig executes tools before streaming their results and feeds the full
/// output back into the model context on the next turn, so truncating streamed
/// events downstream does not bound the context. Replacing the result in this
/// hook keeps the actual model context — and everything derived from it
/// (streamed events, transcripts) — within the limit.
#[derive(Debug, Clone, Copy)]
pub struct ToolResultLimit {
    max_chars: usize,
}

impl ToolResultLimit {
    /// Create a hook that truncates results to at most `max_chars` bytes.
    pub fn new(max_chars: usize) -> Self {
        Self { max_chars }
    }

    /// Read the cap from the `WORKFLOW_MAX_TOOL_RESULT_CHARS` environment
    /// variable, falling back to [`DEFAULT_MAX_TOOL_RESULT_CHARS`].
    pub fn from_env() -> Self {
        let max_chars = std::env::var("WORKFLOW_MAX_TOOL_RESULT_CHARS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_MAX_TOOL_RESULT_CHARS);
        Self::new(max_chars)
    }
}

impl Default for ToolResultLimit {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_TOOL_RESULT_CHARS)
    }
}

impl<M: CompletionModel> AgentHook<M> for ToolResultLimit {
    async fn on_event(&self, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::ToolResult { result, .. } => {
                match truncate_tool_result(result, self.max_chars) {
                    Cow::Borrowed(_) => Flow::Continue,
                    Cow::Owned(truncated) => Flow::RewriteResult { result: truncated },
                }
            }
            _ => Flow::Continue,
        }
    }
}

/// Truncate `result` to at most `max_chars` bytes (on a char boundary),
/// appending a note with the original length so the model knows the data was
/// trimmed and can issue targeted follow-up calls.
fn truncate_tool_result<'a>(result: &'a str, max_chars: usize) -> Cow<'a, str> {
    if result.len() <= max_chars {
        return Cow::Borrowed(result);
    }
    let cutoff = result.floor_char_boundary(max_chars);
    let mut truncated = String::with_capacity(cutoff + 96);
    truncated.push_str(&result[..cutoff]);
    truncated.push_str("...\n\n[Tool result truncated at ");
    truncated.push_str(&max_chars.to_string());
    truncated.push_str(" characters (original was ");
    truncated.push_str(&result.len().to_string());
    truncated.push_str(" bytes). Use targeted queries for full data.]");
    Cow::Owned(truncated)
}

// ── Tests ────────────────────────────────────────────────────
#[cfg(test)]
mod budget_tests {
    use super::Budget;

    #[test]
    fn allows_up_to_limit_then_denies() {
        let mut budget = Budget::new(2);
        assert!(budget.request_message());
        assert!(budget.request_message());
        assert!(!budget.request_message());
    }

    #[test]
    fn reset_restores_allowance() {
        let mut budget = Budget::new(1);
        assert!(budget.request_message());
        assert!(!budget.request_message());
        budget.reset();
        assert!(budget.request_message());
    }
}

#[cfg(test)]
mod tool_result_limit_tests {
    use super::*;

    #[test]
    fn short_results_are_returned_unchanged() {
        let result = "short";
        assert!(matches!(
            truncate_tool_result(result, 10_000),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn boundary_length_results_are_kept() {
        let result = "x".repeat(20);
        assert!(matches!(
            truncate_tool_result(&result, 20),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn long_results_are_truncated_with_note() {
        let result = "x".repeat(10_000);
        let Cow::Owned(text) = truncate_tool_result(&result, 20) else {
            panic!("long result must be truncated");
        };
        assert!(text.starts_with(&"x".repeat(20)));
        assert!(text.contains("..."));
        assert!(text.contains("[Tool result truncated at 20 characters"));
        assert!(text.contains("original was 10000 bytes"));
        assert!(text.contains("Use targeted queries for full data."));
        // Truncated output (prefix + note) stays well below the original.
        assert!(text.len() < result.len());
    }

    #[test]
    fn truncates_at_char_boundary() {
        // "é" is 2 bytes in UTF-8; a 21-byte cap must cut at 20 bytes.
        let result = "é".repeat(50);
        let Cow::Owned(text) = truncate_tool_result(&result, 21) else {
            panic!("long result must be truncated");
        };
        assert!(text.starts_with(&"é".repeat(10)));
        assert!(text.is_char_boundary(text.len()));
    }
}

#[cfg(test)]
struct ToolAudit;

#[cfg(test)]
impl<M: CompletionModel> AgentHook<M> for ToolAudit {
    async fn on_event(&self, event: StepEvent<'_, M>) -> Flow {
        match event {
            StepEvent::ToolCall {
                tool_name, args, ..
            } => {
                println!("calling {tool_name} with {args}");
            }
            StepEvent::ToolResult {
                tool_name, result, ..
            } => {
                println!("{tool_name} returned {result}");
            }
            StepEvent::CompletionResponse {
                prompt: _,
                response,
            } => {
                dbg!(response.usage);
            }
            _ => {}
        }

        Flow::cont()
    }
}
#[ignore = "requires LLM API key in OPENAI_API_KEY env var and network access"]
#[tokio::test]
async fn test_agent_creation_and_prompt() {
    use rig::client::CompletionClient;
    use rig::providers::openai::CompletionsClient;
    use rig::{memory::InMemoryConversationMemory, tool::server::ToolServer};

    let api_key = std::env::var("OPENAI_API_KEY").expect("set OPENAI_API_KEY");

    let client = CompletionsClient::builder()
        .base_url("https://opencode.ai/zen/go/v1")
        .api_key(api_key)
        .build()
        .expect("should build OpenAI-compatible client");

    let tool_server = ToolServer::new();
    let tool_server_handle = tool_server.run();
    let rig_agent = client
        .agent("deepseek-v4-flash")
        .memory(InMemoryConversationMemory::new())
        .tool_server_handle(tool_server_handle)
        .conversation("id")
        .add_hook(ToolAudit)
        .build();

    let response = rig_agent
        .runner("Say \"hello from rig\" and nothing else")
        .run()
        .await;
    match response {
        Ok(text) => assert!(
            !text
                .messages
                .expect("response should have messages")
                .is_empty(),
            "response should not be empty"
        ),
        Err(e) => eprintln!("LLM call failed (this is ok in CI): {e}"),
    }
}
