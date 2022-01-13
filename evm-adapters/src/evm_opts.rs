use clap::Parser;
use ethers::types::{Address, U256};
use std::str::FromStr;

#[cfg(feature = "evmodin")]
use evmodin::util::mocked_host::MockedHost;
#[cfg(feature = "sputnik")]
use sputnik::backend::MemoryVicinity;

#[derive(Clone, Debug)]
pub enum EvmType {
    #[cfg(feature = "sputnik")]
    Sputnik,
    #[cfg(feature = "evmodin")]
    EvmOdin,
}

#[cfg(any(feature = "sputnik", feature = "evmodin"))]
impl Default for EvmType {
    fn default() -> Self {
        // if sputnik is enabled, default to it
        #[cfg(feature = "sputnik")]
        #[rustfmt::skip]
        return EvmType::Sputnik;
        // if not, fall back to evmodin
        #[allow(unreachable_code)]
        #[cfg(feature = "evmodin")]
        EvmType::EvmOdin
    }
}

impl FromStr for EvmType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // silence this warning which indicates that if no evm features are
        // enabled, the Ok(...) will never be reached.
        #[allow(unreachable_code)]
        Ok(match s.to_lowercase().as_str() {
            #[cfg(feature = "sputnik")]
            "sputnik" => EvmType::Sputnik,
            #[cfg(feature = "evmodin")]
            "evmodin" => EvmType::EvmOdin,
            other => eyre::bail!("unknown EVM type {}", other),
        })
    }
}

#[derive(Debug, Clone, Parser)]
#[cfg_attr(any(feature = "sputnik", feature = "evmodin"), derive(Default))]
pub struct EvmOpts {
    #[clap(flatten)]
    pub env: Env,

    #[clap(
        long,
        short,
        help = "the EVM type you want to use (e.g. sputnik, evmodin)",
        default_value = "sputnik"
    )]
    pub evm_type: EvmType,

    #[clap(help = "fetch state over a remote instead of starting from empty state", long, short)]
    #[clap(alias = "rpc-url")]
    pub fork_url: Option<String>,

    #[clap(help = "pins the block number for the state fork", long)]
    #[clap(env = "DAPP_FORK_BLOCK")]
    pub fork_block_number: Option<u64>,

    #[clap(
        help = "the initial balance of each deployed test contract",
        long,
        default_value = "0xffffffffffffffffffffffff"
    )]
    pub initial_balance: U256,

    #[clap(
        help = "the address which will be executing all tests",
        long,
        default_value = "0x00a329c0648769A73afAc7F9381E08FB43dBEA72",
        env = "DAPP_TEST_CALLER"
    )]
    pub sender: Address,

    #[clap(help = "enables the FFI cheatcode", long)]
    pub ffi: bool,

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
    pub verbosity: u8,

    #[clap(help = "enable debugger", long)]
    pub debug: bool,
}

#[cfg(feature = "sputnik")]
pub use sputnik_helpers::BackendKind;

// Helper functions for sputnik
#[cfg(feature = "sputnik")]
mod sputnik_helpers {
    use super::*;

    use crate::{sputnik::cache::SharedBackend, FAUCET_ACCOUNT};
    use ethers::providers::Provider;
    use sputnik::backend::MemoryBackend;

    pub enum BackendKind<'a> {
        Simple(MemoryBackend<'a>),
        Shared(SharedBackend),
    }

    impl EvmOpts {
        #[cfg(feature = "sputnik")]
        pub fn backend<'a>(
            &'a self,
            vicinity: &'a MemoryVicinity,
        ) -> eyre::Result<BackendKind<'a>> {
            let mut backend = MemoryBackend::new(vicinity, Default::default());
            // max out the balance of the faucet
            let faucet =
                backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
            faucet.balance = U256::MAX;
            // set deployer nonce to 1 to get the same contract addresses
            // as dapptools, provided the sender is also
            // `0x00a329c0648769A73afAc7F9381E08FB43dBEA72`
            let deployer = backend.state_mut().entry(self.sender).or_insert_with(Default::default);
            deployer.nonce = U256::from(1);

            let backend = if let Some(ref url) = self.fork_url {
                let provider = Provider::try_from(url.as_str())?;
                let init_state = backend.state().clone();
                let cache = crate::sputnik::new_shared_cache(init_state);
                let backend = SharedBackend::new(
                    provider,
                    cache,
                    vicinity.clone(),
                    self.fork_block_number.map(Into::into),
                );
                BackendKind::Shared(backend)
            } else {
                BackendKind::Simple(backend)
            };

            Ok(backend)
        }

        #[cfg(feature = "sputnik")]
        pub fn vicinity(&self) -> eyre::Result<MemoryVicinity> {
            Ok(if let Some(ref url) = self.fork_url {
                let provider = ethers::providers::Provider::try_from(url.as_str())?;
                let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
                rt.block_on(crate::sputnik::vicinity(
                    &provider,
                    self.fork_block_number,
                    Some(self.env.tx_origin),
                ))?
            } else {
                self.env.sputnik_state()
            })
        }
    }
}

#[derive(Debug, Clone, Default, Parser)]
pub struct Env {
    // structopt does not let use `u64::MAX`:
    // https://doc.rust-lang.org/std/primitive.u64.html#associatedconstant.MAX
    #[clap(help = "the block gas limit", long, default_value = "18446744073709551615")]
    pub gas_limit: u64,

    #[clap(help = "the chainid opcode value", long, default_value = "1")]
    pub chain_id: u64,

    #[clap(help = "the tx.gasprice value during EVM execution", long, default_value = "0")]
    pub gas_price: u64,

    #[clap(help = "the base fee in a block", long, default_value = "0")]
    pub block_base_fee_per_gas: u64,

    #[clap(
        help = "the tx.origin value during EVM execution",
        long,
        default_value = "0x00a329c0648769A73afAc7F9381E08FB43dBEA72",
        env = "DAPP_TEST_ORIGIN"
    )]
    pub tx_origin: Address,

    #[clap(
        help = "the block.coinbase value during EVM execution",
        long,
        // TODO: It'd be nice if we could use Address::zero() here.
        default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub block_coinbase: Address,
    #[clap(
        help = "the block.timestamp value during EVM execution",
        long,
        default_value = "0",
        env = "DAPP_TEST_TIMESTAMP"
    )]
    pub block_timestamp: u64,

    #[clap(help = "the block.number value during EVM execution", long, default_value = "0")]
    #[clap(env = "DAPP_TEST_NUMBER")]
    pub block_number: u64,

    #[clap(help = "the block.difficulty value during EVM execution", long, default_value = "0")]
    pub block_difficulty: u64,

    #[clap(help = "the block.gaslimit value during EVM execution", long)]
    pub block_gas_limit: Option<u64>,
    // TODO: Add configuration option for base fee.
}

impl Env {
    #[cfg(feature = "sputnik")]
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

    #[cfg(feature = "evmodin")]
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
