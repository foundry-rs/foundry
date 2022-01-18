use clap::Parser;
use ethers::types::{Address, U256};
use evm_adapters::evm_opts::EvmType;
use foundry_config::{
    figment::{
        self,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Metadata, Profile, Provider,
    },
    Config,
};
use serde::Serialize;

#[derive(Debug, Clone, Parser, Serialize)]
pub struct EvmArgs {
    #[clap(flatten)]
    #[serde(flatten)]
    pub env: EnvArgs,

    #[clap(
        long,
        short,
        help = "the EVM type you want to use (e.g. sputnik, evmodin)",
        default_value = "sputnik"
    )]
    #[serde(skip)]
    pub evm_type: EvmType,

    #[clap(help = "fetch state over a remote instead of starting from empty state", long, short)]
    #[clap(alias = "rpc-url")]
    #[serde(rename = "eth_rpc_url", skip_serializing_if = "Option::is_none")]
    pub fork_url: Option<String>,

    #[clap(help = "pins the block number for the state fork", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<u64>,

    #[clap(help = "the initial balance of each deployed test contract", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_balance: Option<U256>,

    #[clap(help = "the address which will be executing all tests", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Address>,

    #[clap(help = "enables the FFI cheatcode", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ffi: Option<bool>,

    #[clap(
        help = r#"Verbosity mode of EVM output as number of occurences of the `v` flag (-v, -vv, -vvv, etc.)
    3: print test trace for failing tests
    4: always print test trace, print setup for failing tests
    5: always print test trace and setup
"#,
        long,
        short,
        parse(from_occurrences)
    )]
    #[serde(skip)]
    pub verbosity: u8,

    #[clap(help = "enable debugger", long)]
    pub debug: bool,
}

// Make this set of options a `figment::Provider` so that it can be merged into the `Config`
impl Provider for EvmArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Evm Opts Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let mut dict = value.into_dict().ok_or(error)?;

        if self.verbosity > 0 {
            dict.insert("verbosity".to_string(), self.verbosity.into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

#[derive(Debug, Clone, Default, Parser, Serialize)]
pub struct EnvArgs {
    // structopt does not let use `u64::MAX`:
    // https://doc.rust-lang.org/std/primitive.u64.html#associatedconstant.MAX
    #[clap(help = "the block gas limit", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,

    #[clap(help = "the chainid opcode value", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,

    #[clap(help = "the tx.gasprice value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<u64>,

    #[clap(help = "the base fee in a block", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_base_fee_per_gas: Option<u64>,

    #[clap(help = "the tx.origin value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_origin: Option<Address>,

    #[clap(help = "the block.coinbase value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_coinbase: Option<Address>,
    #[clap(help = "the block.timestamp value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_timestamp: Option<u64>,

    #[clap(help = "the block.number value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,

    #[clap(help = "the block.difficulty value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_difficulty: Option<u64>,

    #[clap(help = "the block.gaslimit value during EVM execution", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_gas_limit: Option<u64>,
    // TODO: Add configuration option for base fee.
}
