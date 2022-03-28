use ethers::{signers, types::Address};
use forge_node_core::{
    error::RpcError,
    eth::transaction::{TypedTransaction, TypedTransactionRequest},
};

/// A transaction signer
pub trait Signer: Send + Sync {
    /// returns the available accounts for this signer
    fn accounts(&self) -> Vec<Address>;
    /// signs a transaction request using the given account in request
    fn sign(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<TypedTransaction, RpcError>;
}
