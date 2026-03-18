use std::fmt::Debug;

pub use alloy_evm::EvmEnv;
use alloy_primitives::{Address, B256, Bytes, U256};
use revm::{
    Context, Database,
    context::{Block, BlockEnv, CfgEnv, JournalTr, Transaction, TxEnv},
    context_interface::{ContextTr, transaction::AccessList},
    primitives::{TxKind, hardfork::SpecId},
};

use crate::backend::{DatabaseExt, FoundryJournalExt};

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

/// Marker trait for Foundry's [`CfgEnv`] type, abstracting `Spec` type.
pub trait FoundryCfg: Clone + Debug {
    type Spec: Into<SpecId> + Clone + Debug;
}

impl<SPEC: Into<SpecId> + Clone + Debug> FoundryCfg for CfgEnv<SPEC> {
    type Spec = SPEC;
}

/// Extension trait providing mutable field access to block, tx, and cfg environments.
///
/// [`ContextTr`] only exposes immutable references for block, tx, and cfg.
/// Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()` need to mutate these fields.
pub trait FoundryContextExt:
    ContextTr<Block: FoundryBlock + Clone, Tx: FoundryTransaction + Clone, Cfg: FoundryCfg>
{
    /// Mutable reference to the block environment.
    fn block_mut(&mut self) -> &mut Self::Block;
    /// Mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut Self::Tx;
    /// Mutable reference to the configuration environment.
    fn cfg_mut(&mut self) -> &mut Self::Cfg;
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
    /// Sets EVM environment.
    fn set_evm(&mut self, evm_env: EvmEnv<<Self::Cfg as FoundryCfg>::Spec, Self::Block>)
    where
        Self::Cfg: From<CfgEnv<<Self::Cfg as FoundryCfg>::Spec>>,
    {
        *self.cfg_mut() = evm_env.cfg_env.into();
        *self.block_mut() = evm_env.block_env;
    }
    /// Cloned transaction environment.
    fn tx_clone(&self) -> Self::Tx {
        self.tx().clone()
    }
    /// Cloned EVM environment (Cfg + Block).
    fn evm_clone(&self) -> EvmEnv<<Self::Cfg as FoundryCfg>::Spec, Self::Block>
    where
        Self::Cfg: Into<CfgEnv<<Self::Cfg as FoundryCfg>::Spec>>,
    {
        EvmEnv::new(self.cfg().clone().into(), self.block().clone())
    }
}

impl<DB: Database, J: JournalTr<Database = DB>, C> FoundryContextExt
    for Context<BlockEnv, TxEnv, CfgEnv, DB, J, C>
{
    fn block_mut(&mut self) -> &mut Self::Block {
        &mut self.block
    }
    fn tx_mut(&mut self) -> &mut Self::Tx {
        &mut self.tx
    }
    fn cfg_mut(&mut self) -> &mut Self::Cfg {
        &mut self.cfg
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
        Block = BlockEnv,
        Tx = TxEnv,
        Cfg = CfgEnv,
        Journal: FoundryJournalExt<Self>,
        Db: DatabaseExt,
    >
{
}
impl<CTX> EthCheatCtx for CTX where
    CTX: FoundryContextExt<
            Block = BlockEnv,
            Tx = TxEnv,
            Cfg = CfgEnv,
            Journal: FoundryJournalExt<Self>,
            Db: DatabaseExt,
        >
{
}
