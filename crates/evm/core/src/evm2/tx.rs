//! Network-neutral transaction environments.

use alloy_consensus::{TxEip1559, TxEip2930, TxEip4844, TxEip7702, TxLegacy};
use alloy_eips::{
    eip2930::AccessList,
    eip7702::{RecoveredAuthorization, SignedAuthorization},
};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256};
use evm2::ethereum::{LazyAuthorization, LazyTxEip7702, TxEnvelope};
use std::fmt::Debug;

/// Successor of the REVM `TxEnv` family behind [`crate::FoundryTransaction`].
///
/// Cheatcodes, scripts and fork replay mutate this flat description. The factory turns it into the
/// network's evm2 transaction with [`FoundryEvmFactory::build_tx`](super::FoundryEvmFactory) right
/// before execution, so Foundry code never edits typed envelopes. Network-only fields live in
/// [`Self::ext`] instead of defaulted methods on a shared trait.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundryTxEnv<E = ()> {
    /// EIP-2718 transaction type.
    pub tx_type: u8,
    /// Sender.
    pub caller: Address,
    /// Gas limit.
    pub gas_limit: u64,
    /// Gas price, or max fee per gas for dynamic-fee transactions.
    pub gas_price: u128,
    /// Call target or create.
    pub kind: TxKind,
    /// Value sent with the transaction.
    pub value: U256,
    /// Calldata or initcode.
    pub data: Bytes,
    /// Sender nonce.
    pub nonce: u64,
    /// Chain ID, `None` for pre-EIP-155 legacy transactions.
    pub chain_id: Option<u64>,
    /// EIP-2930 access list.
    pub access_list: AccessList,
    /// Max priority fee per gas.
    pub gas_priority_fee: Option<u128>,
    /// EIP-4844 blob versioned hashes.
    pub blob_hashes: Vec<B256>,
    /// EIP-4844 max fee per blob gas.
    pub max_fee_per_blob_gas: u128,
    /// EIP-7702 authorizations, signed or already recovered.
    pub authorization_list: Vec<LazyAuthorization>,
    /// Network-specific fields.
    pub ext: E,
}

impl<E: FoundryTxExt> FoundryTxEnv<E> {
    /// Returns the transaction type.
    pub const fn tx_type(&self) -> u8 {
        self.tx_type
    }

    /// Returns the sender.
    pub const fn caller(&self) -> Address {
        self.caller
    }

    /// Returns the call target or create.
    pub const fn kind(&self) -> TxKind {
        self.kind
    }

    /// Returns the calldata or initcode.
    pub const fn input(&self) -> &Bytes {
        &self.data
    }

    /// Sets the transaction type.
    pub const fn set_tx_type(&mut self, tx_type: u8) {
        self.tx_type = tx_type;
    }

    /// Sets the sender.
    pub const fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }

    /// Sets the gas limit.
    pub const fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }

    /// Sets the gas price, or max fee per gas for dynamic-fee transactions.
    pub const fn set_gas_price(&mut self, gas_price: u128) {
        self.gas_price = gas_price;
    }

    /// Sets the call target or create, keeping network-specific call lists in sync.
    pub fn set_kind(&mut self, kind: TxKind) {
        self.kind = kind;
        self.ext.on_call_change(kind, self.value, &self.data);
    }

    /// Sets the value, keeping network-specific call lists in sync.
    pub fn set_value(&mut self, value: U256) {
        self.value = value;
        self.ext.on_call_change(self.kind, value, &self.data);
    }

    /// Sets the calldata or initcode, keeping network-specific call lists in sync.
    pub fn set_data(&mut self, data: Bytes) {
        self.data = data;
        self.ext.on_call_change(self.kind, self.value, &self.data);
    }

    /// Sets the sender nonce.
    pub const fn set_nonce(&mut self, nonce: u64) {
        self.nonce = nonce;
    }

    /// Sets the chain ID.
    pub const fn set_chain_id(&mut self, chain_id: Option<u64>) {
        self.chain_id = chain_id;
    }

    /// Sets the access list.
    pub fn set_access_list(&mut self, access_list: AccessList) {
        self.access_list = access_list;
    }

    /// Returns the EIP-7702 authorization list.
    pub const fn authorization_list_mut(&mut self) -> &mut Vec<LazyAuthorization> {
        &mut self.authorization_list
    }

    /// Replaces the EIP-7702 authorization list with signed authorizations.
    pub fn set_signed_authorization(&mut self, auth: Vec<SignedAuthorization>) {
        self.authorization_list = auth.into_iter().map(Into::into).collect();
    }

    /// Replaces the EIP-7702 authorization list with recovered authorizations.
    pub fn set_recovered_authorization(&mut self, auth: Vec<RecoveredAuthorization>) {
        self.authorization_list = auth.into_iter().map(Into::into).collect();
    }

    /// Sets the max priority fee per gas.
    pub const fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
        self.gas_priority_fee = gas_priority_fee;
    }

    /// Sets the blob versioned hashes.
    pub fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
        self.blob_hashes = blob_hashes;
    }

    /// Sets the max fee per blob gas.
    pub const fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = max_fee_per_blob_gas;
    }

    /// Returns the fee token, for networks with token-denominated fees.
    pub fn fee_token(&self) -> Option<Address> {
        self.ext.fee_token()
    }

    /// Sets the fee token. A no-op on networks without token-denominated fees.
    pub fn set_fee_token(&mut self, token: Option<Address>) {
        self.ext.set_fee_token(token);
    }

    /// Returns the sponsored fee payer, for networks with fee sponsorship.
    pub fn fee_payer(&self) -> Option<Option<Address>> {
        self.ext.fee_payer()
    }

    /// Sets the sponsored fee payer. A no-op on networks without fee sponsorship.
    pub fn set_fee_payer(&mut self, payer: Option<Option<Address>>) {
        self.ext.set_fee_payer(payer);
    }

    /// Returns whether this is an OP-stack deposit transaction.
    pub fn is_deposit(&self) -> bool {
        self.ext.is_deposit(self.tx_type)
    }

    /// Returns whether this transaction executes a single contract creation.
    pub fn is_single_create(&self) -> bool {
        self.kind.is_create() && self.ext.call_count() <= 1
    }
}

