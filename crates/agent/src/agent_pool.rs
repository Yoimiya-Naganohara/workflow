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
}
