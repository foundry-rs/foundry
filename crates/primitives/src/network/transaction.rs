use alloy_consensus::{
    BlobTransactionSidecar, BlobTransactionSidecarEip7594, BlobTransactionSidecarVariant,
};
use alloy_network::{AnyNetwork, Ethereum, Network, TransactionBuilder};
use alloy_primitives::{Address, B256, Signature, U256};
use alloy_rpc_types::SignedAuthorization;
use tempo_alloy::TempoNetwork;

/// Composite transaction builder trait for Foundry transactions.
///
/// This extends the base `TransactionBuilder` trait with the same methods as
/// [`alloy_network::TransactionBuilder4844`] for handling blob transaction sidecars, and
/// [`alloy_network::TransactionBuilder7702`] for handling EIP-7702 authorization lists.
///
/// By default, all methods have no-op implementations, so this can be implemented for any Network.
///
/// If the Network supports Eip4844 blob transactions implement these methods:
/// - [`FoundryTransactionBuilder::max_fee_per_blob_gas`]
/// - [`FoundryTransactionBuilder::set_max_fee_per_blob_gas`]
/// - [`FoundryTransactionBuilder::blob_versioned_hashes`]
/// - [`FoundryTransactionBuilder::set_blob_versioned_hashes`]
/// - [`FoundryTransactionBuilder::blob_sidecar`]
/// - [`FoundryTransactionBuilder::set_blob_sidecar`]
///
/// If the Network supports EIP-7702 authorization lists, implement these methods:
/// - [`FoundryTransactionBuilder::authorization_list`]
/// - [`FoundryTransactionBuilder::set_authorization_list`]
///
/// If the Network supports Tempo transactions, implement these methods:
/// - [`FoundryTransactionBuilder::set_fee_token`]
/// - [`FoundryTransactionBuilder::set_nonce_key`]
/// - [`FoundryTransactionBuilder::set_key_id`]
/// - [`FoundryTransactionBuilder::set_valid_before`]
/// - [`FoundryTransactionBuilder::set_valid_after`]
/// - [`FoundryTransactionBuilder::set_fee_payer_signature`]
pub trait FoundryTransactionBuilder<N: Network>: TransactionBuilder<N> {
    /// Reset gas limit
    fn reset_gas_limit(&mut self);

