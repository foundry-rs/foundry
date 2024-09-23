//! Errors related to fuzz tests.

use proptest::test_runner::Reason;

/// Possible errors when running fuzz tests
#[derive(Debug, thiserror::Error)]
pub enum FuzzError {
    #[error("`vm.assume` reject")]
    AssumeReject,
    #[error("`vm.assume` rejected too many inputs ({0} allowed)")]
    TooManyRejects(u32),
}

impl From<FuzzError> for Reason {
    fn from(error: FuzzError) -> Self {
        error.to_string().into()
    }
}
