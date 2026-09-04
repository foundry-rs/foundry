use std::fmt::Debug;

use alloy_chains::NamedChain;
use alloy_consensus::{Transaction as _, Typed2718};
pub use alloy_evm::EvmEnv;
use alloy_evm::FromRecoveredTx;
use alloy_network::{AnyRpcTransaction, AnyTxEnvelope, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, U256};
use foundry_evm_networks::celo::CELO_DYNAMIC_FEE_TX_TYPE;
#[cfg(feature = "optimism")]
use op_revm::transaction::deposit::DEPOSIT_TRANSACTION_TYPE;
use revm::{
    Context, Database, Journal,
    context::{Block, BlockEnv, Cfg, CfgEnv, Transaction, TxEnv},
    context_interface::{
        ContextTr,
        either::Either,
        transaction::{AccessList, RecoveredAuthorization, SignedAuthorization},
    },
    inspector::JournalExt,
    primitives::{TxKind, hardfork::SpecId},
};
use tempo_revm::{TempoBlockEnv, TempoTxEnv};

use crate::backend::JournaledState;

/// Extension of [`Block`] with mutable setters, allowing EVM-agnostic mutation of block fields.
pub trait FoundryBlock: Block {
    /// Sets the block number.
    fn set_number(&mut self, number: U256);

    /// Sets the beneficiary (coinbase) address.
    fn set_beneficiary(&mut self, beneficiary: Address);

    /// Sets the block timestamp.
    fn set_timestamp(&mut self, timestamp: U256);

    /// Sets the gas limit.
    fn set_gas_limit(&mut self, gas_limit: u64);

    /// Sets the base fee per gas.
    fn set_basefee(&mut self, basefee: u64);

    /// Sets the block difficulty.
    fn set_difficulty(&mut self, difficulty: U256);

    /// Sets the prevrandao value.
    fn set_prevrandao(&mut self, prevrandao: Option<B256>);

    /// Sets the excess blob gas and blob gasprice.
    fn set_blob_excess_gas_and_price(
        &mut self,
        _excess_blob_gas: u64,
        _base_fee_update_fraction: u64,
    );

    // Tempo methods

    /// Returns the milliseconds portion of the block timestamp.
    fn timestamp_millis_part(&self) -> u64 {
        0
    }

    /// Sets the milliseconds portion of the block timestamp.
    fn set_timestamp_millis_part(&mut self, _millis: u64) {}
}

impl FoundryBlock for BlockEnv {
    fn set_number(&mut self, number: U256) {
        self.number = number;
    }

    fn set_beneficiary(&mut self, beneficiary: Address) {
        self.beneficiary = beneficiary;
    }

    fn set_timestamp(&mut self, timestamp: U256) {
        self.timestamp = timestamp;
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }

    fn set_basefee(&mut self, basefee: u64) {
        self.basefee = basefee;
    }

    fn set_difficulty(&mut self, difficulty: U256) {
        self.difficulty = difficulty;
    }

    fn set_prevrandao(&mut self, prevrandao: Option<B256>) {
        self.prevrandao = prevrandao;
    }

    fn set_blob_excess_gas_and_price(
        &mut self,
        excess_blob_gas: u64,
        base_fee_update_fraction: u64,
    ) {
        self.set_blob_excess_gas_and_price(excess_blob_gas, base_fee_update_fraction);
    }
}

impl FoundryBlock for TempoBlockEnv {
    fn set_number(&mut self, number: U256) {
        self.inner.set_number(number);
    }

    fn set_beneficiary(&mut self, beneficiary: Address) {
        self.inner.set_beneficiary(beneficiary);
    }

    fn set_timestamp(&mut self, timestamp: U256) {
        self.inner.set_timestamp(timestamp);
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.inner.set_gas_limit(gas_limit);
    }

    fn set_basefee(&mut self, basefee: u64) {
        self.inner.set_basefee(basefee);
    }

    fn set_difficulty(&mut self, difficulty: U256) {
        self.inner.set_difficulty(difficulty);
    }

    fn set_prevrandao(&mut self, prevrandao: Option<B256>) {
        self.inner.set_prevrandao(prevrandao);
    }

    fn set_blob_excess_gas_and_price(
        &mut self,
        _excess_blob_gas: u64,
        _base_fee_update_fraction: u64,
    ) {
    }

    fn timestamp_millis_part(&self) -> u64 {
        self.timestamp_millis_part
    }

    fn set_timestamp_millis_part(&mut self, millis: u64) {
        self.timestamp_millis_part = millis;
    }
}

/// Extension of [`Transaction`] with mutable setters, allowing EVM-agnostic mutation of transaction
/// fields.
pub trait FoundryTransaction: Transaction {
    /// Sets the transaction type.
    fn set_tx_type(&mut self, tx_type: u8);

    /// Sets the caller (sender) address.
    fn set_caller(&mut self, caller: Address);

    /// Sets the gas limit.
    fn set_gas_limit(&mut self, gas_limit: u64);

    /// Sets the gas price (or max fee per gas for EIP-1559).
    fn set_gas_price(&mut self, gas_price: u128);

    /// Sets the transaction kind (call or create).
    fn set_kind(&mut self, kind: TxKind);

    /// Sets the value sent with the transaction.
    fn set_value(&mut self, value: U256);

    /// Sets the transaction input data.
    fn set_data(&mut self, data: Bytes);

    /// Sets the nonce.
    fn set_nonce(&mut self, nonce: u64);

    /// Sets the chain ID.
    fn set_chain_id(&mut self, chain_id: Option<u64>);

    /// Sets the access list.
    fn set_access_list(&mut self, access_list: AccessList);

    /// Returns a mutable reference to the EIP-7702 authorization list.
    fn authorization_list_mut(
        &mut self,
    ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>>;

    /// Sets the max priority fee per gas.
    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>);

