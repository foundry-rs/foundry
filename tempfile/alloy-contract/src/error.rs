use alloy_dyn_abi::Error as AbiError;
use alloy_primitives::Selector;
use alloy_provider::PendingTransactionError;
use alloy_transport::TransportError;
use thiserror::Error;

/// Dynamic contract result type.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Error when interacting with contracts.
#[derive(Debug, Error)]
pub enum Error {
    /// Unknown function referenced.
    #[error("unknown function: function {0} does not exist")]
    UnknownFunction(String),
    /// Unknown function selector referenced.
    #[error("unknown function: function with selector {0} does not exist")]
    UnknownSelector(Selector),
    /// Called `deploy` with a transaction that is not a deployment transaction.
    #[error("transaction is not a deployment transaction")]
    NotADeploymentTransaction,
    /// `contractAddress` was not found in the deployment transaction’s receipt.
    #[error("missing `contractAddress` from deployment transaction receipt")]
    ContractNotDeployed,
    /// The contract returned no data.
    #[error("contract call to `{0}` returned no data (\"0x\"); the called address might not be a contract")]
    ZeroData(String, #[source] AbiError),
    /// An error occurred ABI encoding or decoding.
    #[error(transparent)]
    AbiError(#[from] AbiError),
    /// An error occurred interacting with a contract over RPC.
    #[error(transparent)]
    TransportError(#[from] TransportError),
    /// An error occured while waiting for a pending transaction.
    #[error(transparent)]
    PendingTransactionError(#[from] PendingTransactionError),
}

impl From<alloy_sol_types::Error> for Error {
    #[inline]
    fn from(e: alloy_sol_types::Error) -> Self {
        Self::AbiError(e.into())
    }
}

impl Error {
    #[cold]
    pub(crate) fn decode(name: &str, data: &[u8], error: AbiError) -> Self {
        if data.is_empty() {
            let name = name.split('(').next().unwrap_or(name);
            return Self::ZeroData(name.to_string(), error);
        }
        Self::AbiError(error)
    }
}
