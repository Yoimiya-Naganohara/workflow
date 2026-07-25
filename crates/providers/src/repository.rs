use std::collections::HashMap;

use reqwest::StatusCode;

use crate::cache::{CacheLayout, CacheMetadata};
use crate::error::Result;
use crate::model::ProviderInfo;

pub enum FetchResult {
    NotModified,
    Updated(Vec<ProviderInfo>),
}

pub struct ProviderRepository {
    client: reqwest::Client,
    metadata: CacheMetadata,
    layout: CacheLayout,
}

impl ProviderRepository {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            metadata: CacheMetadata::default(),
            layout: CacheLayout::default(),
        }
    }

    pub async fn fetch(&mut self, url: &str) -> Result<FetchResult> {
        let mut req = self.client.get(url);

        if let Some(etag) = &self.metadata.etag {
            req = req.header("if-none-match", etag);
        }

        let response = req.send().await?;

        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(FetchResult::NotModified);
        }

        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let map: HashMap<String, ProviderInfo> = response.json().await?;

        self.metadata.etag = etag;

        Ok(FetchResult::Updated(map.into_values().collect()))
    }

    pub async fn load_cache(&self) -> Result<Vec<ProviderInfo>> {
        let data = tokio::fs::read_to_string(&self.layout.data).await?;
        let map: HashMap<String, ProviderInfo> = serde_json::from_str(&data)?;
        Ok(map.into_values().collect())
    }

    pub async fn save_cache(&self, providers: &[ProviderInfo]) -> Result<()> {
        if let Some(parent) = self.layout.data.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Save as HashMap (matching remote format) for consistency
        let map: HashMap<String, &ProviderInfo> =
            providers.iter().map(|p| (p.id.clone(), p)).collect();
        let json = serde_json::to_string_pretty(&map)?;
        tokio::fs::write(&self.layout.data, json).await?;
        Ok(())
    }
}

impl Default for ProviderRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_deserialize_api_json() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = std::fs::canonicalize(manifest_dir.join("../../api.json"))
            .expect("api.json not found at workspace root");
        let data = tokio::fs::read_to_string(&path)
            .await
            .expect("read api.json");
        let map: HashMap<String, ProviderInfo> =
            serde_json::from_str(&data).expect("deserialize api.json");
        let providers: Vec<ProviderInfo> = map.into_values().collect();
        assert!(!providers.is_empty(), "no providers loaded");
        if let Some(p) = providers.first() {
            assert!(!p.models.is_empty(), "first provider has no models");
            println!(
                "loaded {} providers, first: {} ({} models)",
                providers.len(),
                p.name,
                p.models.len()
            );
        }
    }

    #[tokio::test]
    async fn test_save_and_load_cache() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let data_path = dir.path().join("api.json");

        let repo = ProviderRepository {
            client: reqwest::Client::new(),
            metadata: CacheMetadata::default(),
            layout: CacheLayout {
                data: data_path.clone(),
                meta: dir.path().join("meta.json"),
            },
        };

        let sample = vec![ProviderInfo {
            id: "test".into(),
            name: "Test Provider".into(),
            api: None,
            env: vec![],
            npm: None,
            doc: None,
            models: HashMap::new(),
        }];

        repo.save_cache(&sample).await.expect("save cache");
        assert!(data_path.exists(), "cache file created");

        let loaded = repo.load_cache().await.expect("load cache");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "test");
    }
}
