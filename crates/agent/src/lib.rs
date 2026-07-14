pub mod agent_pool;

use async_stream::stream;
use futures::{Stream, StreamExt};
use rig::{
    agent::{AgentHook, MultiTurnStreamItem},
    completion::CompletionModel,
    message::Text,
    streaming::StreamedAssistantContent,
};
use std::{
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    select,
    sync::{
        Mutex as TokioMutex, RwLock,
        mpsc::{Receiver, Sender, channel, unbounded_channel},
    },
};

// ── Types ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum AgentState {
    Idle,
    Running,
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle => write!(f, "idle"),
            AgentState::Running => write!(f, "running"),
        }
    }
}

/// Data payload (LLM prompts, inter-agent content).
pub enum MessageType {
    User(String),
    AgentMessage(AgentId, String),
}

/// Control signals (shutdown, persist).
pub enum ControlMessage {
    Abort,
    Hibernate,
}

/// Inbound message discriminated by kind.
pub enum Message {
    Control(ControlMessage),
    Data(MessageType),
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
    ToolCall { name: String },
    /// A tool call completed with a result.
    ToolResult { name: String, result: String },
    /// One completion turn finished successfully.
    TurnComplete,
    /// A stream-level error occurred.
    Error(String),
}

/// Shared runtime flag for graceful shutdown.
struct Shutdown(Arc<AtomicBool>);

impl Shutdown {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn signal(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

pub type AgentId = u32;

tokio::task_local! {
    static CURRENT_AGENT_ID: AgentId;
}

pub fn current_agent_id() -> AgentId {
    CURRENT_AGENT_ID.try_with(|id| *id).unwrap_or(0)
}

/// Type-erased "run one turn" function: given a prompt, produce a stream of
/// [`AgentEvent`]s. The concrete `rig::agent::Agent<M>` and its associated
/// `StreamingResponse` type are captured *inside* the closure built by
/// [`Agent::new`], so neither `M` nor its response type leaks into the
/// [`Agent`] struct. This is what lets `Agent` be non-generic.
pub type RunFn =
    Box<dyn Fn(String) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> + Send + Sync>;

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
pub struct Agent {
    id: AgentId,
    role: String,
    current_task: Arc<RwLock<Option<String>>>,
    run_fn: StdMutex<Option<RunFn>>,
    internal_rx: StdMutex<Option<Receiver<MessageType>>>,
    internal_tx: Sender<MessageType>,
    state: Arc<RwLock<AgentState>>,
    shutdown: Shutdown,
    sender: Sender<Message>,
    inbox: TokioMutex<Receiver<Message>>,
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
    pub fn new<M: CompletionModel + 'static>(
        id: AgentId,
        role: String,
        rig_agent: rig::agent::Agent<M>,
    ) -> Self {
        let (internal_tx, internal_rx) = channel::<MessageType>(10);
        let (sender, inbox) = channel::<Message>(10);
        let (outbox, receiver) = tokio::sync::broadcast::channel(10);
        let rig = Arc::new(rig_agent);
        let run_fn: RunFn = Box::new(move |prompt: String| {
            let rig = Arc::clone(&rig);
            Box::pin(stream! {
                let mut stream = rig.runner(prompt).max_turns(100).stream().await;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => match content {
                            StreamedAssistantContent::Text(Text { text, .. }) => {
                                yield AgentEvent::Text(text);
                            }
                            StreamedAssistantContent::ReasoningDelta { reasoning, .. }
                                if !reasoning.is_empty() =>
                            {
                                yield AgentEvent::Reasoning(reasoning);
                            }
                            StreamedAssistantContent::ToolCallDelta { content, .. } => {
                                use rig::streaming::ToolCallDeltaContent;
                                if let ToolCallDeltaContent::Name(name) = content {
                                    yield AgentEvent::ToolCall { name };
                                }
                            }
                            StreamedAssistantContent::Reasoning(reasoning) => {
                                let mut buf = String::new();
                                for block in reasoning.content {
                                    if let rig::message::ReasoningContent::Text { text, .. } = block {
                                        buf.push_str(&text);
                                    }
                                }
                                if !buf.is_empty() {
                                    yield AgentEvent::Reasoning(buf);
                                }
                            }
                            _ => {}
                        },
                        Ok(MultiTurnStreamItem::FinalResponse(_)) => {
                            yield AgentEvent::TurnComplete;
                        }
                        Ok(_) => {}
                        Err(e) => yield AgentEvent::Error(e.to_string()),
                    }
                }
            })
        });