    /// Sets the blob versioned hashes.
    fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>);

    /// Sets the max fee per blob gas.
    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128);

    /// Sets the EIP-7702 signed authorization list.
    fn set_signed_authorization(&mut self, auth: Vec<SignedAuthorization>) {
        *self.authorization_list_mut() = auth.into_iter().map(Either::Left).collect();
    }

    // `OpTransaction` methods

    /// Enveloped transaction bytes.
    fn enveloped_tx(&self) -> Option<&Bytes> {
        None
    }

    /// Set Enveloped transaction bytes.
    fn set_enveloped_tx(&mut self, _bytes: Bytes) {}

    /// Source hash of the deposit transaction.
    fn source_hash(&self) -> Option<B256> {
        None
    }

    /// Sets source hash of the deposit transaction.
    fn set_source_hash(&mut self, _source_hash: B256) {}

    /// Mint of the deposit transaction
    fn mint(&self) -> Option<u128> {
        None
    }

    /// Sets mint of the deposit transaction.
    fn set_mint(&mut self, _mint: u128) {}

    /// Whether the transaction is a system transaction
    fn is_system_transaction(&self) -> bool {
        false
    }

    /// Sets whether the transaction is a system transaction
    fn set_system_transaction(&mut self, _is_system_transaction: bool) {}

    /// Returns `true` if transaction is an Optimism deposit transaction.
    fn is_deposit(&self) -> bool {
        #[cfg(feature = "optimism")]
        {
            self.tx_type() == DEPOSIT_TRANSACTION_TYPE
        }
        #[cfg(not(feature = "optimism"))]
        {
            false
        }
    }

    // Tempo methods

    /// Returns the fee token address for this transaction.
    fn fee_token(&self) -> Option<Address> {
        None
    }

    /// Sets the fee token address for this transaction.
    fn set_fee_token(&mut self, _token: Option<Address>) {}

    /// Returns the fee payer for this transaction.
    fn fee_payer(&self) -> Option<Option<Address>> {
        None
    }

    /// Sets the fee payer for this transaction.
    fn set_fee_payer(&mut self, _payer: Option<Option<Address>>) {}
}

impl FoundryTransaction for TxEnv {
    fn set_tx_type(&mut self, tx_type: u8) {
        self.tx_type = tx_type;
    }

    fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.gas_price = gas_price;
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.kind = kind;
    }

    fn set_value(&mut self, value: U256) {
        self.value = value;
    }

    fn set_data(&mut self, data: Bytes) {
        self.data = data;
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.nonce = nonce;
    }

    fn set_chain_id(&mut self, chain_id: Option<u64>) {
        self.chain_id = chain_id;
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.access_list = access_list;
    }

    fn authorization_list_mut(
        &mut self,
    ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
        &mut self.authorization_list
    }

    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
        self.gas_priority_fee = gas_priority_fee;
    }

    fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
        self.blob_hashes = blob_hashes;
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = max_fee_per_blob_gas;
    }
}

impl FoundryTransaction for TempoTxEnv {
    fn set_tx_type(&mut self, tx_type: u8) {
        self.inner.set_tx_type(tx_type);
    }

    fn set_caller(&mut self, caller: Address) {
        self.inner.set_caller(caller);
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.inner.set_gas_limit(gas_limit);
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.inner.set_gas_price(gas_price);
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.inner.set_kind(kind);
        if let Some(call) =
            self.tempo_tx_env.as_deref_mut().and_then(|env| env.aa_calls.first_mut())
        {
            call.to = kind;
        }
    }

    fn set_value(&mut self, value: U256) {
        self.inner.set_value(value);
        if let Some(call) =
            self.tempo_tx_env.as_deref_mut().and_then(|env| env.aa_calls.first_mut())
        {
            call.value = value;
        }
    }

    fn set_data(&mut self, data: Bytes) {
        self.inner.set_data(data.clone());
        if let Some(call) =
            self.tempo_tx_env.as_deref_mut().and_then(|env| env.aa_calls.first_mut())
        {
            call.input = data;
        }
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.inner.set_nonce(nonce);
    }

    fn set_chain_id(&mut self, chain_id: Option<u64>) {
        self.inner.set_chain_id(chain_id);
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.inner.set_access_list(access_list);
    }

    fn authorization_list_mut(
        &mut self,
    ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
        self.inner.authorization_list_mut()
    }

    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
        self.inner.set_gas_priority_fee(gas_priority_fee);
    }

    fn set_blob_hashes(&mut self, _blob_hashes: Vec<B256>) {}

    fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

    fn fee_token(&self) -> Option<Address> {
        self.fee_token
    }

    fn set_fee_token(&mut self, token: Option<Address>) {
        self.fee_token = token;
    }

    fn fee_payer(&self) -> Option<Option<Address>> {
        self.fee_payer
    }

    fn set_fee_payer(&mut self, payer: Option<Option<Address>>) {
        self.fee_payer = payer;
    }
}

/// Foundry extension for chain context type
///
/// Every family that doesn't need chain metadata uses `()`.
pub trait FoundryChain<Tx>: Clone + Debug + Default + Send + Sync {
    /// Builds chain context for a standalone synthetic transaction.
    fn for_transaction(_tx: &Tx) -> Self {
        Self::default()
    }

    /// Builds chain context for a transaction at an exact block position.
    fn for_block(
        _grandparent: &[Tx],
        _parent: &[Tx],
        _current: &[Tx],
        _current_tx_index: usize,
    ) -> Self {
        Self::default()
    }

    /// Refreshes journal state derived from the active chain position.
    fn refresh_journal<J: FoundryJournal>(&self, _journal: &mut J) {}
}

impl<Tx> FoundryChain<Tx> for () {}

/// Foundry extension for Journal type
pub trait FoundryJournal: JournalExt {
    /// Captures Monad's reserve-balance tracker for the active transaction.
    #[cfg(feature = "monad")]
    fn capture_reserve_balance(
        &self,
    ) -> monad_revm::reserve_balance::tracker::ReserveBalanceTracker {
        monad_revm::reserve_balance::tracker::ReserveBalanceTracker::default()
    }

    /// Restores Monad's reserve-balance tracker for the active transaction.
    #[cfg(feature = "monad")]
    fn restore_reserve_balance(
        &mut self,
        _tracker: monad_revm::reserve_balance::tracker::ReserveBalanceTracker,
    ) {
    }

    /// Whether transaction boundaries currently preserve the reserve-balance tracker, e.g. for
    /// an isolated call that models an inner call of the enclosing transaction rather than a
    /// new one.
    #[cfg(feature = "monad")]
    fn preserves_reserve_balance(&self) -> bool {
        false
    }

    /// Sets whether transaction boundaries preserve the reserve-balance tracker.
    #[cfg(feature = "monad")]
    fn set_preserve_reserve_balance(&mut self, _preserve: bool) {}
}

impl<DB: Database> FoundryJournal for Journal<DB> {}

