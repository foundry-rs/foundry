pub use alloy_evm::EvmEnv;
use alloy_primitives::{Address, B256, Bytes, U256};
use revm::{
    Context, Database,
    context::{Block, BlockEnv, Cfg, CfgEnv, JournalTr, Transaction, TxEnv},
    context_interface::{ContextTr, transaction::AccessList},
    primitives::{TxKind, hardfork::SpecId},
};

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: TxEnv,
}

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
impl Env {
    pub fn default_with_spec_id(spec_id: SpecId) -> Self {
        let mut cfg = CfgEnv::default();
        cfg.spec = spec_id;

        Self::from(cfg, BlockEnv::default(), TxEnv::default())
    }

    pub fn from(cfg: CfgEnv, block: BlockEnv, tx: TxEnv) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx }
    }

    pub fn new_with_spec_id(cfg: CfgEnv, block: BlockEnv, tx: TxEnv, spec_id: SpecId) -> Self {
        let mut cfg = cfg;
        cfg.spec = spec_id;

        Self::from(cfg, block, tx)
    }
}

/// Helper struct with mutable references to the block and cfg environments.
pub struct EnvMut<'a> {
    pub block: &'a mut BlockEnv,
    pub cfg: &'a mut CfgEnv,
    pub tx: &'a mut TxEnv,
}

impl EnvMut<'_> {
    /// Returns a copy of the environment.
    pub fn to_owned(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg.to_owned(), block_env: self.block.to_owned() },
            tx: self.tx.to_owned(),
        }
    }

    /// Writes an owned [`Env`] back into the context.
    ///
    /// Counterpart to [`to_owned`](Self::to_owned): completes the read/write pair so callers
    /// that receive an updated [`Env`] by value (e.g. after a fork switch or snapshot revert)
    /// can apply it without manually assigning each field.
    pub fn set_env(&mut self, env: Env) {
        *self.block = env.evm_env.block_env;
        *self.cfg = env.evm_env.cfg_env;
        *self.tx = env.tx;
    }
}

pub trait AsEnvMut {
    fn as_env_mut(&mut self) -> EnvMut<'_>;
}

impl AsEnvMut for EnvMut<'_> {
    fn as_env_mut(&mut self) -> EnvMut<'_> {
        EnvMut { block: self.block, cfg: self.cfg, tx: self.tx }
    }
}

impl AsEnvMut for Env {
    fn as_env_mut(&mut self) -> EnvMut<'_> {
        EnvMut {
            block: &mut self.evm_env.block_env,
            cfg: &mut self.evm_env.cfg_env,
            tx: &mut self.tx,
        }
    }
}

impl<DB: Database, J: JournalTr<Database = DB>, C> AsEnvMut
    for Context<BlockEnv, TxEnv, CfgEnv, DB, J, C>
{
    fn as_env_mut(&mut self) -> EnvMut<'_> {
        EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx }
    }
}

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

/// Extension of [`Cfg`] with mutable setters, allowing EVM-agnostic mutation of EVM configuration
/// fields.
pub trait FoundryCfg: Cfg {
    /// Sets the EVM spec (hardfork).
    fn set_spec(&mut self, spec: SpecId);

    /// Sets the chain ID.
    fn set_chain_id(&mut self, chain_id: u64);

    /// Sets the contract code size limit.
    fn set_limit_contract_code_size(&mut self, limit: Option<usize>);

    /// Sets the contract initcode size limit.
    fn set_limit_contract_initcode_size(&mut self, limit: Option<usize>);

    /// Sets whether nonce checks are disabled.
    fn set_disable_nonce_check(&mut self, disabled: bool);

    /// Sets the max blobs per transaction.
    fn set_max_blobs_per_tx(&mut self, max: Option<u64>);

    /// Sets the blob base fee update fraction.
    fn set_blob_base_fee_update_fraction(&mut self, fraction: Option<u64>);

    /// Sets the transaction gas limit cap.
    fn set_tx_gas_limit_cap(&mut self, cap: Option<u64>);
}

impl FoundryCfg for CfgEnv {
    fn set_spec(&mut self, spec: SpecId) {
        self.spec = spec;
    }

    fn set_chain_id(&mut self, chain_id: u64) {
        self.chain_id = chain_id;
    }

    fn set_limit_contract_code_size(&mut self, limit: Option<usize>) {
        self.limit_contract_code_size = limit;
    }

    fn set_limit_contract_initcode_size(&mut self, limit: Option<usize>) {
        self.limit_contract_initcode_size = limit;
    }

    fn set_disable_nonce_check(&mut self, disabled: bool) {
        self.disable_nonce_check = disabled;
    }

    fn set_max_blobs_per_tx(&mut self, max: Option<u64>) {
        self.max_blobs_per_tx = max;
    }

    fn set_blob_base_fee_update_fraction(&mut self, fraction: Option<u64>) {
        self.blob_base_fee_update_fraction = fraction;
    }

    fn set_tx_gas_limit_cap(&mut self, cap: Option<u64>) {
        self.tx_gas_limit_cap = cap;
    }
}

/// Extension trait providing mutable field access to block, tx, and cfg environments.
///
/// [`ContextTr`] only exposes immutable references for block, tx, and cfg.
/// Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()` need to mutate these fields.
///
/// Also provides [`journal_and_env_mut`](FoundryContextExt::journal_and_env_mut) for
/// simultaneous mutable access to journal and env — needed because calling `journal_mut()`
/// and `block_mut()` separately would create conflicting borrows on `&mut self`.
pub trait FoundryContextExt:
    ContextTr<Block: FoundryBlock, Tx: FoundryTransaction, Cfg: FoundryCfg>
{
    /// Mutable reference to the block environment.
    fn block_mut(&mut self) -> &mut BlockEnv;
    /// Mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut TxEnv;
    /// Mutable reference to the configuration environment.
    fn cfg_mut(&mut self) -> &mut CfgEnv;

    /// Returns a cloned snapshot of the current environment.
    fn to_env(&self) -> Env;

    /// Applies an owned [`Env`] to this context, replacing block, cfg, and tx.
    fn apply_env(&mut self, env: Env);

    /// Returns mutable references to the journal and environment simultaneously.
    ///
    /// This solves the borrow-splitting problem: calling `self.journal_mut()` and
    /// `self.block_mut()` separately would both borrow `&mut self`. This method
    /// splits the borrows at the field level in one call.
    fn journal_and_env_mut(&mut self) -> (&mut Self::Journal, EnvMut<'_>);
}

impl<DB: Database, J: JournalTr<Database = DB>, C> FoundryContextExt
    for Context<BlockEnv, TxEnv, CfgEnv, DB, J, C>
{
    fn block_mut(&mut self) -> &mut BlockEnv {
        &mut self.block
    }
    fn tx_mut(&mut self) -> &mut TxEnv {
        &mut self.tx
    }
    fn cfg_mut(&mut self) -> &mut CfgEnv {
        &mut self.cfg
    }
    fn to_env(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg.clone(), block_env: self.block.clone() },
            tx: self.tx.clone(),
        }
    }
    fn apply_env(&mut self, env: Env) {
        self.block = env.evm_env.block_env;
        self.cfg = env.evm_env.cfg_env;
        self.tx = env.tx;
    }
    fn journal_and_env_mut(&mut self) -> (&mut J, EnvMut<'_>) {
        (
            &mut self.journaled_state,
            EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx },
        )
    }
}
