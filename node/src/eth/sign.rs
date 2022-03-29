use crate::eth::error::BlockchainError;
use ethers::types::Address;
use forge_node_core::eth::transaction::{TypedTransaction, TypedTransactionRequest};

/// A transaction signer
pub trait Signer: Send + Sync {
    /// returns the available accounts for this signer
    fn accounts(&self) -> Vec<Address>;
    /// signs a transaction request using the given account in request
    fn sign(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<TypedTransaction, BlockchainError>;
}

// TODO implement a dev signer