#[cfg(feature = "monad")]
impl<DB: Database> FoundryJournal for monad_revm::MonadJournal<DB> {
    fn capture_reserve_balance(
        &self,
    ) -> monad_revm::reserve_balance::tracker::ReserveBalanceTracker {
        monad_revm::MonadJournalTr::reserve_balance(self).clone()
    }

    fn restore_reserve_balance(
        &mut self,
        tracker: monad_revm::reserve_balance::tracker::ReserveBalanceTracker,
    ) {
        *monad_revm::MonadJournalTr::reserve_balance_mut(self) = tracker;
    }

    fn preserves_reserve_balance(&self) -> bool {
        monad_revm::MonadJournalTr::preserves_reserve_balance_tracker(self)
    }

    fn set_preserve_reserve_balance(&mut self, preserve: bool) {
        monad_revm::MonadJournalTr::set_preserve_reserve_balance_tracker(self, preserve);
    }
}

/// Extension trait providing mutable field access to block, tx, and cfg environments.
///
/// [`ContextTr`] only exposes immutable references for block, tx, and cfg.
/// Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()` need to mutate these fields.
pub trait FoundryContextExt:
    ContextTr<
        Block: FoundryBlock + Clone,
        Tx: FoundryTransaction + Clone,
        Cfg: Cfg<Spec = Self::Spec> + Clone + From<CfgEnv<Self::Spec>> + Into<CfgEnv<Self::Spec>>,
        Journal: FoundryJournal,
        Chain: FoundryChain<Self::Tx>,
    >
{
    /// Specification id type
    ///
    /// Bubbled-up from `ContextTr::Cfg` for convenience and simplified bounds.
    type Spec: Into<SpecId> + Copy + Debug;

    /// Mutable reference to the block environment.
    fn block_mut(&mut self) -> &mut Self::Block;

    /// Mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut Self::Tx;

    /// Mutable reference to the configuration environment.
    fn cfg_mut(&mut self) -> &mut Self::Cfg;

    /// Reference to the underlying [`CfgEnv`].
    fn cfg_env(&self) -> &CfgEnv<Self::Spec>;

    /// Mutable reference to the underlying [`CfgEnv`].
    fn cfg_env_mut(&mut self) -> &mut CfgEnv<Self::Spec>;

    /// Mutable reference to the db and the journal inner.
    fn db_journal_inner_mut(&mut self) -> (&mut Self::Db, &mut JournaledState);

    /// Reference to the journal inner.
    fn journal_inner(&self) -> &JournaledState;

    /// Sets the spec and refreshes gas params for the concrete EVM family.
    fn set_spec_and_gas_params(&mut self, spec: Self::Spec) {
        self.cfg_env_mut().set_spec_and_mainnet_gas_params(spec);
    }

    /// Sets block environment.
    fn set_block(&mut self, block: Self::Block) {
        *self.block_mut() = block;
    }

    /// Sets transaction environment.
    fn set_tx(&mut self, tx: Self::Tx) {
        *self.tx_mut() = tx;
    }

    /// Sets configuration environment.
    fn set_cfg(&mut self, cfg: Self::Cfg) {
        *self.cfg_mut() = cfg;
    }

    /// Sets journal inner.
    fn set_journal_inner(&mut self, journal_inner: JournaledState) {
        *self.db_journal_inner_mut().1 = journal_inner;
    }

    /// Sets EVM environment.
    fn set_evm(&mut self, evm_env: EvmEnv<Self::Spec, Self::Block>) {
        *self.cfg_mut() = evm_env.cfg_env.into();
        *self.block_mut() = evm_env.block_env;
    }

    /// Cloned transaction environment.
    fn tx_clone(&self) -> Self::Tx {
        self.tx().clone()
    }

    /// Cloned EVM environment (Cfg + Block).
    fn evm_clone(&self) -> EvmEnv<Self::Spec, Self::Block> {
        EvmEnv::new(self.cfg().clone().into(), self.block().clone())
    }
}

/// Refreshes journal state derived from a context's active chain position.
pub fn refresh_chain_journal<CTX: FoundryContextExt>(context: &mut CTX) {
    let chain = context.chain().clone();
    chain.refresh_journal(context.journal_mut());
}

impl<
    BLOCK: FoundryBlock + Clone,
    TX: FoundryTransaction + Clone,
    SPEC: Into<SpecId> + Copy + Debug,
    DB: Database,
    C: FoundryChain<TX>,
> FoundryContextExt for Context<BLOCK, TX, CfgEnv<SPEC>, DB, Journal<DB>, C>
{
    type Spec = <Self::Cfg as Cfg>::Spec;
    fn block_mut(&mut self) -> &mut Self::Block {
        &mut self.block
    }

    fn tx_mut(&mut self) -> &mut Self::Tx {
        &mut self.tx
    }

    fn cfg_mut(&mut self) -> &mut Self::Cfg {
        &mut self.cfg
    }

    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        &self.cfg
    }

    fn cfg_env_mut(&mut self) -> &mut CfgEnv<Self::Spec> {
        &mut self.cfg
    }

    fn db_journal_inner_mut(&mut self) -> (&mut Self::Db, &mut JournaledState) {
        (&mut self.journaled_state.database, &mut self.journaled_state.inner)
    }

    fn journal_inner(&self) -> &JournaledState {
        &self.journaled_state.inner
    }
}

#[cfg(feature = "monad")]
impl<DB: Database> FoundryContextExt
    for Context<
        BlockEnv,
        TxEnv,
        monad_revm::MonadCfgEnv,
        DB,
        monad_revm::MonadJournal<DB>,
        monad_revm::MonadChainContext,
    >
{
    type Spec = <Self::Cfg as Cfg>::Spec;
    fn block_mut(&mut self) -> &mut Self::Block {
        &mut self.block
    }

    fn tx_mut(&mut self) -> &mut Self::Tx {
        &mut self.tx
    }

    fn cfg_mut(&mut self) -> &mut Self::Cfg {
        &mut self.cfg
    }

    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        self.cfg.inner()
    }

    fn cfg_env_mut(&mut self) -> &mut CfgEnv<Self::Spec> {
        self.cfg.inner_mut()
    }

    fn set_spec_and_gas_params(&mut self, spec: Self::Spec) {
        let mut cfg = self.cfg.clone().into_inner();
        cfg.spec = spec;
        self.cfg = monad_revm::MonadCfgEnv::from(cfg);
    }

    fn db_journal_inner_mut(&mut self) -> (&mut Self::Db, &mut JournaledState) {
        let journal: &mut Journal<DB> = std::ops::DerefMut::deref_mut(&mut self.journaled_state);
        (&mut journal.database, &mut journal.inner)
    }

    fn journal_inner(&self) -> &JournaledState {
        let journal: &Journal<DB> = std::ops::Deref::deref(&self.journaled_state);
        &journal.inner
    }
}