/// Network-specific fields of a [`FoundryTxEnv`].
///
/// Every method defaults to the Ethereum behavior, so networks override only what they carry.
pub trait FoundryTxExt: Clone + Debug + Default + PartialEq + Send + Sync + 'static {
    /// Returns the fee token.
    fn fee_token(&self) -> Option<Address> {
        None
    }

    /// Sets the fee token.
    fn set_fee_token(&mut self, _token: Option<Address>) {}

    /// Returns the sponsored fee payer.
    fn fee_payer(&self) -> Option<Option<Address>> {
        None
    }

    /// Sets the sponsored fee payer.
    fn set_fee_payer(&mut self, _payer: Option<Option<Address>>) {}

    /// Returns whether a transaction of `tx_type` is an OP-stack deposit.
    fn is_deposit(&self, _tx_type: u8) -> bool {
        false
    }

    /// Returns how many calls the transaction executes.
    fn call_count(&self) -> usize {
        1
    }

    /// Mirrors a change of the primary call into network-specific call lists.
    fn on_call_change(&mut self, _kind: TxKind, _value: U256, _data: &Bytes) {}
}

impl FoundryTxExt for () {}

/// EIP-2718 type of OP-stack deposit transactions.
pub const DEPOSIT_TX_TYPE: u8 = 0x7E;

/// OP-stack fields of a [`FoundryTxEnv`], shared by OP and Base.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepositTxExt {
    /// EIP-2718 bytes the L1 data fee is priced over. `None` prices simulations at zero.
    pub enveloped_tx: Option<Bytes>,
    /// Deposit source hash.
    pub source_hash: B256,
    /// ETH minted by a deposit.
    pub mint: Option<u128>,
    /// Whether a deposit is a pre-Regolith system transaction.
    pub is_system_transaction: bool,
}

impl FoundryTxExt for DepositTxExt {
    fn is_deposit(&self, tx_type: u8) -> bool {
        tx_type == DEPOSIT_TX_TYPE
    }
}

/// Builds the standard Ethereum transaction described by `tx`.
///
/// Networks whose evm2 envelope embeds Ethereum transactions build on this helper.
pub fn build_eth_tx<E>(tx: &FoundryTxEnv<E>) -> eyre::Result<TxEnvelope> {
    let to_call = || match tx.kind {
        TxKind::Call(to) => Ok(to),
        TxKind::Create => eyre::bail!("transaction type {} cannot create contracts", tx.tx_type),
    };
    Ok(match tx.tx_type {
        0 => TxEnvelope::Legacy(TxLegacy {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_price: tx.gas_price,
            gas_limit: tx.gas_limit,
            to: tx.kind,
            value: tx.value,
            input: tx.data.clone(),
        }),
        1 => TxEnvelope::Eip2930(TxEip2930 {
            chain_id: tx.chain_id.unwrap_or_default(),
            nonce: tx.nonce,
            gas_price: tx.gas_price,
            gas_limit: tx.gas_limit,
            to: tx.kind,
            value: tx.value,
            access_list: tx.access_list.clone(),
            input: tx.data.clone(),
        }),
        2 => TxEnvelope::Eip1559(TxEip1559 {
            chain_id: tx.chain_id.unwrap_or_default(),
            nonce: tx.nonce,
            gas_limit: tx.gas_limit,
            max_fee_per_gas: tx.gas_price,
            max_priority_fee_per_gas: tx.gas_priority_fee.unwrap_or_default(),
            to: tx.kind,
            value: tx.value,
            access_list: tx.access_list.clone(),
            input: tx.data.clone(),
        }),
        3 => TxEnvelope::Eip4844(
            TxEip4844 {
                chain_id: tx.chain_id.unwrap_or_default(),
                nonce: tx.nonce,
                gas_limit: tx.gas_limit,
                max_fee_per_gas: tx.gas_price,
                max_priority_fee_per_gas: tx.gas_priority_fee.unwrap_or_default(),
                to: to_call()?,
                value: tx.value,
                access_list: tx.access_list.clone(),
                blob_versioned_hashes: tx.blob_hashes.clone(),
                max_fee_per_blob_gas: tx.max_fee_per_blob_gas,
                input: tx.data.clone(),
            }
            .into(),
        ),
        4 => TxEnvelope::Eip7702(LazyTxEip7702::from_authorizations(
            TxEip7702 {
                chain_id: tx.chain_id.unwrap_or_default(),
                nonce: tx.nonce,
                gas_limit: tx.gas_limit,
                max_fee_per_gas: tx.gas_price,
                max_priority_fee_per_gas: tx.gas_priority_fee.unwrap_or_default(),
                to: to_call()?,
                value: tx.value,
                access_list: tx.access_list.clone(),
                authorization_list: Vec::new(),
                input: tx.data.clone(),
            },
            tx.authorization_list.clone(),
        )),
        ty => eyre::bail!("unsupported Ethereum transaction type {ty:#x}"),
    })
}
