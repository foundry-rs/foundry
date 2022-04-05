use crate::eth::error::BlockchainError;
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{Address, Wallet},
};
use foundry_node_core::eth::transaction::{TypedTransaction, TypedTransactionRequest};
use std::collections::HashMap;

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

pub struct DevSigner {
    accounts: HashMap<Address, Wallet<SigningKey>>,
}

impl DevSigner {
    pub fn new(accounts: HashMap<Address, Wallet<SigningKey>>) -> Self {
        Self { accounts }
    }
}

impl Signer for DevSigner {
    fn accounts(&self) -> Vec<Address> {
        self.accounts.keys().copied().collect()
    }

    fn sign(
        &self,
        _request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<TypedTransaction, BlockchainError> {
        let _signer = self.accounts.get(address).ok_or(BlockchainError::NoSignerAvailable)?;

        todo!("Need to unify ethers_core and node_core types first")
    }
}
