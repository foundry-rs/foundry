//! errors related to fuzz tests
use proptest::test_runner::Reason;

/// Possible errors when running fuzz tests
#[derive(Debug, thiserror::Error)]
pub enum FuzzError {
    #[error("Couldn't call unknown contract")]
    UnknownContract,
    #[error("Failed contract call")]
    FailedContractCall,
    #[error("`vm.assume` reject")]
    AssumeReject,
    #[error("The `vm.assume` cheatcode rejected too many inputs ({0} allowed)")]
    TooManyRejects(u32),
}

impl From<FuzzError> for Reason {
    fn from(error: FuzzError) -> Self {
        error.to_string().into()
    }
}
