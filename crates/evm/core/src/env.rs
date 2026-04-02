use std::fmt::Debug;

pub use alloy_evm::EvmEnv;
use alloy_evm::{FromRecoveredTx, ToTxEnv};
use alloy_network::{AnyRpcTransaction, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::Transaction as RpcTransaction;
use op_alloy_rpc_types::Transaction as OpRpcTransaction;
use op_revm::{
    OpTransaction,
    transaction::{OpTxTr, deposit::DEPOSIT_TRANSACTION_TYPE},
};
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
use tempo_primitives::TempoTxEnvelope;
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

    /// Returns `true` if transaction is of type [`DEPOSIT_TRANSACTION_TYPE`].
    fn is_deposit(&self) -> bool {
        self.tx_type() == DEPOSIT_TRANSACTION_TYPE
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
    }

    fn set_value(&mut self, value: U256) {
        self.inner.set_value(value);
    }

    fn set_data(&mut self, data: Bytes) {
        self.inner.set_data(data);
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

/// Extension trait providing mutable field access to block, tx, and cfg environments.
///
/// [`ContextTr`] only exposes immutable references for block, tx, and cfg.
/// Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()` need to mutate these fields.
pub trait FoundryContextExt:
    ContextTr<
        Block: FoundryBlock + Clone,
        Tx: FoundryTransaction + Clone,
        Cfg = CfgEnv<Self::Spec>,
        Journal: JournalExt,
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

    /// Mutable reference to the db and the journal inner.
    fn db_journal_inner_mut(&mut self) -> (&mut Self::Db, &mut JournaledState);

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
        *self.cfg_mut() = evm_env.cfg_env;
        *self.block_mut() = evm_env.block_env;
    }

    /// Cloned transaction environment.
    fn tx_clone(&self) -> Self::Tx {
        self.tx().clone()
    }

    /// Cloned EVM environment (Cfg + Block).
    fn evm_clone(&self) -> EvmEnv<Self::Spec, Self::Block> {
        EvmEnv::new(self.cfg().clone(), self.block().clone())
    }
}

impl<
    BLOCK: FoundryBlock + Clone,
    TX: FoundryTransaction + Clone,
    SPEC: Into<SpecId> + Copy + Debug,
    DB: Database,
    C,
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

    fn db_journal_inner_mut(&mut self) -> (&mut Self::Db, &mut JournaledState) {
        (&mut self.journaled_state.database, &mut self.journaled_state.inner)
    }
}

/// Abstraction trait for converting any RPC transaction into corresponding `TxEnv`.
///
/// This trait bridges the gap between different network RPC transaction types and the EVM's
/// `TxEnv`:
/// - For [`RpcTransaction`] (Ethereum): delegates to [`ToTxEnv`].
/// - For [`AnyRpcTransaction`] (AnyNetwork): extracts the inner [`alloy_consensus::TxEnvelope`] via
///   [`as_envelope()`](alloy_network::AnyTxEnvelope::as_envelope) then delegates to
///   [`FromRecoveredTx`].
/// - For [`OpRpcTransaction`] (Optimism): delegates to [`ToTxEnv`].
pub trait TryAnyToTxEnv<TxEnv> {
    /// Tries to convert this RPC transaction into a [`TxEnv`].
    fn try_any_to_tx_env(&self) -> eyre::Result<TxEnv>;
}

impl TryAnyToTxEnv<TxEnv> for RpcTransaction {
    fn try_any_to_tx_env(&self) -> eyre::Result<TxEnv> {
        Ok(self.as_recovered().to_tx_env())
    }
}

impl TryAnyToTxEnv<TxEnv> for AnyRpcTransaction {
    fn try_any_to_tx_env(&self) -> eyre::Result<TxEnv> {
        if let Some(envelope) = self.as_envelope() {
            Ok(TxEnv::from_recovered_tx(envelope, self.from()))
        } else {
            eyre::bail!("cannot convert unknown transaction type to TxEnv")
        }
    }
}

impl TryAnyToTxEnv<OpTransaction<TxEnv>> for OpRpcTransaction {
    fn try_any_to_tx_env(&self) -> eyre::Result<OpTransaction<TxEnv>> {
        Ok(self.as_recovered().to_tx_env())
    }
}

