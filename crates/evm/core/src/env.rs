use std::fmt::Debug;

pub use alloy_evm::EvmEnv;
use alloy_evm::{FromRecoveredTx, ToTxEnv};
use alloy_network::{AnyRpcTransaction, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, U256};
use op_revm::{
    OpTransaction,
    transaction::{OpTxTr, deposit::DEPOSIT_TRANSACTION_TYPE},
};
use revm::{
    Context, Database, Journal,
    context::{Block, BlockEnv, Cfg, CfgEnv, Transaction, TxEnv},
    context_interface::{ContextTr, transaction::AccessList},
    inspector::JournalExt,
    primitives::{TxKind, hardfork::SpecId},
};

use crate::backend::{DatabaseExt, JournaledState};

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

    /// Sets the max priority fee per gas.
    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>);

    /// Sets the blob versioned hashes.
    fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>);

    /// Sets the max fee per blob gas.
    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128);

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

/// Temporary bound alias used during the transition to a fully generic foundry-evm and
/// foundry-cheatcodes.
///
/// Pins the EVM context to Ethereum-specific environment types (`BlockEnv`, `TxEnv`, `CfgEnv`)
/// so that cheatcode implementations don't need to repeat the full where-clause everywhere.
/// Once cheatcodes are fully generic over network/environment types this alias will be removed.
pub trait EthCheatCtx:
    FoundryContextExt<
        Spec = SpecId,
        Block = BlockEnv,
        Tx = TxEnv,
        Cfg = CfgEnv,
        Db: DatabaseExt<Self::Block, Self::Tx, Self::Spec>,
    >
{
}
impl<CTX> EthCheatCtx for CTX where
    CTX: FoundryContextExt<
            Spec = SpecId,
            Block = BlockEnv,
            Tx = TxEnv,
            Cfg = CfgEnv,
            Db: DatabaseExt<Self::Block, Self::Tx, Self::Spec>,
        >
{
}

/// Abstraction trait for converting any RPC transaction into corresponding `TxEnv`.
///
/// This trait bridges the gap between different network RPC transaction types and the EVM's
/// `TxEnv`:
/// - For [`alloy_rpc_types::Transaction`] (Ethereum): delegates to [`ToTxEnv`].
/// - For [`AnyRpcTransaction`] (AnyNetwork): extracts the inner [`alloy_consensus::TxEnvelope`] via
///   [`as_envelope()`](alloy_network::AnyTxEnvelope::as_envelope) then delegates to
///   [`FromRecoveredTx`].
/// - For [`op_alloy_rpc_types::Transaction`] (Optimism): delegates to [`ToTxEnv`].
pub trait TryAnyToTxEnv<TxEnv> {
    /// Tries to convert this RPC transaction into a [`TxEnv`].
    fn try_any_to_tx_env(&self) -> eyre::Result<TxEnv>;
}

impl TryAnyToTxEnv<TxEnv> for alloy_rpc_types::Transaction {
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

impl TryAnyToTxEnv<OpTransaction<TxEnv>> for op_alloy_rpc_types::Transaction {
    fn try_any_to_tx_env(&self) -> eyre::Result<OpTransaction<TxEnv>> {
        Ok(self.as_recovered().to_tx_env())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Sealed, Signed, TxEip1559, transaction::Recovered};
    use alloy_network::{AnyTxEnvelope, AnyTxType, UnknownTxEnvelope, UnknownTypedTransaction};
    use alloy_primitives::Signature;
    use alloy_rpc_types::{Transaction, TransactionInfo};
    use alloy_serde::WithOtherFields;
    use op_alloy_consensus::{OpTxEnvelope, TxDeposit, transaction::OpTransactionInfo};

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
        let tx = Transaction::from_transaction(
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
        let any_tx = <AnyRpcTransaction as From<Transaction>>::from(tx);
        let any_tx_env: TxEnv = any_tx.try_any_to_tx_env().unwrap();

        // TxEnv from AnyRpcTransaction must be the same
        assert_eq!(tx_env, any_tx_env);
    }

    #[test]
    fn try_any_to_tx_env_for_op_transactions() {
        let from = Address::random();
        let signed_tx = make_signed_eip1559();

        // Build the eth TxEnv to compare against op base
        let eth_tx = Transaction::from_transaction(
            Recovered::new_unchecked(signed_tx.clone().into(), from),
            TransactionInfo::default(),
        );
        let expected_base: TxEnv = eth_tx.try_any_to_tx_env().unwrap();

        let op_tx = op_alloy_rpc_types::Transaction::from_transaction(
            Recovered::new_unchecked(signed_tx.into(), from),
            OpTransactionInfo::default(),
        );
        let op_tx_env: OpTransaction<TxEnv> = op_tx.try_any_to_tx_env().unwrap();

        assert_eq!(op_tx_env.base, expected_base);

        // Test with Deposit tx
        let op_deposit_tx = op_alloy_rpc_types::Transaction::from_transaction(
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
        let any_tx = AnyRpcTransaction::new(WithOtherFields::new(Transaction {
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
}
