use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::core::types::SpawnRequest;

pub struct SuspendConfig {
    pub hard_timeout_ms: u64,
    pub dynamic_timeout_ms: u64,
}

impl SuspendConfig {
    pub fn effective_timeout_ms(&self) -> u64 {
        self.dynamic_timeout_ms.min(self.hard_timeout_ms)
    }
}

impl Default for SuspendConfig {
    fn default() -> Self {
        Self {
            hard_timeout_ms: crate::core::types::DEFAULT_SUSPEND_TIMEOUT_MS,
            dynamic_timeout_ms: crate::core::types::DEFAULT_SUSPEND_TIMEOUT_MS,
        }
    }
}

pub struct SuspendedRequest {
    pub request: SpawnRequest,
    pub priority: f32,
    pub enqueued_at: std::time::Instant,
}

pub struct SuspendQueue {
    queue: VecDeque<SuspendedRequest>,
    notify: Arc<Notify>,
    config: SuspendConfig,
}

impl SuspendQueue {
    pub fn new(config: SuspendConfig) -> Self {
        Self {
            queue: VecDeque::new(),
            notify: Arc::new(Notify::new()),
            config,
        }
    }

    pub fn enqueue(&mut self, request: SpawnRequest, priority: f32) {
        let entry = SuspendedRequest {
            request,
            priority,
            enqueued_at: std::time::Instant::now(),
        };

        let pos = self
            .queue
            .binary_search_by(|r| priority.partial_cmp(&r.priority).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or_else(|p| p);
        self.queue.insert(pos, entry);
        self.notify.notify_one();
    }

    pub fn dequeue(&mut self) -> Option<SuspendedRequest> {
        self.queue.pop_front()
    }

    pub async fn wait_for_item(&self) {
        if self.queue.is_empty() {
            let _ = timeout(
                Duration::from_millis(self.config.effective_timeout_ms()),
                self.notify.notified(),
            )
            .await;
        }
    }

    pub fn prune_expired(&mut self) -> Vec<SpawnRequest> {
        let timeout = Duration::from_millis(self.config.effective_timeout_ms());
        let now = std::time::Instant::now();
        let mut pruned = Vec::new();

        self.queue.retain(|r| {
            if now.duration_since(r.enqueued_at) > timeout {
                pruned.push(r.request.clone());
                false
            } else {
                true
            }
        });

        pruned
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Priority-ordered queue for deferred spawn requests.
pub trait SuspendQueueOps: Send + Sync {
    fn enqueue(&mut self, request: SpawnRequest, priority: f32);
    fn dequeue(&mut self) -> Option<SuspendedRequest>;
    fn prune_expired(&mut self) -> Vec<SpawnRequest>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

impl SuspendQueueOps for SuspendQueue {
    fn enqueue(&mut self, request: SpawnRequest, priority: f32) {
        self.enqueue(request, priority)
    }

    fn dequeue(&mut self) -> Option<SuspendedRequest> {
        self.dequeue()
    }

    fn prune_expired(&mut self) -> Vec<SpawnRequest> {
        self.prune_expired()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(budget: u64) -> SpawnRequest {
        SpawnRequest {
            trace_id: [0u8; 16],
            span_id: 0,
            parent_span_id: 0,
            task_description_embedding: [0.0f32; crate::core::types::EMBEDDING_DIM],
            role_description_embedding: [0.0f32; crate::core::types::EMBEDDING_DIM],
            value_statement_embedding: [0.0f32; crate::core::types::EMBEDDING_DIM],
            requested_budget: budget,
            current_depth: 0,
            responsibility_chain: vec![],
            raw_text_ref: None,
        }
    }

    #[test]
    fn test_priority_ordering() {
        let config = SuspendConfig {
            hard_timeout_ms: 1000,
            dynamic_timeout_ms: 1000,
        };
        let mut queue = SuspendQueue::new(config);

        queue.enqueue(make_request(100), 0.3);
        queue.enqueue(make_request(200), 0.9);
        queue.enqueue(make_request(300), 0.6);

        let r1 = queue.dequeue().unwrap();
        let r2 = queue.dequeue().unwrap();
        let r3 = queue.dequeue().unwrap();

        assert!((r1.priority - 0.9).abs() < f32::EPSILON);
        assert!((r2.priority - 0.6).abs() < f32::EPSILON);
        assert!((r3.priority - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn test_prune_expired() {
        let config = SuspendConfig {
            hard_timeout_ms: 10,
            dynamic_timeout_ms: 10,
        };
        let mut queue = SuspendQueue::new(config);

        queue.enqueue(make_request(100), 0.5);
        queue.enqueue(make_request(200), 0.8);

        std::thread::sleep(Duration::from_millis(20));

        let pruned = queue.prune_expired();
        assert_eq!(pruned.len(), 2);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_wait_timeout() {
        let config = SuspendConfig {
            hard_timeout_ms: 20,
            dynamic_timeout_ms: 20,
        };
        let queue = SuspendQueue::new(config);

        let start = std::time::Instant::now();
        queue.wait_for_item().await;
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(15));
        assert!(elapsed < Duration::from_millis(100));
    }
}
