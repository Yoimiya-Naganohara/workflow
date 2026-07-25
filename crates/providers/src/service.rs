use crate::error::Result;
use crate::repository::{FetchResult, ProviderRepository};
use crate::store::ProviderStore;

pub struct ProviderService {
    store: ProviderStore,
    repository: ProviderRepository,
}

impl ProviderService {
    pub fn new() -> Self {
        Self {
            store: ProviderStore::new(),
            repository: ProviderRepository::new(),
        }
    }

    pub fn store(&self) -> &ProviderStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut ProviderStore {
        &mut self.store
    }

    pub async fn initialize(&mut self) -> Result<()> {
        if let Ok(data) = self.repository.load_cache().await {
            self.store.replace(data);
        }
        Ok(())
    }

    pub async fn refresh(&mut self) -> Result<bool> {
        match self.repository.fetch("https://models.dev/api.json").await? {
            FetchResult::NotModified => Ok(false),
            FetchResult::Updated(data) => {
                let changed = self.store.replace(data.clone());
                if changed {
                    self.repository.save_cache(&data).await?;
                }
                Ok(changed)
            }
        }
    }
}

impl Default for ProviderService {
    fn default() -> Self {
        Self::new()
    }
}
