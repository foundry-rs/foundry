use ethers::types::{Address, U256};
use revm::{BlockEnv, CfgEnv, SpecId, TxEnv};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvmOpts {
    #[serde(flatten)]
    pub env: Env,

    /// fetch state over a remote instead of starting from empty state
    #[serde(rename = "eth_rpc_url")]
    pub fork_url: Option<String>,

    /// pins the block number for the state fork
    pub fork_block_number: Option<u64>,

    /// the initial balance of each deployed test contract
    pub initial_balance: U256,

    /// the address which will be executing all tests
    pub sender: Address,

    /// enables the FFI cheatcode
    pub ffi: bool,

    /// Verbosity mode of EVM output as number of occurences
    pub verbosity: u8,

    /// enable debugger
    pub debug: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Env {
    /// the block gas limit
    pub gas_limit: u64,

    /// the chainid opcode value
    pub chain_id: Option<u64>,

    /// the tx.gasprice value during EVM execution
    pub gas_price: u64,

    /// the base fee in a block
    pub block_base_fee_per_gas: u64,

    /// the tx.origin value during EVM execution
    pub tx_origin: Address,

    /// the block.coinbase value during EVM execution
    pub block_coinbase: Address,

    /// the block.timestamp value during EVM execution
    pub block_timestamp: u64,

    /// the block.number value during EVM execution"
    pub block_number: u64,

    /// the block.difficulty value during EVM execution
    pub block_difficulty: u64,

    /// the block.gaslimit value during EVM execution
    pub block_gas_limit: Option<u64>,
}

impl Env {
    pub fn evm_env(&self) -> revm::Env {
        revm::Env {
            block: BlockEnv {
                number: self.block_number.into(),
                coinbase: self.block_coinbase,
                timestamp: self.block_timestamp.into(),
                difficulty: self.block_difficulty.into(),
                basefee: self.block_base_fee_per_gas.into(),
                gas_limit: self.block_gas_limit.unwrap_or(self.gas_limit).into(),
            },
            cfg: CfgEnv {
                chain_id: self.chain_id.unwrap_or(99).into(),
                spec_id: SpecId::LONDON,
                perf_all_precompiles_have_balance: false,
            },
            tx: TxEnv {
                gas_price: self.gas_price.into(),
                gas_limit: self.block_gas_limit.unwrap_or(self.gas_limit),
                ..Default::default()
            },
        }
    }
}
