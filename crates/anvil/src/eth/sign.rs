use crate::eth::error::BlockchainError;

use alloy_network::{Signed, Transaction};
use alloy_primitives::{Address, Signature, B256, U256};
use alloy_signer::{LocalWallet, Signer as AlloySigner, SignerSync as AlloySignerSync};
use alloy_sol_types::Eip712Domain;
use anvil_core::eth::transaction::{
    alloy::{TypedTransaction, TypedTransactionRequest},
    optimism::{DepositTransaction, DepositTransactionRequest},
};
use std::collections::HashMap;

/// A transaction signer
#[async_trait::async_trait]
pub trait Signer: Send + Sync {
    /// returns the available accounts for this signer
    fn accounts(&self) -> Vec<Address>;

    /// Returns `true` whether this signer can sign for this address
    fn is_signer_for(&self, addr: Address) -> bool {
        self.accounts().contains(&addr)
    }

    /// Returns the signature
    async fn sign(&self, address: Address, message: &[u8]) -> Result<Signature, BlockchainError>;

    /// Encodes and signs the typed data according EIP-712. Payload must be a SolStruct.
    async fn sign_typed_data(
        &self,
        address: Address,
        payload: &Eip712Domain,
    ) -> Result<Signature, BlockchainError>;

    /// Signs the given hash.
    async fn sign_hash(&self, address: Address, hash: B256) -> Result<Signature, BlockchainError>;

    /// signs a transaction request using the given account in request
    fn sign_transaction(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<Signature, BlockchainError>;
}

/// Maintains developer keys
pub struct DevSigner {
    addresses: Vec<Address>,
    accounts: HashMap<Address, LocalWallet>,
}

impl DevSigner {
    pub fn new(accounts: Vec<LocalWallet>) -> Self {
        let addresses = accounts.iter().map(|wallet| wallet.address()).collect::<Vec<_>>();
        let accounts = addresses.iter().cloned().zip(accounts).collect();
        Self { addresses, accounts }
    }
}

#[async_trait::async_trait]
impl Signer for DevSigner {
    fn accounts(&self) -> Vec<Address> {
        self.addresses.clone()
    }

    fn is_signer_for(&self, addr: Address) -> bool {
        self.accounts.contains_key(&addr)
    }

    async fn sign(&self, address: Address, message: &[u8]) -> Result<Signature, BlockchainError> {
        let signer = self.accounts.get(&address).ok_or(BlockchainError::NoSignerAvailable)?;

        Ok(signer.sign_message(message).await?)
    }

    async fn sign_typed_data(
        &self,
        _address: Address,
        _payload: &Eip712Domain,
    ) -> Result<Signature, BlockchainError> {
        Err(BlockchainError::RpcUnimplemented)
        // let signer = self.accounts.get(&address).ok_or(BlockchainError::NoSignerAvailable)?;
        // Ok(signer.sign_typed_data(payload).await?)
    }

    async fn sign_hash(&self, address: Address, hash: B256) -> Result<Signature, BlockchainError> {
        let signer = self.accounts.get(&address).ok_or(BlockchainError::NoSignerAvailable)?;

        Ok(signer.sign_hash(hash).await?)
    }

    fn sign_transaction(
        &self,
        request: TypedTransactionRequest,
        address: &Address,
    ) -> Result<Signature, BlockchainError> {
        let signer = self.accounts.get(address).ok_or(BlockchainError::NoSignerAvailable)?;
        match request {
            TypedTransactionRequest::Legacy(mut tx) => Ok(signer.sign_transaction_sync(&mut tx)?),
            TypedTransactionRequest::EIP2930(mut tx) => Ok(signer.sign_transaction_sync(&mut tx)?),
            TypedTransactionRequest::EIP1559(mut tx) => Ok(signer.sign_transaction_sync(&mut tx)?),
            TypedTransactionRequest::Deposit(mut tx) => Ok(signer.sign_transaction_sync(&mut tx)?),
        }
    }
}

/// converts the `request` into a [`TypedTransactionRequest`] with the given signature
///
/// # Errors
///
/// This will fail if the `signature` contains an erroneous recovery id.
pub fn build_typed_transaction(
    request: TypedTransactionRequest,
    signature: Signature,
) -> Result<TypedTransaction, BlockchainError> {
    let tx = match request {
        TypedTransactionRequest::Legacy(tx) => {
            let sighash = tx.signature_hash();
            TypedTransaction::Legacy(Signed::new_unchecked(tx, signature, sighash))
        }
        TypedTransactionRequest::EIP2930(tx) => {
            let sighash = tx.signature_hash();
            TypedTransaction::EIP2930(Signed::new_unchecked(tx, signature, sighash))
        }
        TypedTransactionRequest::EIP1559(tx) => {
            let sighash = tx.signature_hash();
            TypedTransaction::EIP1559(Signed::new_unchecked(tx, signature, sighash))
        }
        TypedTransactionRequest::Deposit(tx) => {
            let DepositTransactionRequest {
                from,
                gas_limit,
                kind,
                value,
                input,
                source_hash,
                mint,
                is_system_tx,
                ..
            } = tx;
            TypedTransaction::Deposit(DepositTransaction {
                from,
                gas_limit,
                kind,
                value,
                input,
                source_hash,
                mint,
                is_system_tx,
                nonce: U256::ZERO,
            })
        }
    };

    Ok(tx)
}
