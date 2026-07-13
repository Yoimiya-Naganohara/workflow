use std::{num::NonZeroUsize, sync::Arc};

use anyhow::Result;
use dashmap::DashMap;
use lru::LruCache;
use tokio::{
    spawn,
    sync::{Mutex, RwLock},
    task::JoinHandle,
};

use crate::{Agent, AgentId, AgentState, ControlMessage, Message};

#[derive(serde::Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub role: String,
    pub current_task: Option<String>,
}

/// Registry of live agents. Non-generic so it can hold agents backed by
/// different completion models/providers simultaneously.
struct AgentEntity {
    agent: Arc<Agent>,
    handler: JoinHandle<()>,
}
pub struct AgentPool {
    lru: Arc<Mutex<LruCache<AgentId, Arc<AgentEntity>>>>,
    sender: tokio::sync::mpsc::UnboundedSender<Arc<Agent>>,
}

impl AgentPool {
    pub fn new(capacity: NonZeroUsize) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Arc<Agent>>();
        let lru = Arc::new(Mutex::new(LruCache::new(capacity)));
        let lru_clone = lru.clone();
        spawn(async move {
            while let Some(agent) = receiver.recv().await {
                let agent_clone = agent.clone();
                let handler = tokio::spawn(async move { agent_clone.run().await });

                if let Some((_, agent)) = lru_clone.lock().await.push(
                    agent.id(),
                    Arc::new(AgentEntity {
                        handler,
                        agent: agent,
                    }),
                ) {
                    agent.handler.abort();
                };
            }
            for (_, agent) in lru_clone.lock().await.iter() {
                Self::shutdown(agent).await;
            }
        });
        Self { lru, sender }
    }
    // if the capacity is exceeded, the least recently used agent is shutdown before adding the new one or forbid the addition of new agents
    pub async fn add_agent(&self, agent: Arc<Agent>) -> Result<()> {
        let agent_clone = agent.clone();
        self.sender.send(agent_clone)?;
        Ok(())
    }

    /// Borrow an agent by id. Holds DashMap's shard read guard; drop the
    /// returned [`Ref`] to release it.
    pub async fn get_agent(&self, id: &AgentId) -> Option<Arc<Agent>> {
        self.lru.lock().await.get(id).map(|r| r.agent.clone())
    }
    async fn shutdown(agent_entity: &Arc<AgentEntity>) {
        if let Some(send_error) = agent_entity
            .agent
            .sender()
            .send(Message::Control(ControlMessage::Abort))
            .await
            .err()
        {
            agent_entity.handler.abort();
        };
    }
    pub async fn remove_agent(&self, id: &AgentId) {
        if let Some((agent)) = self.lru.lock().await.pop(id) {
            Self::shutdown(&agent).await;
        };
    }

    /// Snapshot every live agent's id, role, and current task.
    ///
    /// Borrows the LRU cache once and reads each agent's `current_task`
    /// under its async RwLock — no Arc clones, no allocations beyond
    /// the returned `Vec` and the `Option<String>` for the task.
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        let lru = self.lru.lock().await;
        let mut out = Vec::with_capacity(lru.len());
        for (_, entity) in lru.iter() {
            let id = entity.agent.id();
            let role = entity.agent.role().to_owned();
            let task = entity
                .agent
                .current_task()
                .read()
                .await
                .clone();
            out.push(AgentInfo {
                id,
                role,
                current_task: task,
            });
        }
        out
    }
}
