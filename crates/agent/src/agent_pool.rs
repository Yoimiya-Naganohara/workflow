use std::{num::NonZeroUsize, sync::Arc};

use anyhow::Result;
use lru::LruCache;
use tokio::{
    spawn,
    sync::{Mutex, broadcast},
    task::JoinHandle,
};

use crate::{Agent, AgentId};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub role: String,
    pub current_task: Option<String>,
}

#[derive(Clone)]
pub enum AgentPoolEvent {
    Added(Arc<Agent>),
    Removed(AgentId),
}

struct AgentEntity {
    agent: Arc<Agent>,
    handler: JoinHandle<()>,
}

pub struct AgentPool {
    lru: Arc<Mutex<LruCache<AgentId, Arc<AgentEntity>>>>,
    events: broadcast::Sender<AgentPoolEvent>,
}

impl AgentPool {
    pub fn new(capacity: NonZeroUsize) -> Self {
        let (events, _) = broadcast::channel(capacity.get().max(32));
        Self {
            lru: Arc::new(Mutex::new(LruCache::new(capacity))),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentPoolEvent> {
        self.events.subscribe()
    }

    pub async fn add_agent(&self, agent: Arc<Agent>) -> Result<()> {
        let id = agent.id();
        let handler = {
            let agent = agent.clone();
            spawn(async move {
                if let Err(error) = agent.run().await {
                    eprintln!("agent runtime stopped: {error}");
                }
            })
        };
        let entity = Arc::new(AgentEntity {
            agent: Arc::clone(&agent),
            handler,
        });
        let evicted = self.lru.lock().await.push(id, entity);
        if let Some((old_id, old)) = evicted {
            Self::shutdown(&old);
            let _ = self.events.send(AgentPoolEvent::Removed(old_id));
        }
        let _ = self.events.send(AgentPoolEvent::Added(agent));
        Ok(())
    }

    pub async fn get_agent(&self, id: &AgentId) -> Option<Arc<Agent>> {
        self.lru.lock().await.get(id).map(|r| r.agent.clone())
    }

    fn shutdown(entity: &AgentEntity) {
        entity.handler.abort();
    }

    pub async fn remove_agent(&self, id: &AgentId) {
        let entity = self.lru.lock().await.pop(id);
        if let Some(entity) = entity {
            Self::shutdown(&entity);
            let _ = self.events.send(AgentPoolEvent::Removed(*id));
        }
    }

    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        let agents: Vec<Arc<Agent>> = {
            let lru = self.lru.lock().await;
            lru.iter()
                .map(|(_, entity)| Arc::clone(&entity.agent))
                .collect()
        };

        let mut out = Vec::with_capacity(agents.len());
        for agent in agents {
            out.push(AgentInfo {
                id: agent.id(),
                role: agent.role().to_owned(),
                current_task: agent.current_task().read().await.clone(),
            });
        }
        out
    }
}