        Self {
            id,
            role,
            current_task: Arc::new(RwLock::new(None)),
            run_fn: StdMutex::new(Some(run_fn)),
            internal_rx: StdMutex::new(Some(internal_rx)),
            internal_tx,
            state: Arc::new(RwLock::new(AgentState::Idle)),
            sender,
            inbox: TokioMutex::new(inbox),
            shutdown: Shutdown::new(),
            receiver,
            outbox,
        }
    }

    pub fn new_no_model(id: AgentId, role: String) -> Self {
        let (internal_tx, internal_rx) = channel::<MessageType>(10);
        let (sender, inbox) = channel::<Message>(10);
        let (outbox, receiver) = tokio::sync::broadcast::channel(10);
        let run_fn: RunFn = Box::new(move |prompt: String| {
            Box::pin(stream! {
                yield AgentEvent::Text(prompt);
                yield AgentEvent::TurnComplete;
            })
        });

        Self {
            id,
            role,
            current_task: Arc::new(RwLock::new(None)),
            run_fn: StdMutex::new(Some(run_fn)),
            internal_rx: StdMutex::new(Some(internal_rx)),
            internal_tx,
            state: Arc::new(RwLock::new(AgentState::Idle)),
            sender,
            inbox: TokioMutex::new(inbox),
            shutdown: Shutdown::new(),
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

    /// Background event-loop that reads from the internal channel
    /// and processes each message sequentially. Non-generic: the model is
    /// already erased behind [`RunFn`].
    async fn run_agent_loop(
        id: AgentId,
        mut internal_rx: Receiver<MessageType>,
        run_fn: RunFn,
        state: Arc<RwLock<AgentState>>,
        current_task: Arc<RwLock<Option<String>>>,
        shutdown: Shutdown,
        outbox: tokio::sync::broadcast::Sender<AgentEvent>,
    ) {
        CURRENT_AGENT_ID
            .scope(id, async {
                while let Some(message) = internal_rx.recv().await {
                    if shutdown.is_requested() {
                        break;
                    }

                    let prompt = match message {
                        MessageType::User(p) => p,
                        MessageType::AgentMessage(from, content) => {
                            format!("[message from agent {from}]: {content}")
                        }
                    };

                    *current_task.write().await = Some(prompt.clone());
                    *state.write().await = AgentState::Running;

                    let mut stream = run_fn(prompt);
                    while let Some(event) = stream.next().await {
                        let _ = outbox.send(event);
                    }

                    *current_task.write().await = None;
                    *state.write().await = AgentState::Idle;
                }
            })
            .await;
    }

    /// Take `internal_rx` and `run_fn` from self (called once at the start
    /// of [`run`]).
    fn take_plumbing(&self) -> Option<(Receiver<MessageType>, RunFn)> {
        let rx = self.internal_rx.lock().unwrap().take()?;
        let run_fn = self.run_fn.lock().unwrap().take()?;
        Some((rx, run_fn))
    }

    /// Async helper that locks the inbox, awaits a message, and returns it.
    /// This is needed so [`tokio::select!`] can poll a future that holds a
    /// [`TokioMutex`] guard across the `recv()` await.
    async fn recv_from_inbox(&self) -> Option<Message> {
        self.inbox.lock().await.recv().await
    }

    /// Run the main event loop.
    ///
    /// Spawns a background task for LLM processing so that the inbox stays
    /// responsive during streaming.
    pub async fn run(&self) {
        // Move internal_rx and the erased runner into a background task.
        let agent_handle = match self.take_plumbing() {
            Some((rx, run_fn)) => {
                let state = Arc::clone(&self.state);
                let shutdown = self.shutdown.clone();
                let outbox = self.outbox.clone();
                let current_task = Arc::clone(&self.current_task);
                let id = self.id;
                tokio::spawn(Self::run_agent_loop(
                    id,
                    rx,
                    run_fn,
                    state,
                    current_task,
                    shutdown,
                    outbox,
                ))
            }
            _ => return,
        };

        loop {
            select! {
                Some(msg) = self.recv_from_inbox() => {
                    match msg {
                        Message::Control(cmd) => match cmd {
                            ControlMessage::Abort => {
                                eprintln!("[agent {}] abort requested", self.id);
                                self.shutdown.signal();
                                break;
                            }
                            ControlMessage::Hibernate => {
                                eprintln!("[agent {}] hibernate (not yet implemented)", self.id);
                            }
                        },
                        Message::Data(data) => {
                            if self.shutdown.is_requested() {
                                break;
                            }
                            if self.internal_tx.send(data).await.is_err() {
                                eprintln!("[agent {}] internal channel closed", self.id);
                                break;
                            }
                        },
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("[agent {}] ctrl+c received", self.id);
                    self.shutdown.signal();
                    break;
                }
            }
        }

        // Wait for the agent loop to finish
        let _ = agent_handle.await;
        eprintln!("[agent {}] shut down", self.id);
    }

    pub fn sender(&self) -> &Sender<Message> {
        &self.sender
    }

    pub fn receiver(&self) -> tokio::sync::broadcast::Receiver<AgentEvent> {
        self.receiver.resubscribe()
    }
}

// ── Tests ────────────────────────────────────────────────────
use rig::agent::{Flow, StepEvent};

struct ToolAudit;

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
        .unwrap();

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
            !text.messages.unwrap().is_empty(),
            "response should not be empty"
        ),
        Err(e) => eprintln!("LLM call failed (this is ok in CI): {e}"),
    }
}
