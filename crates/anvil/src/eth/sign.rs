use crate::eth::error::BlockchainError;
use alloy_consensus::{Sealed, SignableTransaction, Signed};
use alloy_dyn_abi::TypedData;
use alloy_network::{Network, TxSignerSync};
use alloy_primitives::{Address, B256, Signature, map::AddressHashMap};
use alloy_signer::Signer as AlloySigner;
use alloy_signer_local::PrivateKeySigner;
use foundry_primitives::{FoundryTxEnvelope, FoundryTypedTx};
use tempo_primitives::TempoSignature;

/// A transaction signer, generic over the network.
///
/// Modelled after alloy's `NetworkWallet<N>`: the
/// [`sign_transaction_from`](Signer::sign_transaction_from) method takes an
/// unsigned transaction and returns the fully-signed envelope in one step.
#[async_trait::async_trait]
pub trait AnvilSigner<N: Network>: Send + Sync {
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
impl<N: Network> AnvilSigner<N> for DevSigner
where
    N::TxEnvelope: From<Signed<N::UnsignedTx>>,
    N::UnsignedTx: SignableTransaction<Signature>,
{
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

    fn sign_transaction_from(
        &self,
        sender: &Address,
        mut tx: N::UnsignedTx,
    ) -> Result<N::TxEnvelope, BlockchainError>
where {
        let signer = self.accounts.get(sender).ok_or(BlockchainError::NoSignerAvailable)?;
        let signature = signer.sign_transaction_sync(&mut tx)?;
        Ok(tx.into_signed(signature).into())
    }
}

/// Builds a TxEnvelope from UnsignedTx with a zeroed signature.
///
/// Used for impersonated accounts, where transactions are accepted without a valid signature.
pub fn build_impersonated(typed_tx: FoundryTypedTx) -> FoundryTxEnvelope {
    let signature = Signature::new(Default::default(), Default::default(), false);
    match typed_tx {
        FoundryTypedTx::Legacy(tx) => FoundryTxEnvelope::Legacy(tx.into_signed(signature)),
        FoundryTypedTx::Eip2930(tx) => FoundryTxEnvelope::Eip2930(tx.into_signed(signature)),
        FoundryTypedTx::Eip1559(tx) => FoundryTxEnvelope::Eip1559(tx.into_signed(signature)),
        FoundryTypedTx::Eip7702(tx) => FoundryTxEnvelope::Eip7702(tx.into_signed(signature)),
        FoundryTypedTx::Eip4844(tx) => FoundryTxEnvelope::Eip4844(tx.into_signed(signature)),
        FoundryTypedTx::Deposit(tx) => FoundryTxEnvelope::Deposit(Sealed::new(tx)),
        FoundryTypedTx::Tempo(tx) => {
            let tempo_sig: TempoSignature = signature.into();
            FoundryTxEnvelope::Tempo(tx.into_signed(tempo_sig))
        }
    }
}
