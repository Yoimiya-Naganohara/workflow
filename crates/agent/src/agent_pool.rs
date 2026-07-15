use std::{num::NonZeroUsize, sync::Arc};

use anyhow::Result;
use lru::LruCache;
use tokio::{spawn, sync::Mutex, task::JoinHandle};

use crate::{Agent, AgentId, ControlMessage, MessageType};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub role: String,
    pub current_task: Option<String>,
}

struct AgentEntity {
    agent: Arc<Agent>,
    handler: JoinHandle<()>,
}

pub struct AgentPool {
    lru: Arc<Mutex<LruCache<AgentId, Arc<AgentEntity>>>>,
}

impl AgentPool {
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            lru: Arc::new(Mutex::new(LruCache::new(capacity))),
        }
    }

    pub async fn add_agent(&self, agent: Arc<Agent>) -> Result<()> {
        let id = agent.id();
        let handler = {
            let agent = agent.clone();
            spawn(async move { agent.run().await })
        };
        let entity = Arc::new(AgentEntity { agent, handler });
        if let Some((_, old)) = self.lru.lock().await.push(id, entity) {
            old.handler.abort();
        }
        Ok(())
    }

    pub async fn get_agent(&self, id: &AgentId) -> Option<Arc<Agent>> {
        self.lru.lock().await.get(id).map(|r| r.agent.clone())
    }

    async fn shutdown(entity: &Arc<AgentEntity>) {
        let _ = entity
            .agent
            .sender()
            .send(MessageType::Control(ControlMessage::Abort))
            .await;
        entity.handler.abort();
    }

    pub async fn remove_agent(&self, id: &AgentId) {
        if let Some(entity) = self.lru.lock().await.pop(id) {
            Self::shutdown(&entity).await;
        }
    }

    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        let lru = self.lru.lock().await;
        let mut out = Vec::with_capacity(lru.len());
        for (_, entity) in lru.iter() {
            let id = entity.agent.id();
            let role = entity.agent.role().to_owned();
            let task = entity.agent.current_task().read().await.clone();
            out.push(AgentInfo {
                id,
                role,
                current_task: task,
            });
        }
        out
    }
}
