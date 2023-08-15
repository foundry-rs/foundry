/// Result alias
pub type NodeResult<T> = Result<T, NodeError>;

/// An error that can occur when launching a anvil instance
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error(transparent)]
    Hyper(#[from] hyper::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
