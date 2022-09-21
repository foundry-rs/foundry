//! Error variants used to unify different connection streams

/// An error that can occur when reading an incoming request
#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error(transparent)]
    Axum(#[from] axum::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Disconnect")]
    Disconnect,
}
