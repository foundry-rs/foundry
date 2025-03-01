//! Commonly used errors for the `eth_` namespace.

/// List of JSON-RPC error codes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EthRpcErrorCode {
    /// Failed to send transaction, See also <https://github.com/MetaMask/eth-rpc-errors/blob/main/src/error-constants.ts>
    TransactionRejected,
    /// Custom geth error code, <https://github.com/vapory-legacy/wiki/blob/master/JSON-RPC-Error-Codes-Improvement-Proposal.md>
    ExecutionError,
    /// <https://eips.ethereum.org/EIPS/eip-1898>
    InvalidInput,
    /// Thrown when a block wasn't found <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1898.md>
    /// > If the block is not found, the callee SHOULD raise a JSON-RPC error (the recommended
    /// > error code is -32001: Resource not found).
    ResourceNotFound,
    /// Thrown when querying for `finalized` or `safe` block before the merge transition is
    /// finalized, <https://github.com/ethereum/execution-apis/blob/6d17705a875e52c26826124c2a8a15ed542aeca2/src/schemas/block.yaml#L109>
    UnknownBlock,
}

impl EthRpcErrorCode {
    /// Returns the error code as `i32`
    pub const fn code(&self) -> i32 {
        match *self {
            Self::TransactionRejected => -32003,
            Self::ExecutionError => 3,
            Self::InvalidInput => -32000,
            Self::ResourceNotFound => -32001,
            Self::UnknownBlock => -39001,
        }
    }
}
