use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_network::TxSigner;
use alloy_primitives::{Address, B256, Bytes, Signature, U256};
use alloy_signer::Result as SignerResult;
use jsonrpsee::core::async_trait;
use parking_lot::RwLock;
use reth_ethereum::rpc::eth::SignError;
use reth_rpc_eth_api::{SignableTxRequest, helpers::EthSigner};
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    sync::Arc,
};

/// Shared impersonation state, accessible from the pool validator, the engine EVM config, and the
/// RPC layer.
#[derive(Debug, Clone, Default)]
pub struct ImpersonationState {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    accounts: HashSet<Address>,
    auto_impersonate: bool,
    /// Maps impersonated transaction hashes to the intended sender so the engine can attribute
    /// them during payload execution.
    tx_senders: HashMap<B256, Address>,
    /// Transactions that a revert removed from the chain. The pool rejects them so a reorg does
    /// not bring them back.
    dropped_txs: HashSet<B256>,
    /// Signatures that recover to a chosen sender, set by `anvil_impersonateSignature`.
    signature_overrides: HashMap<Bytes, Address>,
}

impl ImpersonationState {
    /// Makes every transaction carrying `signature` recover to `address`.
    pub fn add_signature_override(&self, signature: Bytes, address: Address) {
        self.inner.write().signature_overrides.insert(signature, address);
    }

    /// Returns whether any signature overrides are set.
    pub fn has_signature_overrides(&self) -> bool {
        !self.inner.read().signature_overrides.is_empty()
    }

    /// Returns the sender a signature override assigns to `signature`, if any.
    pub fn signature_override(&self, signature: &Signature) -> Option<Address> {
        self.signature_override_raw(&signature.as_bytes())
    }

    /// Returns the sender a signature override assigns to the raw 65-byte `signature`, if any.
    pub fn signature_override_raw(&self, signature: &[u8]) -> Option<Address> {
        let inner = self.inner.read();
        if inner.signature_overrides.is_empty() {
            return None;
        }
        inner.signature_overrides.get(signature).copied()
    }

    /// Marks transactions that a revert removed from the chain.
    pub fn drop_txs(&self, hashes: impl IntoIterator<Item = B256>) {
        self.inner.write().dropped_txs.extend(hashes);
    }

    /// Returns whether a revert removed the given transaction from the chain.
    pub fn is_dropped(&self, hash: &B256) -> bool {
        self.inner.read().dropped_txs.contains(hash)
    }

    /// Starts impersonating the given account.
    pub fn impersonate(&self, address: Address) {
        self.inner.write().accounts.insert(address);
    }

    /// Stops impersonating the given account.
    pub fn stop_impersonating(&self, address: Address) {
        self.inner.write().accounts.remove(&address);
    }

    /// Enables or disables impersonation of every account.
    pub fn set_auto_impersonate(&self, enabled: bool) {
        self.inner.write().auto_impersonate = enabled;
    }

    /// Returns whether the given account is impersonated.
    pub fn is_impersonated(&self, address: &Address) -> bool {
        let inner = self.inner.read();
        inner.auto_impersonate || inner.accounts.contains(address)
    }

    /// Returns the explicitly impersonated accounts.
    pub fn impersonated_accounts(&self) -> Vec<Address> {
        self.inner.read().accounts.iter().copied().collect()
    }

    /// Records the intended sender of an impersonated transaction.
    pub fn remember_tx_sender(&self, hash: B256, sender: Address) {
        self.inner.write().tx_senders.insert(hash, sender);
    }

    /// Forgets the recorded sender of a transaction.
    pub fn forget_tx_sender(&self, hash: &B256) {
        self.inner.write().tx_senders.remove(hash);
    }

    /// Forgets the recorded senders of the given transactions.
    pub fn forget_tx_senders(&self, hashes: impl IntoIterator<Item = B256>) {
        let mut inner = self.inner.write();
        for hash in hashes {
            inner.tx_senders.remove(&hash);
        }
    }

    /// Returns the recorded sender of an impersonated transaction.
    pub fn tx_sender(&self, hash: &B256) -> Option<Address> {
        self.inner.read().tx_senders.get(hash).copied()
    }
}

/// Signs transaction requests with a placeholder signature while preserving the requested sender
/// through the surrounding `Recovered<T>` wrapper.
#[derive(Debug, Clone, Copy)]
struct ImpersonatedTxSigner {
    address: Address,
}

#[async_trait]
impl TxSigner<Signature> for ImpersonatedTxSigner {
    fn address(&self) -> Address {
        self.address
    }

    async fn sign_transaction(
        &self,
        _tx: &mut dyn SignableTransaction<Signature>,
    ) -> SignerResult<Signature> {
        // The sender in `r` gives the transactions of different impersonated senders different
        // hashes, as anvil's impersonated hash does; a zero `s` never recovers to an address.
        Ok(Signature::new(U256::from_be_slice(self.address.as_slice()), U256::ZERO, false))
    }
}

/// Dynamic signer that becomes available for any explicitly or automatically impersonated
/// account.
#[derive(Debug)]
pub struct ImpersonatedSigner<T, TxReq> {
    state: ImpersonationState,
    marker: PhantomData<fn() -> (T, TxReq)>,
}

impl<T, TxReq> ImpersonatedSigner<T, TxReq> {
    /// Creates a signer backed by the given impersonation state.
    pub fn new(state: ImpersonationState) -> Self {
        Self { state, marker: PhantomData }
    }
}

impl<T, TxReq> Clone for ImpersonatedSigner<T, TxReq> {
    fn clone(&self) -> Self {
        Self { state: self.state.clone(), marker: PhantomData }
    }
}

#[async_trait]
impl<T, TxReq> EthSigner<T, TxReq> for ImpersonatedSigner<T, TxReq>
where
    T: Send + Sync + 'static,
    TxReq: SignableTxRequest<T> + Send + Sync + 'static,
{
    fn accounts(&self) -> Vec<Address> {
        self.state.impersonated_accounts()
    }

    fn is_signer_for(&self, address: &Address) -> bool {
        self.state.is_impersonated(address)
    }

    async fn sign(&self, _address: Address, _message: &[u8]) -> Result<Signature, SignError> {
        Err(SignError::CouldNotSign)
    }

    async fn sign_transaction(&self, request: TxReq, address: &Address) -> Result<T, SignError> {
        request
            .try_build_and_sign(ImpersonatedTxSigner { address: *address })
            .await
            .map_err(|_| SignError::InvalidTransactionRequest)
    }

    fn sign_typed_data(
        &self,
        _address: Address,
        _payload: &TypedData,
    ) -> Result<Signature, SignError> {
        Err(SignError::CouldNotSign)
    }
}
