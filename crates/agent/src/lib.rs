use futures::StreamExt;
use rig::{agent::AgentHook, client::CompletionClient, completion::CompletionModel};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    select,
    sync::{
        RwLock,
        mpsc::{Receiver, Sender, channel},
    },
};

// ── Types ────────────────────────────────────────────────────

enum AgentState {
    Idle,
    Running,
}

/// Data payload (LLM prompts, inter-agent content).
enum MessageType {
    User(String),
    AgentMessage(AgentId, String),
}

/// Control signals (shutdown, persist).
enum ControlMessage {
    Abort,
    Hibernate,
}

/// Inbound message discriminated by kind.
enum Message {
    Control(ControlMessage),
    Data(MessageType),
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

type AgentId = u32;
type ChatStream<M> = Pin<
    Box<
        dyn futures::Stream<
                Item = std::result::Result<
                    rig::agent::MultiTurnStreamItem<<M as CompletionModel>::StreamingResponse>,
                    rig::agent::StreamingError,
                >,
            > + Send,
    >,
>;
// ── Agent ────────────────────────────────────────────────────

/// Multi-agent orchestrator.
///
/// `rig_agent` is built **outside** this struct (model, preamble, tools,
/// memory all configured externally) and injected via `new()`.
/// This struct owns only the runtime wiring — message routing, state,
/// peer communication — not the LLM client config.
pub struct Agent<M: CompletionModel + 'static> {
    id: AgentId,
    rig_agent: Arc<rig::agent::Agent<M>>,
    internal_rx: Option<Receiver<MessageType>>,
    internal_tx: Sender<MessageType>,
    state: Arc<RwLock<AgentState>>,
    inbox: Receiver<Message>,
    /// Receiver for messages sent by the LLM via `send_message` tool.
    tool_rx: Option<tokio::sync::mpsc::UnboundedReceiver<(String, String)>>,
    shutdown: Shutdown,
    /// Channels to send [`Message`] to sibling agents.
    peer_channels: Arc<RwLock<HashMap<AgentId, Sender<Message>>>>,
}

impl<M: CompletionModel + 'static> Agent<M> {
    /// Create a new Agent.
    ///
    /// * `tool_rx` – the receiver end of [`SendMessageTool`]'s channel,
    ///   so the agent can forward LLM-emitted messages to peers.
    fn new(
        id: AgentId,
        rig_agent: rig::agent::Agent<M>,
        inbox: Receiver<Message>,
        tool_rx: tokio::sync::mpsc::UnboundedReceiver<(String, String)>,
        peer_channels: Arc<RwLock<HashMap<AgentId, Sender<Message>>>>,
    ) -> Self {
        let (internal_tx, internal_rx) = channel::<MessageType>(10);
        Self {
            id,
            rig_agent: Arc::new(rig_agent),
            internal_rx: Some(internal_rx),
            internal_tx,
            state: Arc::new(RwLock::new(AgentState::Idle)),
            inbox,
            tool_rx: Some(tool_rx),
            shutdown: Shutdown::new(),
            peer_channels,
        }
    }

    /// Background event-loop that reads from the internal channel
    /// and processes each message sequentially.
    async fn run_agent_loop(
        mut internal_rx: Receiver<MessageType>,
        rig_agent: Arc<rig::agent::Agent<M>>,
        state: Arc<RwLock<AgentState>>,
        shutdown: Shutdown,
    ) {
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

            *state.write().await = AgentState::Running;

            let mut stream = rig_agent.runner(prompt).max_turns(100).stream().await;
            while let Some(_item) = stream.next().await {}
            //     match item {
            //         Ok(rig::agent::MultiTurnStreamItem::StreamAssistantItem(content)) => {
            //             match content {
            //                 rig::streaming::StreamedAssistantContent::Text(text) => {
            //                     print!("{}", text.text());
            //                     let _ = std::io::stdout().flush();
            //                 }
            //                 rig::streaming::StreamedAssistantContent::ReasoningDelta {
            //                     reasoning,
            //                     ..
            //                 } => {
            //                     eprint!("\x1b[90m{}\x1b[0m", reasoning);
            //                 }
            //                 rig::streaming::StreamedAssistantContent::ToolCallDelta {
            //                     content,
            //                     ..
            //                 } => {
            //                     use rig::streaming::ToolCallDeltaContent;
            //                     if let ToolCallDeltaContent::Name(name) = content {
            //                         eprint!("\n[call {name}]");
            //                     }
            //                 }
            //                 _ => {}
            //             }
            //         }
            //         Ok(rig::agent::MultiTurnStreamItem::FinalResponse(response)) => {
            //             println!();
            //             eprintln!(
            //                 "[agent {id}] done — response length: {}",
            //                 response.response().len()
            //             );
            //         }
            //         Ok(_) => {}
            //         Err(e) => {
            //             eprintln!("\n[agent {id}] error: {e}");
            //         }
            //     }
            // }

            *state.write().await = AgentState::Idle;
        }
    }

    /// Run the main event loop.
    ///
    /// Spawns a background task for LLM processing so that
    /// the inbox stays responsive during streaming.
    pub async fn run(&mut self) {
        // Move internal_rx into a background task for LLM processing
        let agent_handle = match self.internal_rx.take() {
            Some(rx) => {
                let rig = Arc::clone(&self.rig_agent);
                let state = Arc::clone(&self.state);
                let shutdown = self.shutdown.clone();
                tokio::spawn(Self::run_agent_loop(rx, rig, state, shutdown))
            }
            None => return,
        };

        // Take the tool rx so we can poll it
        let mut tool_rx = self.tool_rx.take();

        loop {
            // Build the selectable branches
            let tool_branch = async {
                if let Some(rx) = tool_rx.as_mut() {
                    rx.recv().await
                } else {
                    // Never yield if there's no tool channel
                    std::future::pending::<Option<(String, String)>>().await
                }
            };

            select! {
                Some(msg) = self.inbox.recv() => {
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
                Some((recipient, content)) = tool_branch => {
                    // LLM called send_message → forward to peer
                    if let Ok(peer_id) = recipient.parse::<AgentId>() {
                        let peers = self.peer_channels.read().await;
                        if let Some(peer_tx) = peers.get(&peer_id) {
                            let msg = Message::Data(MessageType::AgentMessage(self.id, content));
                            if peer_tx.send(msg).await.is_err() {
                                eprintln!("[agent {}] peer {peer_id} unreachable", self.id);
                            }
                        } else {
                            eprintln!("[agent {}] unknown peer: {peer_id}", self.id);
                        }
                    } else {
                        eprintln!("[agent {}] invalid peer id: {recipient}", self.id);
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
