mod runner;
use ethers::types::{Address, U256};
use evmodin::util::mocked_host::MockedHost;
pub use runner::{ContractRunner, TestKind, TestKindGas, TestResult};
use sputnik::backend::MemoryVicinity;
use std::str::FromStr;
use structopt::StructOpt;

mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

pub trait TestFilter {
    fn matches_test(&self, test_name: &str) -> bool;
    fn matches_contract(&self, contract_name: &str) -> bool;
}

#[derive(Clone, Debug)]
pub enum EvmType {
    Sputnik,
    EvmOdin,
}

impl FromStr for EvmType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "sputnik" => EvmType::Sputnik,
            "evmodin" => EvmType::EvmOdin,
            other => eyre::bail!("unknown EVM type {}", other),
        })
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct EvmOpts {
    #[structopt(flatten)]
    pub env: Env,

    #[structopt(
        long,
        short,
        help = "the EVM type you want to use (e.g. sputnik, evmodin)",
        default_value = "sputnik"
    )]
    pub evm_type: EvmType,

    #[structopt(
        help = "fetch state over a remote instead of starting from empty state",
        long,
        short
    )]
    #[structopt(alias = "rpc-url")]
    pub fork_url: Option<String>,

    #[structopt(help = "pins the block number for the state fork", long)]
    #[structopt(env = "DAPP_FORK_BLOCK")]
    pub fork_block_number: Option<u64>,

    #[structopt(
        help = "the initial balance of each deployed test contract",
        long,
        default_value = "0xffffffffffffffffffffffff"
    )]
    pub initial_balance: U256,

    #[structopt(
        help = "the address which will be executing all tests",
        long,
        default_value = "0x0000000000000000000000000000000000000000",
        env = "DAPP_TEST_ADDRESS"
    )]
    pub sender: Address,

    #[structopt(help = "enables the FFI cheatcode", long)]
    pub ffi: bool,

    #[structopt(
        help = r#"Verbosity mode of EVM output as number of occurences of the `v` flag (-v, -vv, -vvv, etc.)
    3: print test trace for failing tests
    4: always print test trace, print setup for failing tests
    5: always print test trace and setup
"#,
        long,
        short,
        parse(from_occurrences)
    )]
    pub verbosity: u8,

    #[structopt(help = "enable debugger", long)]
    pub debug: bool,
}

impl EvmOpts {
    pub fn vicinity(&self) -> eyre::Result<MemoryVicinity> {
        Ok(if let Some(ref url) = self.fork_url {
            let provider = ethers::providers::Provider::try_from(url.as_str())?;
            let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
            rt.block_on(evm_adapters::sputnik::vicinity(
                &provider,
                self.fork_block_number,
                Some(self.env.tx_origin),
            ))?
        } else {
            self.env.sputnik_state()
        })
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct Env {
    // structopt does not let use `u64::MAX`:
    // https://doc.rust-lang.org/std/primitive.u64.html#associatedconstant.MAX
    #[structopt(help = "the block gas limit", long, default_value = "18446744073709551615")]
    pub gas_limit: u64,

    #[structopt(help = "the chainid opcode value", long, default_value = "1")]
    pub chain_id: u64,

    #[structopt(help = "the tx.gasprice value during EVM execution", long, default_value = "0")]
    pub gas_price: u64,

    #[structopt(help = "the base fee in a block", long, default_value = "0")]
    pub block_base_fee_per_gas: u64,

    #[structopt(
        help = "the tx.origin value during EVM execution",
        long,
        default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub tx_origin: Address,

    #[structopt(
    help = "the block.coinbase value during EVM execution",
    long,
    // TODO: It'd be nice if we could use Address::zero() here.
    default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub block_coinbase: Address,
    #[structopt(
        help = "the block.timestamp value during EVM execution",
        long,
        default_value = "0",
        env = "DAPP_TEST_TIMESTAMP"
    )]
    pub block_timestamp: u64,

    #[structopt(help = "the block.number value during EVM execution", long, default_value = "0")]
    #[structopt(env = "DAPP_TEST_NUMBER")]
    pub block_number: u64,

    #[structopt(
        help = "the block.difficulty value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_difficulty: u64,

    #[structopt(help = "the block.gaslimit value during EVM execution", long)]
    pub block_gas_limit: Option<u64>,
    // TODO: Add configuration option for base fee.
}

impl Env {
    pub fn sputnik_state(&self) -> MemoryVicinity {
        MemoryVicinity {
            chain_id: self.chain_id.into(),

            gas_price: self.gas_price.into(),
            origin: self.tx_origin,

            block_coinbase: self.block_coinbase,
            block_number: self.block_number.into(),
            block_timestamp: self.block_timestamp.into(),
            block_difficulty: self.block_difficulty.into(),
            block_base_fee_per_gas: self.block_base_fee_per_gas.into(),
            block_gas_limit: self.block_gas_limit.unwrap_or(self.gas_limit).into(),
            block_hashes: Vec::new(),
        }
    }

    pub fn evmodin_state(&self) -> MockedHost {
        let mut host = MockedHost::default();

        host.tx_context.chain_id = self.chain_id.into();
        host.tx_context.tx_gas_price = self.gas_price.into();
        host.tx_context.tx_origin = self.tx_origin;
        host.tx_context.block_coinbase = self.block_coinbase;
        host.tx_context.block_number = self.block_number;
        host.tx_context.block_timestamp = self.block_timestamp;
        host.tx_context.block_difficulty = self.block_difficulty.into();
        host.tx_context.block_gas_limit = self.block_gas_limit.unwrap_or(self.gas_limit);

        host
    }
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;
    use ethers::{
        prelude::Lazy,
        solc::{CompilerOutput, Project, ProjectPathsConfig},
    };
    use regex::Regex;

    pub static COMPILED: Lazy<CompilerOutput> = Lazy::new(|| {
        // NB: should we add a test-helper function that makes creating these
        // ephemeral projects easier?
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        project.compile().unwrap().output()
    });

    pub struct Filter {
        test_regex: Regex,
        contract_regex: Regex,
    }

    impl Filter {
        pub fn new(test_pattern: &str, contract_pattern: &str) -> Self {
            return Filter {
                test_regex: Regex::new(test_pattern).unwrap(),
                contract_regex: Regex::new(contract_pattern).unwrap(),
            }
        }
    }

    impl TestFilter for Filter {
        fn matches_test(&self, test_name: &str) -> bool {
            self.test_regex.is_match(test_name)
        }

        fn matches_contract(&self, contract_name: &str) -> bool {
            self.contract_regex.is_match(contract_name)
        }
    }
}
