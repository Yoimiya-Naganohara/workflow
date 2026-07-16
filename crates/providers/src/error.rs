use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("network error")]
    Network(#[from] reqwest::Error),

    #[error("io error")]
    Io(#[from] std::io::Error),

    #[error("json error")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ProviderError>;