    /// Get the max fee per blob gas for the transaction.
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        None
    }

    /// Set the max fee per blob gas for the transaction.
    fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

    /// Builder-pattern method for setting max fee per blob gas.
    fn with_max_fee_per_blob_gas(mut self, max_fee_per_blob_gas: u128) -> Self {
        self.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
        self
    }

    /// Gets the EIP-4844 blob versioned hashes of the transaction.
    ///
    /// These may be set independently of the sidecar, e.g. when the sidecar
    /// has been pruned but the hashes are still needed for `eth_call`.
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        None
    }

    /// Sets the EIP-4844 blob versioned hashes of the transaction.
    fn set_blob_versioned_hashes(&mut self, _hashes: Vec<B256>) {}

    /// Builder-pattern method for setting the EIP-4844 blob versioned hashes.
    fn with_blob_versioned_hashes(mut self, hashes: Vec<B256>) -> Self {
        self.set_blob_versioned_hashes(hashes);
        self
    }

    /// Gets the blob sidecar (either EIP-4844 or EIP-7594 variant) of the transaction.
    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        None
    }

    /// Sets the blob sidecar (either EIP-4844 or EIP-7594 variant) of the transaction.
    ///
    /// Note: This will also set the versioned blob hashes accordingly:
    /// [BlobTransactionSidecarVariant::versioned_hashes]
    fn set_blob_sidecar(&mut self, _sidecar: BlobTransactionSidecarVariant) {}

    /// Builder-pattern method for setting the blob sidecar of the transaction.
    fn with_blob_sidecar(mut self, sidecar: BlobTransactionSidecarVariant) -> Self {
        self.set_blob_sidecar(sidecar);
        self
    }

    /// Gets the EIP-4844 blob sidecar if the current sidecar is of that variant.
    fn blob_sidecar_4844(&self) -> Option<&BlobTransactionSidecar> {
        self.blob_sidecar().and_then(|s| s.as_eip4844())
    }

    /// Sets the EIP-4844 blob sidecar of the transaction.
    fn set_blob_sidecar_4844(&mut self, sidecar: BlobTransactionSidecar) {
        self.set_blob_sidecar(BlobTransactionSidecarVariant::Eip4844(sidecar));
    }

    /// Builder-pattern method for setting the EIP-4844 blob sidecar of the transaction.
    fn with_blob_sidecar_4844(mut self, sidecar: BlobTransactionSidecar) -> Self {
        self.set_blob_sidecar_4844(sidecar);
        self
    }

    /// Gets the EIP-7594 blob sidecar if the current sidecar is of that variant.
    fn blob_sidecar_7594(&self) -> Option<&BlobTransactionSidecarEip7594> {
        self.blob_sidecar().and_then(|s| s.as_eip7594())
    }

    /// Sets the EIP-7594 blob sidecar of the transaction.
    fn set_blob_sidecar_7594(&mut self, sidecar: BlobTransactionSidecarEip7594) {
        self.set_blob_sidecar(BlobTransactionSidecarVariant::Eip7594(sidecar));
    }

    /// Builder-pattern method for setting the EIP-7594 blob sidecar of the transaction.
    fn with_blob_sidecar_7594(mut self, sidecar: BlobTransactionSidecarEip7594) -> Self {
        self.set_blob_sidecar_7594(sidecar);
        self
    }

    /// Get the EIP-7702 authorization list for the transaction.
    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        None
    }

    /// Sets the EIP-7702 authorization list.
    fn set_authorization_list(&mut self, _authorization_list: Vec<SignedAuthorization>) {}

    /// Builder-pattern method for setting the authorization list.
    fn with_authorization_list(mut self, authorization_list: Vec<SignedAuthorization>) -> Self {
        self.set_authorization_list(authorization_list);
        self
    }

    /// Get the fee token for a Tempo transaction.
    fn fee_token(&self) -> Option<Address> {
        None
    }

    /// Set the fee token for a Tempo transaction.
    fn set_fee_token(&mut self, _fee_token: Address) {}

    /// Builder-pattern method for setting the Tempo fee token.
    fn with_fee_token(mut self, fee_token: Address) -> Self {
        self.set_fee_token(fee_token);
        self
    }

    /// Get the 2D nonce key for a Tempo transaction.
    fn nonce_key(&self) -> Option<U256> {
        None
    }

    /// Set the 2D nonce key for the Tempo transaction.
    fn set_nonce_key(&mut self, _nonce_key: U256) {}

    /// Builder-pattern method for setting a 2D nonce key for a Tempo transaction.
    fn with_nonce_key(mut self, nonce_key: U256) -> Self {
        self.set_nonce_key(nonce_key);
        self
    }

    /// Get the access key ID for a Tempo transaction.
    fn key_id(&self) -> Option<Address> {
        None
    }

    /// Set the access key ID for a Tempo transaction.
    ///
    /// Used during gas estimation to override the key_id that would normally be
    /// recovered from the signature.
    fn set_key_id(&mut self, _key_id: Address) {}

    /// Builder-pattern method for setting the Tempo access key ID.
    fn with_key_id(mut self, key_id: Address) -> Self {
        self.set_key_id(key_id);
        self
    }

    /// Get the valid_before timestamp for a Tempo expiring nonce transaction.
    fn valid_before(&self) -> Option<u64> {
        None
    }

    /// Set the valid_before timestamp for a Tempo expiring nonce transaction.
    fn set_valid_before(&mut self, _valid_before: u64) {}

    /// Builder-pattern method for setting the valid_before timestamp.
    fn with_valid_before(mut self, valid_before: u64) -> Self {
        self.set_valid_before(valid_before);
        self
    }

    /// Get the valid_after timestamp for a Tempo expiring nonce transaction.
    fn valid_after(&self) -> Option<u64> {
        None
    }

    /// Set the valid_after timestamp for a Tempo expiring nonce transaction.
    fn set_valid_after(&mut self, _valid_after: u64) {}

    /// Builder-pattern method for setting the valid_after timestamp.
    fn with_valid_after(mut self, valid_after: u64) -> Self {
        self.set_valid_after(valid_after);
        self
    }

    /// Get the fee payer (sponsor) signature for a Tempo sponsored transaction.
    fn fee_payer_signature(&self) -> Option<Signature> {
        None
    }

    /// Set the fee payer (sponsor) signature for a Tempo sponsored transaction.
    fn set_fee_payer_signature(&mut self, _signature: Signature) {}

    /// Builder-pattern method for setting the fee payer signature.
    fn with_fee_payer_signature(mut self, signature: Signature) -> Self {
        self.set_fee_payer_signature(signature);
        self
    }

    /// Computes the sponsor (fee payer) signature hash for this transaction.
    ///
    /// This builds an unsigned consensus-level transaction from the request and computes
    /// the hash that a sponsor needs to sign. Returns `None` for networks that don't
    /// support sponsored transactions.
    fn compute_sponsor_hash(&self, _from: Address) -> Option<B256> {
        None
    }
}

