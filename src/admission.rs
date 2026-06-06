use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::types::SpawnRejection;

pub struct AdmissionController {
    semaphore: Arc<Semaphore>,
    timeout_ms: u64,
}

impl AdmissionController {
    pub fn new(max_concurrent: usize, timeout_ms: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            timeout_ms,
        }
    }

    pub async fn acquire_owned(&self) -> Result<AdmissionPermit, SpawnRejection> {
        let permit = timeout(
            Duration::from_millis(self.timeout_ms),
            self.semaphore.clone().acquire_owned(),
        )
        .await;

        match permit {
            Ok(Ok(permit)) => Ok(AdmissionPermit { _permit: permit }),
            Ok(Err(_)) => Err(SpawnRejection::SystemOverloaded),
            Err(_) => Err(SpawnRejection::SystemOverloaded),
        }
    }

    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

pub struct AdmissionPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::task::JoinSet;

    #[tokio::test]
    async fn test_admission_basic() {
        let controller = AdmissionController::new(10, 100);
        let permit = controller.acquire_owned().await;
        assert!(permit.is_ok());
        assert_eq!(controller.available_permits(), 9);
    }

    #[tokio::test]
    async fn test_admission_timeout() {
        let controller = AdmissionController::new(1, 50);
        let _permit = controller.acquire_owned().await.unwrap();

        let result = controller.acquire_owned().await;
        assert!(matches!(result, Err(SpawnRejection::SystemOverloaded)));
    }

    #[tokio::test]
    async fn test_admission_concurrent() {
        let controller = Arc::new(AdmissionController::new(5, 100));
        let mut set = JoinSet::new();

        // Spawn all tasks concurrently, holding permits until explicitly dropped
        let mut permits = Vec::new();
        for _ in 0..10 {
            let c = controller.clone();
            set.spawn(async move { c.acquire_owned().await });
        }

        // Collect all results
        let mut successes = 0;
        let mut failures = 0;
        while let Some(result) = set.join_next().await {
            match result.unwrap() {
                Ok(permit) => {
                    successes += 1;
                    permits.push(permit);
                }
                Err(_) => failures += 1,
            }
        }

        assert_eq!(successes, 5);
        assert_eq!(failures, 5);
    }

    #[tokio::test]
    async fn test_admission_release_on_drop() {
        let controller = Arc::new(AdmissionController::new(2, 100));
        {
            let _p1 = controller.acquire_owned().await.unwrap();
            let _p2 = controller.acquire_owned().await.unwrap();
            assert_eq!(controller.available_permits(), 0);
        }
        assert_eq!(controller.available_permits(), 2);
    }
}