impl TryAnyToTxEnv<TempoTxEnv> for RpcTransaction<TempoTxEnvelope> {
    fn try_any_to_tx_env(&self) -> eyre::Result<TempoTxEnv> {
        Ok(self.as_recovered().to_tx_env())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Sealed, Signed, TxEip1559, transaction::Recovered};
    use alloy_evm::{EthEvmFactory, EvmFactory};
    use alloy_network::{AnyTxEnvelope, AnyTxType, UnknownTxEnvelope, UnknownTypedTransaction};
    use alloy_op_evm::OpEvmFactory;
    use alloy_primitives::Signature;
    use alloy_rpc_types::TransactionInfo;
    use alloy_serde::WithOtherFields;
    use foundry_evm_hardforks::TempoHardfork;
    use op_alloy_consensus::{OpTxEnvelope, TxDeposit, transaction::OpTransactionInfo};
    use op_revm::OpSpecId;
    use revm::database::EmptyDB;
    use tempo_evm::TempoEvmFactory;
    use tempo_primitives::{TempoSignature, TempoTransaction, transaction::PrimitiveSignature};

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
    fn try_any_to_tx_env_for_eth_and_any_transactions() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();
        let tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(signed_tx.into(), from),
            TransactionInfo::default(),
        );
        let tx_env: TxEnv = tx.try_any_to_tx_env().unwrap();

        assert_eq!(tx_env.caller, from);
        assert_eq!(tx_env.nonce, 42);
        assert_eq!(tx_env.gas_limit, 21001);
        assert_eq!(tx_env.value, U256::from(101));
        assert_eq!(tx_env.kind, TxKind::Call(Address::with_last_byte(0xBB)));

        // Wrap as AnyRpcTransaction (Ethereum variant) via From<Transaction<TxEnvelope>>.
        let any_tx = <AnyRpcTransaction as From<RpcTransaction>>::from(tx);
        let any_tx_env: TxEnv = any_tx.try_any_to_tx_env().unwrap();

        // TxEnv from AnyRpcTransaction must be the same
        assert_eq!(tx_env, any_tx_env);
    }

    #[test]
    fn try_any_to_tx_env_for_op_transactions() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();

        // Build the eth TxEnv to compare against op base
        let eth_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(signed_tx.clone().into(), from),
            TransactionInfo::default(),
        );
        let expected_base: TxEnv = eth_tx.try_any_to_tx_env().unwrap();

        let op_tx = OpRpcTransaction::from_transaction(
            Recovered::new_unchecked(signed_tx.into(), from),
            OpTransactionInfo::default(),
        );
        let op_tx_env: OpTransaction<TxEnv> = op_tx.try_any_to_tx_env().unwrap();

        assert_eq!(op_tx_env.base, expected_base);

        // Test with Deposit tx
        let op_deposit_tx = OpRpcTransaction::from_transaction(
            Recovered::new_unchecked(
                OpTxEnvelope::Deposit(Sealed::new(TxDeposit {
                    from,
                    mint: 1111,
                    ..Default::default()
                })),
                from,
            ),
            OpTransactionInfo::default(),
        );
        let op_deposit_tx_env: OpTransaction<TxEnv> = op_deposit_tx.try_any_to_tx_env().unwrap();

        assert_eq!(op_deposit_tx_env.deposit.mint, Some(1111));
        assert_eq!(op_deposit_tx_env.base.caller, from);
    }

    #[test]
    fn try_any_to_tx_env_unknown_envelope_errors() {
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

        let result = any_tx.try_any_to_tx_env().unwrap_err();
        assert!(result.to_string().contains("unknown transaction type"));
    }

    #[test]
    fn try_any_to_tx_env_for_tempo_transactions() {
        let from = Address::random();
        let fee_token = Some(Address::random());
        let nonce_key = U256::from(4242);
        let valid_after = Some(1800000000);
        let tempo_tx =
            TempoTransaction {
                chain_id: 42431,
                nonce: 42,
                gas_limit: 424242,
                fee_token,
                nonce_key,
                valid_after,
                ..Default::default()
            }
            .into_signed(TempoSignature::Primitive(PrimitiveSignature::Secp256k1(
                Signature::new(U256::ZERO, U256::ZERO, false),
            )));

        // Build the eth TxEnv to compare against op base
        let rpc_tempo_tx = RpcTransaction::from_transaction(
            Recovered::new_unchecked(tempo_tx.into(), from),
            TransactionInfo::default(),
        );

        let tx_env: TempoTxEnv = rpc_tempo_tx.try_any_to_tx_env().unwrap();
        assert_eq!(tx_env.inner.caller, from);
        assert_eq!(tx_env.inner.nonce, 42);
        assert_eq!(tx_env.inner.gas_limit, 424242);
        assert_eq!(tx_env.inner.chain_id, Some(42431));
        assert_eq!(tx_env.fee_token, fee_token);

        let tempo_tx_env = tx_env.tempo_tx_env.unwrap();
        assert_eq!(tempo_tx_env.nonce_key, nonce_key);
        assert_eq!(tempo_tx_env.valid_after, valid_after);
    }

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

    #[test]
    fn op_evm_foundry_context_ext_implementation() {
        let mut evm = OpEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());

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
}
