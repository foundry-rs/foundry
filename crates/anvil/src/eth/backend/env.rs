//! Anvil's execution environment.
//!
//! Anvil keeps one network-agnostic environment for the next block and for RPC executions: the
//! block fields, the chain ID, the Ethereum-equivalent spec used to gate RPC behavior, and the
//! execution toggles configured by the user. It is converted into the selected network's evm2
//! environment right before execution, see [`crate::eth::backend::evm`].

use alloy_consensus::BlockHeader;
use alloy_network::{BlockResponse, Network};
use alloy_primitives::{B256, ChainId, U256};
use evm2::{BaseEvmTypes, EvmFeatures, Version};
use foundry_evm_core::{
    evm::EvmEnv as CoreEvmEnv,
    utils::{self, get_blob_base_fee_update_fraction},
};
use foundry_evm_networks::NetworkConfigs;

pub use evm2::SpecId;
pub use foundry_evm_core::fork::{BlobExcessGasAndPrice, BlockEnv};

/// Chain configuration and execution toggles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfgEnv {
    /// Ethereum-equivalent spec of the active hardfork.
    pub spec: SpecId,
    /// Chain ID used by `CHAINID` and transaction validation.
    pub chain_id: u64,
    /// Whether transactions must carry this chain's ID.
    pub tx_chain_id_check: bool,
    /// Contract code size limit, `None` for the spec's default.
    pub limit_contract_code_size: Option<usize>,
    /// Initcode size limit, `None` for twice the code size limit.
    pub limit_contract_initcode_size: Option<usize>,
    /// Skips the sender nonce check.
    pub disable_nonce_check: bool,
    /// Per-transaction gas limit cap, `None` for the spec's default.
    pub tx_gas_limit_cap: Option<u64>,
    /// Memory limit in bytes.
    pub memory_limit: u64,
    /// Skips the sender balance check.
    pub disable_balance_check: bool,
    /// Skips the block gas limit check.
    pub disable_block_gas_limit: bool,
    /// Allows senders with deployed code (EIP-3607).
    pub disable_eip3607: bool,
    /// Skips the base fee check.
    pub disable_base_fee: bool,
    /// Skips the blob base fee check for zero-cap simulations.
    pub disable_blob_base_fee: bool,
    /// Skips the priority fee check.
    pub disable_priority_fee_check: bool,
    /// Skips charging the transaction fee.
    pub disable_fee_charge: bool,
}

impl Default for CfgEnv {
    fn default() -> Self {
        Self {
            spec: SpecId::default(),
            chain_id: 1,
            tx_chain_id_check: true,
            limit_contract_code_size: None,
            limit_contract_initcode_size: None,
            disable_nonce_check: false,
            tx_gas_limit_cap: None,
            memory_limit: (1 << 32) - 1,
            disable_balance_check: false,
            disable_block_gas_limit: false,
            disable_eip3607: false,
            disable_base_fee: false,
            disable_blob_base_fee: false,
            disable_priority_fee_check: false,
            disable_fee_charge: false,
        }
    }
}

impl CfgEnv {
    /// Returns the effective per-transaction gas limit cap.
    pub fn tx_gas_limit_cap(&self) -> u64 {
        self.tx_gas_limit_cap.unwrap_or(evm2::Version::new(self.spec).tx_gas_limit_cap)
    }

    /// Returns the effective contract code size limit.
    pub fn max_code_size(&self) -> usize {
        self.limit_contract_code_size.unwrap_or(evm2::Version::new(self.spec).max_code_size)
    }

    /// Returns the effective initcode size limit.
    pub fn max_initcode_size(&self) -> usize {
        self.limit_contract_initcode_size.unwrap_or_else(|| {
            if self.limit_contract_code_size.is_some() {
                2 * self.max_code_size()
            } else {
                evm2::Version::new(self.spec).max_initcode_size
            }
        })
    }
}

/// The configuration and block environment of an execution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvmEnv {
    /// Chain configuration and execution toggles.
    pub cfg_env: CfgEnv,
    /// Block environment.
    pub block_env: BlockEnv,
}

impl EvmEnv {
    /// Creates an environment.
    pub const fn new(cfg_env: CfgEnv, block_env: BlockEnv) -> Self {
        Self { cfg_env, block_env }
    }

    /// Returns the Ethereum-equivalent spec.
    pub const fn spec_id(&self) -> &SpecId {
        &self.cfg_env.spec
    }
}

/// Constructs a block environment from a block header.
pub fn block_env_from_header(header: &impl BlockHeader) -> BlockEnv {
    BlockEnv {
        number: U256::from(header.number()),
        beneficiary: header.beneficiary(),
        timestamp: U256::from(header.timestamp()),
        gas_limit: header.gas_limit(),
        basefee: header.base_fee_per_gas().unwrap_or_default(),
        difficulty: header.difficulty(),
        prevrandao: header.mix_hash(),
        slot_num: header.slot_number().unwrap_or_default(),
        ..Default::default()
    }
}

/// Returns `env` as an Ethereum evm2 environment, for the chain helpers of
/// [`foundry_evm_core::utils`].
fn core_env(env: &EvmEnv) -> CoreEvmEnv<BaseEvmTypes> {
    let spec = env.cfg_env.spec;
    CoreEvmEnv::new(
        spec,
        Version::new(spec),
        crate::eth::backend::evm::block_env::<BaseEvmTypes>(env),
    )
}

/// Applies [`utils::apply_chain_specific_tx_replay_env_changes_for_chain`] to `evm_env`.
pub fn apply_chain_specific_tx_replay_env_changes_for_chain(
    evm_env: &mut EvmEnv,
    source_chain_id: ChainId,
) {
    let mut env = core_env(evm_env);
    utils::apply_chain_specific_tx_replay_env_changes_for_chain(&mut env, source_chain_id);
    let cfg = &mut evm_env.cfg_env;
    cfg.disable_priority_fee_check |= !env.version.feature(EvmFeatures::PRIORITY_FEE_CHECK);
    cfg.disable_base_fee |= !env.version.feature(EvmFeatures::BASE_FEE_CHECK);
}

/// Applies [`utils::apply_chain_and_block_specific_env_changes_for_chain`] to `evm_env`.
pub fn apply_chain_and_block_specific_env_changes_for_chain<N: Network>(
    evm_env: &mut EvmEnv,
    block: &N::BlockResponse,
    source_chain_id: ChainId,
    configs: NetworkConfigs,
) {
    let header = block.header();
    if let Some(excess_blob_gas) = header.excess_blob_gas() {
        evm_env.block_env.set_blob_excess_gas_and_price(
            excess_blob_gas,
            get_blob_base_fee_update_fraction(source_chain_id, header.timestamp()),
        );
    }
    let mut env = core_env(evm_env);
    utils::apply_chain_and_block_specific_env_changes_for_chain::<N, _>(
        &mut env,
        block,
        source_chain_id,
        configs,
    );
    let block_env = &mut evm_env.block_env;
    block_env.number = env.block.number;
    block_env.difficulty = env.block.difficulty;
    let prevrandao = B256::from(env.block.prevrandao);
    if prevrandao != block_env.prevrandao.unwrap_or_default() {
        block_env.prevrandao = Some(prevrandao);
    }
}
