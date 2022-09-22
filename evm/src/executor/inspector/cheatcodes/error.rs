use bytes::Bytes;

/// Errors that can happen when working with [`Cheacodes`]
#[derive(Debug, thiserror::Error)]
pub enum CheatcodesError {
    #[error("You need to stop broadcasting before you can select forks.")]
    SelectForkDuringBroadcast,
}

impl From<CheatcodesError> for Bytes {
    fn from(err: CheatcodesError) -> Self {
        Bytes::copy_from_slice(err.to_string().as_bytes())
    }
}
