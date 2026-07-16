use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub etag: Option<String>,
    pub hash: Option<String>,
    pub updated_at: Option<u64>,
}

pub struct CacheLayout {
    pub data: PathBuf,
    pub meta: PathBuf,
}

impl CacheLayout {
    pub fn default_path() -> Self {
        let root = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("workflow")
            .join("providers");

        Self {
            data: root.join("api.json"),
            meta: root.join("meta.json"),
        }
    }
}

impl Default for CacheLayout {
    fn default() -> Self {
        Self::default_path()
    }
}
