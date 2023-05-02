//! errors related to fuzz tests
use proptest::test_runner::Reason;

/// Magic return code for the `assume` cheatcode
pub const ASSUME_MAGIC_RETURN_CODE: &str = "FOUNDRY::ASSUME";

/// Possible errors when running fuzz tests
#[derive(Debug, thiserror::Error)]
pub enum FuzzError {
    #[error("Couldn't call unknown contract")]
    UnknownContract,
    #[error("Couldn't find function")]
    UnknownFunction,
    #[error("Failed to decode fuzzer inputs")]
    FailedDecodeInput,
    #[error("Failed contract call")]
    FailedContractCall,
    #[error("Empty state changeset")]
    EmptyChangeset,
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
