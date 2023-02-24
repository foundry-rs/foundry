use crate::error::SolError;

/// Errors that can happen when working with [`Cheacodes`]
#[derive(Debug, thiserror::Error)]
pub enum CheatcodesError {
    #[error("You need to stop broadcasting before you can select forks.")]
    SelectForkDuringBroadcast,
}

impl SolError for CheatcodesError {}