impl FoundryTransactionBuilder<Ethereum> for <Ethereum as Network>::TransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.max_fee_per_blob_gas
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = Some(max_fee_per_blob_gas);
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.blob_versioned_hashes.as_deref()
    }

    fn set_blob_versioned_hashes(&mut self, hashes: Vec<B256>) {
        self.blob_versioned_hashes = Some(hashes);
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        self.sidecar.as_ref()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecarVariant) {
        self.sidecar = Some(sidecar);
        self.populate_blob_hashes();
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }
}

impl FoundryTransactionBuilder<AnyNetwork> for <AnyNetwork as Network>::TransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.max_fee_per_blob_gas
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = Some(max_fee_per_blob_gas);
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.blob_versioned_hashes.as_deref()
    }

    fn set_blob_versioned_hashes(&mut self, hashes: Vec<B256>) {
        self.blob_versioned_hashes = Some(hashes);
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        self.sidecar.as_ref()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecarVariant) {
        self.sidecar = Some(sidecar);
        self.populate_blob_hashes();
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }
}

impl FoundryTransactionBuilder<TempoNetwork> for <TempoNetwork as Network>::TransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }

    fn fee_token(&self) -> Option<Address> {
        self.fee_token
    }

    fn set_fee_token(&mut self, fee_token: Address) {
        self.fee_token = Some(fee_token);
    }

    fn nonce_key(&self) -> Option<U256> {
        self.nonce_key
    }

    fn set_nonce_key(&mut self, nonce_key: U256) {
        self.nonce_key = Some(nonce_key);
    }

    fn key_id(&self) -> Option<Address> {
        self.key_id
    }

    fn set_key_id(&mut self, key_id: Address) {
        self.key_id = Some(key_id);
    }

    fn valid_before(&self) -> Option<u64> {
        self.valid_before
    }

    fn set_valid_before(&mut self, valid_before: u64) {
        self.valid_before = Some(valid_before);
    }

    fn valid_after(&self) -> Option<u64> {
        self.valid_after
    }

    fn set_valid_after(&mut self, valid_after: u64) {
        self.valid_after = Some(valid_after);
    }

    fn fee_payer_signature(&self) -> Option<Signature> {
        self.fee_payer_signature
    }

    fn set_fee_payer_signature(&mut self, signature: Signature) {
        self.fee_payer_signature = Some(signature);
    }

    fn compute_sponsor_hash(&self, from: Address) -> Option<B256> {
        let tx = self.clone().build_aa().ok()?;
        Some(tx.fee_payer_signature_hash(from))
    }
}
