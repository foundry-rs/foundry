use crate::eth::error::BlockchainError;
use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_network::{Network, TxSignerSync};
use alloy_primitives::{Address, B256, Signature, map::AddressHashMap};
use alloy_signer::Signer as AlloySigner;
use alloy_signer_local::PrivateKeySigner;
use foundry_primitives::{FoundryTxEnvelope, FoundryTypedTx};

/// Network-agnostic signing: messages, typed data, and hashes.
#[async_trait::async_trait]
pub trait MessageSigner: Send + Sync {
    /// returns the available accounts for this signer
    fn accounts(&self) -> Vec<Address>;

    /// Returns `true` whether this signer can sign for this address
    fn is_signer_for(&self, addr: Address) -> bool {
        self.accounts().contains(&addr)
    }

    /// Returns the signature
    async fn sign(&self, address: Address, message: &[u8]) -> Result<Signature, BlockchainError>;

    /// Encodes and signs the typed data according EIP-712. Payload must conform to the EIP-712
    /// standard.
    async fn sign_typed_data(
        &self,
        address: Address,
        payload: &TypedData,
    ) -> Result<Signature, BlockchainError>;

    /// Signs the given hash.
    async fn sign_hash(&self, address: Address, hash: B256) -> Result<Signature, BlockchainError>;
}

/// A transaction signer, generic over the network.
///
/// Modelled after alloy's `NetworkWallet<N>`: the
/// [`sign_transaction_from`](Signer::sign_transaction_from) method takes an
/// unsigned transaction and returns the fully-signed envelope in one step.
pub trait Signer<N: Network>: MessageSigner {
    /// Signs an unsigned transaction and returns the signed envelope.
    ///
    /// Mirrors `NetworkWallet::sign_transaction_from`.
    fn sign_transaction_from(
        &self,
        sender: &Address,
        tx: N::UnsignedTx,
    ) -> Result<N::TxEnvelope, BlockchainError>;
}

/// Maintains developer keys
pub struct DevSigner {
    addresses: Vec<Address>,
    accounts: AddressHashMap<PrivateKeySigner>,
}

impl DevSigner {
    pub fn new(accounts: Vec<PrivateKeySigner>) -> Self {
        let addresses = accounts.iter().map(|wallet| wallet.address()).collect::<Vec<_>>();
        let accounts = addresses.iter().copied().zip(accounts).collect();
        Self { addresses, accounts }
    }
}

#[async_trait::async_trait]
impl MessageSigner for DevSigner {
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
        address: Address,
        payload: &TypedData,
    ) -> Result<Signature, BlockchainError> {
        let mut signer =
            self.accounts.get(&address).ok_or(BlockchainError::NoSignerAvailable)?.to_owned();

        // Explicitly set chainID as none, to avoid any EIP-155 application to `v` when signing
        // typed data.
        signer.set_chain_id(None);

        Ok(signer.sign_dynamic_typed_data(payload).await?)
    }

    async fn sign_hash(&self, address: Address, hash: B256) -> Result<Signature, BlockchainError> {
        let signer = self.accounts.get(&address).ok_or(BlockchainError::NoSignerAvailable)?;

        Ok(signer.sign_hash(&hash).await?)
    }
}

impl Signer<foundry_primitives::FoundryNetwork> for DevSigner {
    fn sign_transaction_from(
        &self,
        sender: &Address,
        tx: FoundryTypedTx,
    ) -> Result<FoundryTxEnvelope, BlockchainError> {
        let mut signer =
            self.accounts.get(sender).ok_or(BlockchainError::NoSignerAvailable)?.clone();
        // The transaction is authoritative for its chain ID. Developer wallets are created with
        // the node's initial chain ID, which can later change through a reset or
        // `anvil_setChainId`.
        signer.set_chain_id(None);
        let envelope = match tx {
            FoundryTypedTx::Legacy(mut t) => {
                let sig = signer.sign_transaction_sync(&mut t)?;
                FoundryTxEnvelope::Legacy(t.into_signed(sig))
            }
            FoundryTypedTx::Eip2930(mut t) => {
                let sig = signer.sign_transaction_sync(&mut t)?;
                FoundryTxEnvelope::Eip2930(t.into_signed(sig))
            }
            FoundryTypedTx::Eip1559(mut t) => {
                let sig = signer.sign_transaction_sync(&mut t)?;
                FoundryTxEnvelope::Eip1559(t.into_signed(sig))
            }
            FoundryTypedTx::Eip7702(mut t) => {
                let sig = signer.sign_transaction_sync(&mut t)?;
                FoundryTxEnvelope::Eip7702(t.into_signed(sig))
            }
            FoundryTypedTx::Eip4844(mut t) => {
                let sig = signer.sign_transaction_sync(&mut t)?;
                FoundryTxEnvelope::Eip4844(t.into_signed(sig))
            }
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTypedTx::Deposit(_) => {
                unreachable!("op deposit txs should not be signed")
            }
            #[cfg(feature = "optimism")]
            FoundryTypedTx::PostExec(_) => {
                unreachable!("op post-exec txs should not be signed")
            }
            #[cfg(feature = "base")]
            FoundryTypedTx::Eip8130(_) => {
                unreachable!("EIP-8130 requires a signed raw transaction envelope")
            }
            FoundryTypedTx::Tempo(mut t) => {
                let sig = signer.sign_transaction_sync(&mut t)?;
                FoundryTxEnvelope::Tempo(t.into_signed(sig.into()))
            }
        };
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TxLegacy;

    #[test]
    fn dev_signer_uses_transaction_chain_id() {
        let mut account = PrivateKeySigner::random();
        account.set_chain_id(Some(1));
        let sender = account.address();
        let signer = DevSigner::new(vec![account]);
        let tx = TxLegacy { chain_id: Some(56), ..Default::default() };

        let FoundryTxEnvelope::Legacy(signed) =
            signer.sign_transaction_from(&sender, FoundryTypedTx::Legacy(tx)).unwrap()
        else {
            panic!("expected legacy transaction")
        };

        assert_eq!(signed.tx().chain_id, Some(56));
        assert_eq!(signed.recover_signer().unwrap(), sender);
    }
}