/// Trait for converting an [`AnyRpcTransaction`] into a specific `TxEnv`.
///
/// Ethereum envelopes delegate to [`FromRecoveredTx`]. Implementations may also explicitly
/// project compatible network-specific envelopes into their execution environment.
pub trait FromAnyRpcTransaction: Sized {
    /// Tries to convert an [`AnyRpcTransaction`] into `Self`.
    fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self>;
}

impl FromAnyRpcTransaction for TxEnv {
    fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
        if let Some(envelope) = tx.as_envelope() {
            return Ok(Self::from_recovered_tx(envelope, tx.from()));
        }

        // CIP-64 transactions have EIP-1559 execution fields plus a Celo-specific fee currency.
        // Foundry does not model fee payment in TxEnv, but can replay their EVM payload. Preserve
        // the custom type so revm does not compare the fee-currency price with the native-CELO
        // base fee. Keep this projection restricted to active Celo chains so an unrelated network
        // cannot silently acquire semantics for its own type 0x7b envelope.
        if let AnyTxEnvelope::Unknown(unknown) = &*tx.inner.inner
            && unknown.ty() == CELO_DYNAMIC_FEE_TX_TYPE
            && matches!(
                unknown.chain_id().and_then(NamedChain::from_chain_id),
                Some(NamedChain::Celo | NamedChain::CeloSepolia)
            )
        {
            return Ok(Self {
                tx_type: CELO_DYNAMIC_FEE_TX_TYPE,
                caller: tx.from(),
                gas_limit: unknown.gas_limit(),
                gas_price: unknown.max_fee_per_gas(),
                gas_priority_fee: unknown.max_priority_fee_per_gas(),
                kind: unknown.kind(),
                value: unknown.value(),
                data: unknown.input().clone(),
                nonce: unknown.nonce(),
                chain_id: unknown.chain_id(),
                access_list: unknown.access_list().cloned().unwrap_or_default(),
                ..Default::default()
            });
        }

        eyre::bail!("cannot convert unknown transaction type to TxEnv");
    }
}

impl FromAnyRpcTransaction for TempoTxEnv {
    fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
        if let Some(envelope) = tx.as_envelope() {
            return Ok(TxEnv::from_recovered_tx(envelope, tx.from()).into());
        }

        // Handle Tempo transactions from `Unknown` envelope variant.
        if let AnyTxEnvelope::Unknown(unknown) = &*tx.inner.inner
            && unknown.ty() == tempo_alloy::primitives::TEMPO_TX_TYPE_ID
        {
            let base = TxEnv {
                tx_type: unknown.ty(),
                caller: tx.from(),
                gas_limit: unknown.gas_limit(),
                gas_price: unknown.max_fee_per_gas(),
                gas_priority_fee: unknown.max_priority_fee_per_gas(),
                kind: unknown.kind(),
                value: unknown.value(),
                data: unknown.input().clone(),
                nonce: unknown.nonce(),
                chain_id: unknown.chain_id(),
                access_list: unknown.access_list().cloned().unwrap_or_default(),
                ..Default::default()
            };
            let fee_token =
                unknown.inner.fields.get_deserialized::<Address>("feeToken").and_then(Result::ok);
            return Ok(Self { inner: base, fee_token, ..Default::default() });
        }

        eyre::bail!("cannot convert unknown transaction type to TempoTxEnv");
    }
}

#[cfg(feature = "base")]
mod base {
    use base_common_consensus::BaseTxEnvelope;
    use base_common_evm::{
        BaseTransaction, BaseTxTr, DEPOSIT_TRANSACTION_TYPE, EIP8130_TRANSACTION_TYPE,
    };
    use base_common_rpc_types::Transaction as BaseRpcTransaction;

    use super::*;

    impl<TX: FoundryTransaction> FoundryTransaction for BaseTransaction<TX> {
        fn set_tx_type(&mut self, tx_type: u8) {
            self.base.set_tx_type(tx_type);
        }

        fn set_caller(&mut self, caller: Address) {
            self.base.set_caller(caller);
        }

        fn set_gas_limit(&mut self, gas_limit: u64) {
            self.base.set_gas_limit(gas_limit);
        }

        fn set_gas_price(&mut self, gas_price: u128) {
            self.base.set_gas_price(gas_price);
        }

        fn set_kind(&mut self, kind: TxKind) {
            self.base.set_kind(kind);
        }

        fn set_value(&mut self, value: U256) {
            self.base.set_value(value);
        }

        fn set_data(&mut self, data: Bytes) {
            self.base.set_data(data);
        }

        fn set_nonce(&mut self, nonce: u64) {
            self.base.set_nonce(nonce);
        }

        fn set_chain_id(&mut self, chain_id: Option<u64>) {
            self.base.set_chain_id(chain_id);
        }

        fn set_access_list(&mut self, access_list: AccessList) {
            self.base.set_access_list(access_list);
        }

        fn authorization_list_mut(
            &mut self,
        ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
            self.base.authorization_list_mut()
        }

        fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
            self.base.set_gas_priority_fee(gas_priority_fee);
        }

        fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
            self.base.set_blob_hashes(blob_hashes);
        }

        fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
            self.base.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
        }

        fn enveloped_tx(&self) -> Option<&Bytes> {
            BaseTxTr::enveloped_tx(self)
        }

        fn set_enveloped_tx(&mut self, bytes: Bytes) {
            self.enveloped_tx = Some(bytes);
        }

        fn source_hash(&self) -> Option<B256> {
            BaseTxTr::source_hash(self)
        }

        fn set_source_hash(&mut self, source_hash: B256) {
            self.deposit.source_hash = source_hash;
        }

        fn mint(&self) -> Option<u128> {
            BaseTxTr::mint(self)
        }

        fn set_mint(&mut self, mint: u128) {
            self.deposit.mint = Some(mint);
        }

        fn is_system_transaction(&self) -> bool {
            BaseTxTr::is_system_transaction(self)
        }

        fn set_system_transaction(&mut self, is_system_transaction: bool) {
            self.deposit.is_system_transaction = is_system_transaction;
        }

        fn is_deposit(&self) -> bool {
            self.tx_type() == DEPOSIT_TRANSACTION_TYPE
        }
    }

    impl FromAnyRpcTransaction for BaseTransaction<TxEnv> {
        fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
            let envelope = match BaseTxEnvelope::try_from(tx.clone()) {
                Ok(envelope) => envelope,
                Err(_) if tx.ty() == EIP8130_TRANSACTION_TYPE => {
                    let rpc_tx =
                        serde_json::from_value::<BaseRpcTransaction>(serde_json::to_value(tx)?)
                            .map_err(|err| {
                                eyre::eyre!(
                                    "cannot convert RPC transaction to Base envelope: {err}"
                                )
                            })?;
                    rpc_tx.inner.into_inner()
                }
                Err(_) => eyre::bail!("cannot convert transaction to BaseTxEnvelope"),
            };
            Ok(Self::from_recovered_tx(&envelope, tx.from()))
        }
    }
}

