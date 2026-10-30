//! Anvil's execution environment.
//!
//! Anvil keeps one network-agnostic environment for the next block and for RPC executions: the
//! block fields, the chain ID, the Ethereum-equivalent spec used to gate RPC behavior, and the
//! execution toggles configured by the user. It is converted into the selected network's evm2
//! environment right before execution, see [`crate::eth::backend::evm`].

use alloy_chains::NamedChain;
use alloy_consensus::BlockHeader;
use alloy_eips::{
    eip4844::{BLOB_TX_MIN_BLOB_GASPRICE, fake_exponential},
    eip7691::BLOB_GASPRICE_UPDATE_FRACTION_PECTRA,
};
use alloy_network::{BlockResponse, Network};
use alloy_primitives::{Address, B256, ChainId, U256};
use foundry_evm_core::utils::get_blob_base_fee_update_fraction;
use foundry_evm_networks::NetworkConfigs;
use serde::{Deserialize, Serialize};

pub use evm2::SpecId;

/// Excess blob gas of a block and the blob gas price derived from it.
///
/// Serializes like the block environment of Anvil state dumps written before the evm2 migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobExcessGasAndPrice {
    /// The excess blob gas of the block.
    pub excess_blob_gas: u64,
    /// The blob gas price derived from `excess_blob_gas`.
    pub blob_gasprice: u128,
}

impl BlobExcessGasAndPrice {
    /// Creates a new instance, deriving the blob gas price from the excess blob gas.
    pub fn new(excess_blob_gas: u64, blob_base_fee_update_fraction: u64) -> Self {
        Self {
            excess_blob_gas,
            blob_gasprice: fake_exponential(
                BLOB_TX_MIN_BLOB_GASPRICE,
                excess_blob_gas as u128,
                blob_base_fee_update_fraction as u128,
            ),
        }
    }
}

/// The block environment of the next block or of a replayed block.
///
/// Serializes like the block environment of Anvil state dumps written before the evm2 migration.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockEnv {
    /// Block number.
    pub number: U256,
    /// Block producer.
    pub beneficiary: Address,
    /// Block timestamp in seconds.
    pub timestamp: U256,
    /// Block gas limit.
    pub gas_limit: u64,
    /// Block base fee.
    pub basefee: u64,
    /// Block difficulty, before the merge.
    pub difficulty: U256,
    /// Block `prevrandao`, after the merge.
    pub prevrandao: Option<B256>,
    /// Excess blob gas and blob gas price, after Cancun.
    pub blob_excess_gas_and_price: Option<BlobExcessGasAndPrice>,
    /// Slot number, after Amsterdam.
    #[serde(default)]
    pub slot_num: u64,
}

impl Default for BlockEnv {
    fn default() -> Self {
        Self {
            number: U256::ZERO,
            beneficiary: Address::ZERO,
            timestamp: U256::ONE,
            gas_limit: u64::MAX,
            basefee: 0,
            difficulty: U256::ZERO,
            prevrandao: Some(B256::ZERO),
            blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
                0,
                BLOB_GASPRICE_UPDATE_FRACTION_PECTRA as u64,
            )),
            slot_num: 0,
        }
    }
}

impl BlockEnv {
    /// Sets the excess blob gas and derives the blob gas price from it.
    pub fn set_blob_excess_gas_and_price(
        &mut self,
        excess_blob_gas: u64,
        base_fee_update_fraction: u64,
    ) {
        self.blob_excess_gas_and_price =
            Some(BlobExcessGasAndPrice::new(excess_blob_gas, base_fee_update_fraction));
    }

    /// Returns the excess blob gas, if blobs are enabled.
    pub fn blob_excess_gas(&self) -> Option<u64> {
        self.blob_excess_gas_and_price.map(|blob| blob.excess_blob_gas)
    }

    /// Returns the blob gas price, if blobs are enabled.
    pub fn blob_gasprice(&self) -> Option<u128> {
        self.blob_excess_gas_and_price.map(|blob| blob.blob_gasprice)
    }
}

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

/// Applies replay normalization for the provided source chain.
///
/// This keeps fork-specific transaction validation independent from an execution `CHAINID`
/// override.
pub fn apply_chain_specific_tx_replay_env_changes_for_chain(
    evm_env: &mut EvmEnv,
    source_chain_id: ChainId,
) {
    if NamedChain::try_from(source_chain_id).is_ok_and(|chain| chain.is_arbitrum()) {
        // Arbitrum does not enforce the EIP-1559 priority fee ordering constraint.
        evm_env.cfg_env.disable_priority_fee_check = true;
    }
}

/// Applies block normalization for the provided source chain.
///
/// - checks for prevrandao mixhash after merge
/// - applies chain specifics: on Arbitrum `block.number` is the L1 block
///
/// This keeps fork-specific header handling independent from an execution `CHAINID` override.
pub fn apply_chain_and_block_specific_env_changes_for_chain<N: Network>(
    evm_env: &mut EvmEnv,
    block: &N::BlockResponse,
    source_chain_id: ChainId,
    configs: NetworkConfigs,
) {
    let header = block.header();
    // The blob fee market is priced from the header's excess blob gas and the source chain's
    // blob schedule at the block timestamp. Headers without the field (pre-Cancun blocks and
    // chains without EIP-4844) keep the default blob environment.
    if let Some(excess_blob_gas) = header.excess_blob_gas() {
        evm_env.block_env.set_blob_excess_gas_and_price(
            excess_blob_gas,
            get_blob_base_fee_update_fraction(source_chain_id, header.timestamp()),
        );
    }

    let block_env = &mut evm_env.block_env;
    let difficulty_as_prevrandao = Some(B256::from(block_env.difficulty));
    if let Ok(chain) = NamedChain::try_from(source_chain_id) {
        match chain {
            NamedChain::BinanceSmartChain
            | NamedChain::BinanceSmartChainTestnet
            | NamedChain::Polygon
            | NamedChain::PolygonAmoy
            | NamedChain::Avalanche
            | NamedChain::AvalancheFuji => {
                // https://github.com/foundry-rs/foundry/issues/9942
                // As far as observed from the source code of bnb-chain/bsc, the `difficulty` field
                // is still in use and returned by the corresponding opcode but `prevrandao`
                // (`mixHash`) is always zero, even though bsc adopts the newer EVM
                // specification. Polygon and Avalanche behave the same way.
                block_env.prevrandao = difficulty_as_prevrandao;
                return;
            }
            c if c.is_arbitrum() => {
                // On Arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field.
                if let Some(l1_block_number) = block
                    .other_fields()
                    .and_then(|other| other.get("l1BlockNumber").cloned())
                    .and_then(|l1_block_number| {
                        serde_json::from_value::<U256>(l1_block_number).ok()
                    })
                {
                    block_env.number = l1_block_number;
                }

                // `mixHash` carries L1 metadata rather than randomness here, while the
                // `PREVRANDAO` opcode returns `difficulty` like it does on the chains above.
                block_env.prevrandao = difficulty_as_prevrandao;
            }
            _ => {}
        }
    }

    if configs.bypass_prevrandao(source_chain_id) && block_env.prevrandao.is_none() {
        // Chains without randomness keep the historical DIFFICULTY opcode semantics.
        // <https://github.com/foundry-rs/foundry/issues/4232>
        block_env.prevrandao = difficulty_as_prevrandao;
    }

    // If difficulty is `0` we assume it's past merge.
    if header.difficulty().is_zero() {
        block_env.difficulty = block_env.prevrandao.unwrap_or_default().into();
    }
}