#[cfg(feature = "optimism")]
mod optimism {
    use super::*;
    use alloy_eips::eip2718::Encodable2718;
    use alloy_op_evm::OpTx;
    use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, TxDeposit};
    use op_revm::{OpTransaction, transaction::OpTxTr};

    impl<TX: FoundryTransaction> FoundryTransaction for OpTransaction<TX> {
        fn set_tx_type(&mut self, tx_type: u8) {
            self.base.set_tx_type(tx_type);
        }

        fn set_caller(&mut self, caller: Address) {
            self.base.set_caller(caller);
        }

        fn set_gas_limit(&mut self, gas_limit: u64) {
            self.base.set_gas_limit(gas_limit);
        }

        fn set_gas_price(&mut self, gas_price: u128) {
            self.base.set_gas_price(gas_price);
        }

        fn set_kind(&mut self, kind: TxKind) {
            self.base.set_kind(kind);
        }

        fn set_value(&mut self, value: U256) {
            self.base.set_value(value);
        }

        fn set_data(&mut self, data: Bytes) {
            self.base.set_data(data);
        }

        fn set_nonce(&mut self, nonce: u64) {
            self.base.set_nonce(nonce);
        }

        fn set_chain_id(&mut self, chain_id: Option<u64>) {
            self.base.set_chain_id(chain_id);
        }

        fn set_access_list(&mut self, access_list: AccessList) {
            self.base.set_access_list(access_list);
        }

        fn authorization_list_mut(
            &mut self,
        ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
            self.base.authorization_list_mut()
        }

        fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
            self.base.set_gas_priority_fee(gas_priority_fee);
        }

        fn set_blob_hashes(&mut self, _blob_hashes: Vec<B256>) {}

        fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

        fn enveloped_tx(&self) -> Option<&Bytes> {
            OpTxTr::enveloped_tx(self)
        }

        fn set_enveloped_tx(&mut self, bytes: Bytes) {
            self.enveloped_tx = Some(bytes);
        }

        fn source_hash(&self) -> Option<B256> {
            OpTxTr::source_hash(self)
        }

        fn set_source_hash(&mut self, source_hash: B256) {
            if self.tx_type() == DEPOSIT_TRANSACTION_TYPE {
                self.deposit.source_hash = source_hash;
            }
        }

        fn mint(&self) -> Option<u128> {
            OpTxTr::mint(self)
        }

        fn set_mint(&mut self, mint: u128) {
            if self.tx_type() == DEPOSIT_TRANSACTION_TYPE {
                self.deposit.mint = Some(mint);
            }
        }

        fn is_system_transaction(&self) -> bool {
            OpTxTr::is_system_transaction(self)
        }

        fn set_system_transaction(&mut self, is_system_transaction: bool) {
            if self.tx_type() == DEPOSIT_TRANSACTION_TYPE {
                self.deposit.is_system_transaction = is_system_transaction;
            }
        }
    }

    impl FoundryTransaction for OpTx {
        fn set_tx_type(&mut self, tx_type: u8) {
            self.0.set_tx_type(tx_type);
        }

        fn set_caller(&mut self, caller: Address) {
            self.0.set_caller(caller);
        }

        fn set_gas_limit(&mut self, gas_limit: u64) {
            self.0.set_gas_limit(gas_limit);
        }

        fn set_gas_price(&mut self, gas_price: u128) {
            self.0.set_gas_price(gas_price);
        }

        fn set_kind(&mut self, kind: TxKind) {
            self.0.set_kind(kind);
        }

        fn set_value(&mut self, value: U256) {
            self.0.set_value(value);
        }

        fn set_data(&mut self, data: Bytes) {
            self.0.set_data(data);
        }

        fn set_nonce(&mut self, nonce: u64) {
            self.0.set_nonce(nonce);
        }

        fn set_chain_id(&mut self, chain_id: Option<u64>) {
            self.0.set_chain_id(chain_id);
        }

        fn set_access_list(&mut self, access_list: AccessList) {
            self.0.set_access_list(access_list);
        }

        fn authorization_list_mut(
            &mut self,
        ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
            self.0.authorization_list_mut()
        }

        fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
            self.0.set_gas_priority_fee(gas_priority_fee);
        }

        fn set_blob_hashes(&mut self, _blob_hashes: Vec<B256>) {}

        fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

        fn enveloped_tx(&self) -> Option<&Bytes> {
            FoundryTransaction::enveloped_tx(&self.0)
        }

        fn set_enveloped_tx(&mut self, bytes: Bytes) {
            self.0.set_enveloped_tx(bytes);
        }

        fn source_hash(&self) -> Option<B256> {
            FoundryTransaction::source_hash(&self.0)
        }

        fn set_source_hash(&mut self, source_hash: B256) {
            self.0.set_source_hash(source_hash);
        }

        fn mint(&self) -> Option<u128> {
            FoundryTransaction::mint(&self.0)
        }

        fn set_mint(&mut self, mint: u128) {
            self.0.set_mint(mint);
        }

        fn is_system_transaction(&self) -> bool {
            FoundryTransaction::is_system_transaction(&self.0)
        }

        fn set_system_transaction(&mut self, is_system_transaction: bool) {
            self.0.set_system_transaction(is_system_transaction);
        }
    }

    impl FromAnyRpcTransaction for OpTx {
        fn from_any_rpc_transaction(tx: &AnyRpcTransaction) -> eyre::Result<Self> {
            if let Some(envelope) = tx.as_envelope() {
                return Ok(Self(OpTransaction::<TxEnv> {
                    base: TxEnv::from_recovered_tx(envelope, tx.from()),
                    // The L1 data fee is charged off these bytes, and op-revm rejects a
                    // non-deposit transaction that arrives without them.
                    enveloped_tx: Some(envelope.encoded_2718().into()),
                    deposit: Default::default(),
                }));
            }

            // Handle OP deposit transactions from `Unknown` envelope variant.
            if let AnyTxEnvelope::Unknown(unknown) = &*tx.inner.inner
                && unknown.ty() == DEPOSIT_TX_TYPE_ID
            {
                let mut fields = unknown.inner.fields.clone();
                fields.insert("from".to_string(), serde_json::to_value(tx.from())?);
                let deposit_tx: TxDeposit = fields
                    .deserialize_into()
                    .map_err(|e| eyre::eyre!("failed to deserialize deposit tx: {e}"))?;
                return Ok(Self::from_recovered_tx(&deposit_tx, deposit_tx.from));
            }

            eyre::bail!("cannot convert unknown transaction type to OpTransaction");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;
    use alloy_consensus::{Signed, TxEip1559, transaction::Recovered};
    use alloy_evm::{EthEvmFactory, EvmFactory};
    use alloy_network::{AnyTxType, UnknownTxEnvelope, UnknownTypedTransaction};
    use alloy_primitives::Signature;
    use alloy_rpc_types::{Transaction as RpcTransaction, TransactionInfo};
    use alloy_serde::WithOtherFields;
    #[cfg(feature = "base")]
    use base_common_evm::{BaseEvmFactory, BaseSpecId, BaseTransaction, BaseUpgrade};
    use foundry_evm_hardforks::TempoHardfork;
    use revm::database::EmptyDB;
    use tempo_alloy::primitives::{
        AASigned, TempoSignature, TempoTransaction, TempoTxEnvelope,
        transaction::{Call, PrimitiveSignature},
    };
    use tempo_evm::TempoEvmFactory;

    #[test]
    fn eth_evm_foundry_context_ext_implementation() {
        let mut evm = EthEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());

        // Test EVM Context Block mutation
        evm.ctx_mut().block_mut().set_number(U256::from(123));
        assert_eq!(evm.ctx().block().number(), U256::from(123));

        // Test EVM Context Tx mutation
        evm.ctx_mut().tx_mut().set_nonce(99);
        assert_eq!(evm.ctx().tx().nonce(), 99);

        // Test EVM Context Cfg mutation
        evm.ctx_mut().cfg_mut().spec = SpecId::AMSTERDAM;
        assert_eq!(evm.ctx().cfg().spec, SpecId::AMSTERDAM);

        // Round-trip test to ensure no issues with cloning and setting tx_env and evm_env
        let tx_env = evm.ctx().tx_clone();
        evm.ctx_mut().set_tx(tx_env);
        let evm_env = evm.ctx().evm_clone();
        evm.ctx_mut().set_evm(evm_env);
    }

    #[cfg(feature = "base")]
    #[test]
    fn base_evm_foundry_context_ext_implementation() {
        let mut evm = BaseEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());

        evm.ctx_mut().block_mut().set_number(U256::from(123));
        assert_eq!(evm.ctx().block().number(), U256::from(123));

        evm.ctx_mut().tx_mut().set_nonce(99);
        assert_eq!(evm.ctx().tx().nonce(), 99);

        evm.ctx_mut().cfg_mut().spec = BaseSpecId::new(BaseUpgrade::Beryl);
        assert_eq!(evm.ctx().cfg().spec, BaseSpecId::new(BaseUpgrade::Beryl));

        let tx_env = evm.ctx().tx_clone();
        evm.ctx_mut().set_tx(tx_env);
        let evm_env = evm.ctx().evm_clone();
        evm.ctx_mut().set_evm(evm_env);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn monad_evm_foundry_context_ext_implementation() {
        let mut evm = alloy_monad_evm::MonadEvmFactory::default().create_evm(
            EmptyDB::default(),
            EvmEnv::new(
                CfgEnv::new_with_spec(monad_revm::MonadHardfork::MonadNine),
                BlockEnv::default(),
            ),
        );

        // Test EVM Context Block mutation
        evm.ctx_mut().block_mut().set_number(U256::from(123));
        assert_eq!(evm.ctx().block().number(), U256::from(123));

        // Test EVM Context Tx mutation
        evm.ctx_mut().tx_mut().set_nonce(99);
        assert_eq!(evm.ctx().tx().nonce(), 99);

        // Test EVM Context Cfg mutation
        evm.ctx_mut().cfg_mut().spec = monad_revm::MonadHardfork::MonadEight;
        assert_eq!(evm.ctx().cfg().spec, monad_revm::MonadHardfork::MonadEight);

        // Round-trip test to ensure no issues with cloning and setting tx_env and evm_env
        let tx_env = evm.ctx().tx_clone();
        evm.ctx_mut().set_tx(tx_env);
        let evm_env = evm.ctx().evm_clone();
        evm.ctx_mut().set_evm(evm_env);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn monad_memory_limit_follows_hardfork_transitions() {
        const FOUNDRY_MEMORY_LIMIT: u64 = 128 * 1024 * 1024;

        let mut cfg = CfgEnv::new_with_spec(monad_revm::MonadHardfork::MonadEight);
        cfg.memory_limit = FOUNDRY_MEMORY_LIMIT;
        let mut evm = alloy_monad_evm::MonadEvmFactory::default()
            .create_evm(EmptyDB::default(), EvmEnv::new(cfg, BlockEnv::default()));

        assert_eq!(evm.ctx().cfg().memory_limit(), FOUNDRY_MEMORY_LIMIT);

        evm.ctx_mut().set_spec_and_gas_params(monad_revm::MonadHardfork::MonadNine);
        assert_eq!(evm.ctx().cfg().inner().memory_limit, FOUNDRY_MEMORY_LIMIT);
        assert_eq!(evm.ctx().cfg().memory_limit(), monad_revm::cfg::MONAD_MEMORY_LIMIT);

        evm.ctx_mut().set_spec_and_gas_params(monad_revm::MonadHardfork::MonadEight);
        assert_eq!(evm.ctx().cfg().memory_limit(), FOUNDRY_MEMORY_LIMIT);
    }

    #[test]
    fn tempo_evm_foundry_context_ext_implementation() {
        let mut evm = TempoEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());

        // Test EVM Context Block mutation
        evm.ctx_mut().block_mut().set_number(U256::from(123));
        assert_eq!(evm.ctx().block().number(), U256::from(123));

        // Test EVM Context Tx mutation
        evm.ctx_mut().tx_mut().set_nonce(99);
        assert_eq!(evm.ctx().tx().nonce(), 99);

        // Test EVM Context Cfg mutation
        evm.ctx_mut().cfg_mut().spec = TempoHardfork::Genesis;
        assert_eq!(evm.ctx().cfg().spec, TempoHardfork::Genesis);

        // Round-trip test to ensure no issues with cloning and setting tx_env and evm_env
        let tx_env = evm.ctx().tx_clone();
        evm.ctx_mut().set_tx(tx_env);
        let evm_env = evm.ctx().evm_clone();
        evm.ctx_mut().set_evm(evm_env);
    }

    #[test]
    fn tempo_tx_env_setters_update_aa_call_payload() {
        let old_to = TxKind::Call(Address::with_last_byte(0xAA));
        let new_to = TxKind::Create;
        let new_value = U256::from(123);
        let new_input = Bytes::from_static(b"local bytecode");

        let mut tx_env = TempoTxEnv {
            inner: TxEnv {
                kind: old_to,
                value: U256::from(1),
                data: Bytes::from_static(b"original bytecode"),
                ..Default::default()
            },
            tempo_tx_env: Some(Box::new(tempo_revm::TempoBatchCallEnv {
                aa_calls: vec![Call {
                    to: old_to,
                    value: U256::from(1),
                    input: Bytes::from_static(b"original bytecode"),
                }],
                ..Default::default()
            })),
            ..Default::default()
        };

        tx_env.set_kind(new_to);
        tx_env.set_value(new_value);
        tx_env.set_data(new_input.clone());

        assert_eq!(tx_env.inner.kind, new_to);
        assert_eq!(tx_env.inner.value, new_value);
        assert_eq!(tx_env.inner.data, new_input);

        let call = &tx_env.tempo_tx_env.as_ref().unwrap().aa_calls[0];
        assert_eq!(call.to, new_to);
        assert_eq!(call.value, new_value);
        assert_eq!(call.input, new_input);
    }

    fn make_signed_eip1559() -> Signed<TxEip1559> {
        Signed::new_unchecked(
            TxEip1559 {
                chain_id: 1,
                nonce: 42,
                gas_limit: 21001,
                to: TxKind::Call(Address::with_last_byte(0xBB)),
                value: U256::from(101),
                ..Default::default()
            },
            Signature::new(U256::ZERO, U256::ZERO, false),
            B256::ZERO,
        )
    }

    #[test]
    fn from_any_rpc_transaction_for_eth() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();
        let rpc_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(signed_tx.into(), from),
            TransactionInfo::default(),
        );

        let any_tx = <AnyRpcTransaction as From<RpcTransaction>>::from(rpc_tx);
        let tx_env = TxEnv::from_any_rpc_transaction(&any_tx).unwrap();

        assert_eq!(tx_env.caller, from);
        assert_eq!(tx_env.nonce, 42);
        assert_eq!(tx_env.gas_limit, 21001);
        assert_eq!(tx_env.value, U256::from(101));
        assert_eq!(tx_env.kind, TxKind::Call(Address::with_last_byte(0xBB)));
    }

    #[cfg(feature = "base")]
    #[test]
    fn from_any_rpc_transaction_for_base_eth_envelope() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();
        let rpc_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(signed_tx.into(), from),
            TransactionInfo::default(),
        );
        let any_tx = <AnyRpcTransaction as From<RpcTransaction>>::from(rpc_tx);

        let tx_env = BaseTransaction::<TxEnv>::from_any_rpc_transaction(&any_tx).unwrap();
        assert_eq!(tx_env.base.caller, from);
        assert_eq!(tx_env.base.nonce, 42);
        assert_eq!(tx_env.base.gas_limit, 21001);
        assert_eq!(tx_env.base.value, U256::from(101));
        assert!(tx_env.enveloped_tx.is_some());
    }

    #[test]
    fn from_any_rpc_transaction_unknown_envelope_errors() {
        let unknown = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
            hash: B256::ZERO,
            inner: UnknownTypedTransaction {
                ty: AnyTxType(0xFF),
                fields: Default::default(),
                memo: Default::default(),
            },
        });
        let from = Address::random();
        let any_tx = AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
            inner: Recovered::new_unchecked(unknown, from),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
            block_timestamp: None,
        }));

        let result = TxEnv::from_any_rpc_transaction(&any_tx).unwrap_err();
        assert!(result.to_string().contains("unknown transaction type"));
    }

    #[test]
    fn from_any_rpc_transaction_for_celo_dynamic_fee() {
        let from = Address::with_last_byte(0xAA);
        let to = Address::with_last_byte(0xBB);
        let fee_currency = Address::with_last_byte(0xCC);
        let json = serde_json::json!({
            "accessList": [],
            "blockHash": B256::ZERO,
            "blockNumber": "0x1",
            "chainId": "0xa4ec",
            "feeCurrency": fee_currency,
            "from": from,
            "gas": "0x5208",
            "gasPrice": "0x3",
            "hash": B256::ZERO,
            "input": "0x1234",
            "maxFeePerGas": "0x3",
            "maxPriorityFeePerGas": "0x1",
            "nonce": "0x2a",
            "r": B256::ZERO,
            "s": B256::ZERO,
            "to": to,
            "transactionIndex": "0x0",
            "type": "0x7b",
            "v": "0x0",
            "value": "0x65",
            "yParity": "0x0"
        });
        let mut non_celo_json = json.clone();
        non_celo_json["chainId"] = serde_json::json!("0x1");
        let non_celo_tx: AnyRpcTransaction = serde_json::from_value(non_celo_json).unwrap();
        assert!(TxEnv::from_any_rpc_transaction(&non_celo_tx).is_err());

        let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

        let tx_env = TxEnv::from_any_rpc_transaction(&any_tx).unwrap();

        assert_eq!(tx_env.tx_type, CELO_DYNAMIC_FEE_TX_TYPE);
        assert_eq!(tx_env.caller, from);
        assert_eq!(tx_env.nonce, 42);
        assert_eq!(tx_env.gas_limit, 21000);
        assert_eq!(tx_env.gas_price, 3);
        assert_eq!(tx_env.gas_priority_fee, Some(1));
        assert_eq!(tx_env.kind, TxKind::Call(to));
        assert_eq!(tx_env.value, U256::from(101));
        assert_eq!(tx_env.data, Bytes::from_static(&[0x12, 0x34]));
        assert_eq!(tx_env.chain_id, Some(42_220));
    }

    #[test]
    fn from_any_rpc_transaction_for_tempo_eth_envelope() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();
        let rpc_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(signed_tx.into(), from),
            TransactionInfo::default(),
        );
        let any_tx = <AnyRpcTransaction as From<RpcTransaction>>::from(rpc_tx);

        let tx_env = TempoTxEnv::from_any_rpc_transaction(&any_tx).unwrap();
        assert_eq!(tx_env.inner.caller, from);
        assert_eq!(tx_env.inner.nonce, 42);
        assert_eq!(tx_env.inner.gas_limit, 21001);
        assert_eq!(tx_env.inner.value, U256::from(101));
        assert_eq!(tx_env.fee_token, None);
    }

    #[test]
    fn from_any_rpc_transaction_for_tempo_aa() {
        let from = Address::random();
        let fee_token = Some(Address::random());
        let tempo_tx = TempoTransaction {
            chain_id: 42431,
            nonce: 42,
            gas_limit: 424242,
            fee_token,
            nonce_key: U256::from(4242),
            valid_after: NonZeroU64::new(1800000000),
            ..Default::default()
        };
        let aa_signed = AASigned::new_unhashed(
            tempo_tx,
            TempoSignature::Primitive(PrimitiveSignature::Secp256k1(Signature::new(
                U256::ZERO,
                U256::ZERO,
                false,
            ))),
        );

        // Build a concrete Tempo RPC transaction, serialize to JSON, deserialize as
        // AnyRpcTransaction.
        let rpc_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(TempoTxEnvelope::AA(aa_signed), from),
            TransactionInfo::default(),
        );
        let json = serde_json::to_value(&rpc_tx).unwrap();
        let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

        let tx_env = TempoTxEnv::from_any_rpc_transaction(&any_tx).unwrap();
        assert_eq!(tx_env.inner.caller, from);
        assert_eq!(tx_env.inner.nonce, 42);
        assert_eq!(tx_env.inner.gas_limit, 424242);
        assert_eq!(tx_env.inner.chain_id, Some(42431));
        assert_eq!(tx_env.fee_token, fee_token);
    }

    #[cfg(feature = "optimism")]
    mod optimism {
        use super::*;
        use alloy_consensus::Sealed;
        use alloy_eips::eip2718::Encodable2718;
        use alloy_op_evm::{OpEvmFactory, OpTx};
        use op_alloy_consensus::{OpTxEnvelope, TxDeposit, transaction::OpTransactionInfo};
        use op_alloy_rpc_types::Transaction as OpRpcTransaction;
        use op_revm::OpSpecId;

        #[test]
        fn op_evm_foundry_context_ext_implementation() {
            let mut evm =
                OpEvmFactory::<OpTx>::default().create_evm(EmptyDB::default(), EvmEnv::default());

            // Test EVM Context Block mutation
            evm.ctx_mut().block_mut().set_number(U256::from(123));
            assert_eq!(evm.ctx().block().number(), U256::from(123));

            // Test EVM Context Tx mutation
            evm.ctx_mut().tx_mut().set_nonce(99);
            assert_eq!(evm.ctx().tx().nonce(), 99);

            // Test EVM Context Cfg mutation
            evm.ctx_mut().cfg_mut().spec = OpSpecId::JOVIAN;
            assert_eq!(evm.ctx().cfg().spec, OpSpecId::JOVIAN);

            // Round-trip test to ensure no issues with cloning and setting tx_env and evm_env
            let tx_env = evm.ctx().tx_clone();
            evm.ctx_mut().set_tx(tx_env);
            let evm_env = evm.ctx().evm_clone();
            evm.ctx_mut().set_evm(evm_env);
        }

        #[test]
        fn from_any_rpc_transaction_for_op() {
            let from = Address::random();
            let signed_tx = make_signed_eip1559();

            // Build the eth TxEnv to compare against op base
            let rpc_tx = RpcTransaction::from_transaction(
                Recovered::new_unchecked(signed_tx.into(), from),
                TransactionInfo::default(),
            );
            let any_tx = <AnyRpcTransaction as From<RpcTransaction>>::from(rpc_tx);
            let expected_base = TxEnv::from_any_rpc_transaction(&any_tx).unwrap();

            let op_tx_env = OpTx::from_any_rpc_transaction(&any_tx).unwrap();
            assert_eq!(op_tx_env.base, expected_base);
            // op-revm charges the L1 data fee off these bytes and rejects a non-deposit
            // transaction that arrives without them.
            assert_eq!(
                op_tx_env.enveloped_tx,
                Some(any_tx.as_envelope().unwrap().encoded_2718().into())
            );
        }

        #[test]
        fn from_any_rpc_transaction_for_op_deposit() {
            let from = Address::random();
            let source_hash = B256::random();
            let deposit = TxDeposit {
                source_hash,
                from,
                to: TxKind::Call(Address::with_last_byte(0xCC)),
                mint: 1111,
                value: U256::from(200),
                gas_limit: 21000,
                is_system_transaction: true,
                input: Default::default(),
            };

            // Build a concrete OpRpcTransaction, serialize to JSON, deserialize as
            // AnyRpcTransaction.
            let op_rpc_tx = OpRpcTransaction::from_transaction(
                Recovered::new_unchecked(OpTxEnvelope::Deposit(Sealed::new(deposit)), from),
                OpTransactionInfo::default(),
            );
            let json = serde_json::to_value(&op_rpc_tx).unwrap();
            let any_tx: AnyRpcTransaction = serde_json::from_value(json).unwrap();

            let op_tx_env = OpTx::from_any_rpc_transaction(&any_tx).unwrap();
            assert_eq!(op_tx_env.base.caller, from);
            assert_eq!(op_tx_env.base.kind, TxKind::Call(Address::with_last_byte(0xCC)));
            assert_eq!(op_tx_env.base.value, U256::from(200));
            assert_eq!(op_tx_env.base.gas_limit, 21000);
            assert_eq!(op_tx_env.deposit.source_hash, source_hash);
            assert_eq!(op_tx_env.deposit.mint, Some(1111));
            assert!(op_tx_env.deposit.is_system_transaction);
        }
    }
}
