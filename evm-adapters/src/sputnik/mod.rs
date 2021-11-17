mod evm;
pub use evm::*;

mod forked_backend;
pub use forked_backend::*;

pub mod cheatcodes;
pub mod state;

use ethers::{
    providers::Middleware,
    types::{Address, H160, H256, U256},
};

use sputnik::{
    backend::MemoryVicinity,
    executor::stack::{PrecompileFailure, PrecompileOutput, StackExecutor, StackState},
    Config, CreateScheme, ExitError, ExitReason, ExitSucceed,
};

pub use sputnik as sputnik_evm;
use sputnik_evm::executor::stack::PrecompileSet;

pub async fn vicinity<M: Middleware>(
    provider: &M,
    pin_block: Option<u64>,
) -> Result<MemoryVicinity, M::Error> {
    let block_number = if let Some(pin_block) = pin_block {
        pin_block
    } else {
        provider.get_block_number().await?.as_u64()
    };
    let (gas_price, chain_id, block) = tokio::try_join!(
        provider.get_gas_price(),
        provider.get_chainid(),
        provider.get_block(block_number)
    )?;
    let block = block.expect("block not found");

    Ok(MemoryVicinity {
        origin: Default::default(),
        chain_id,
        block_hashes: Vec::new(),
        block_number: block.number.expect("block number not found").as_u64().into(),
        block_coinbase: block.author,
        block_difficulty: block.difficulty,
        block_gas_limit: block.gas_limit,
        block_timestamp: block.timestamp,
        block_base_fee_per_gas: block.base_fee_per_gas.unwrap_or_default(),
        gas_price,
    })
}

/// Abstraction over the StackExecutor used inside of Sputnik, so that we can replace
/// it with one that implements HEVM-style cheatcodes (or other features).
pub trait SputnikExecutor<S> {
    fn config(&self) -> &Config;
    fn state(&self) -> &S;
    fn state_mut(&mut self) -> &mut S;
    fn gas_left(&self) -> U256;
    fn transact_call(
        &mut self,
        caller: H160,
        address: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> (ExitReason, Vec<u8>);

    fn transact_create(
        &mut self,
        caller: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> ExitReason;

    fn create_address(&self, caller: CreateScheme) -> Address;

    /// Returns a vector of string parsed logs that occurred during the previous VM
    /// execution
    fn logs(&self) -> Vec<String>;

    /// Clears all logs in the current EVM instance, so that subsequent calls to
    /// `logs` do not print duplicate logs on shared EVM instances.
    fn clear_logs(&mut self);
}

// The implementation for the base Stack Executor just forwards to the internal methods.
impl<'a, 'b, S: StackState<'a>, P: PrecompileSet> SputnikExecutor<S>
    for StackExecutor<'a, 'b, S, P>
{
    fn config(&self) -> &Config {
        self.config()
    }

    fn state(&self) -> &S {
        self.state()
    }

    fn state_mut(&mut self) -> &mut S {
        self.state_mut()
    }

    fn gas_left(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().gas())
    }

    fn transact_call(
        &mut self,
        caller: H160,
        address: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> (ExitReason, Vec<u8>) {
        self.transact_call(caller, address, value, data, gas_limit, access_list)
    }

    fn transact_create(
        &mut self,
        caller: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> ExitReason {
        self.transact_create(caller, value, data, gas_limit, access_list)
    }

    fn create_address(&self, scheme: CreateScheme) -> Address {
        self.create_address(scheme)
    }

    // Empty impls for non-cheatcode handlers
    fn logs(&self) -> Vec<String> {
        vec![]
    }
    fn clear_logs(&mut self) {}
}

use std::borrow::Cow;

type PrecompileFn =
    fn(&[u8], Option<u64>, &sputnik::Context, bool) -> Result<PrecompileOutput, PrecompileFailure>;

pub static PRECOMPILES: Lazy<revm_precompiles::Precompiles> = Lazy::new(|| {
    // We use the const to immediately choose the latest revision of available
    // precompiles. Is this wrong maybe?
    revm_precompiles::Precompiles::new::<3>()
});

// https://github.com/rust-blockchain/evm-tests/blob/d53b17989db45d76b5876b33db63bcaf367a53fa/jsontests/src/state.rs#L55
// We need to do this because closures can only be coerced to `fn` types if they do not capture any
// variables
macro_rules! precompile_entry {
    ($map:expr, $index:expr) => {
        let x: fn(
            &[u8],
            Option<u64>,
            &Context,
            bool,
        ) -> Result<PrecompileOutput, PrecompileFailure> =
            |input: &[u8], gas_limit: Option<u64>, _context: &Context, _is_static: bool| {
                let precompile = PRECOMPILES.get(&H160::from_low_u64_be($index)).unwrap();
                crate::sputnik::exec(&precompile, input, gas_limit.unwrap())
            };
        $map.insert(H160::from_low_u64_be($index), x);
    };
}

use once_cell::sync::Lazy;
use sputnik::Context;
use std::collections::BTreeMap;
pub static PRECOMPILES_MAP: Lazy<BTreeMap<Address, PrecompileFn>> = Lazy::new(|| {
    let mut map = BTreeMap::new();
    precompile_entry!(map, 1);
    precompile_entry!(map, 2);
    precompile_entry!(map, 3);
    precompile_entry!(map, 4);
    precompile_entry!(map, 5);
    precompile_entry!(map, 6);
    precompile_entry!(map, 7);
    precompile_entry!(map, 8);
    precompile_entry!(map, 9);
    map
});

pub fn exec(
    builtin: &revm_precompiles::Precompile,
    input: &[u8],
    gas_limit: u64,
) -> Result<PrecompileOutput, PrecompileFailure> {
    let res = match builtin {
        revm_precompiles::Precompile::Standard(func) => func(input, gas_limit),
        revm_precompiles::Precompile::Custom(func) => func(input, gas_limit),
    };
    match res {
        Ok(res) => {
            let logs = res
                .logs
                .into_iter()
                .map(|log| sputnik::backend::Log {
                    topics: log.topics,
                    data: log.data.to_vec(),
                    address: log.address,
                })
                .collect();
            Ok(PrecompileOutput {
                exit_status: ExitSucceed::Stopped,
                output: res.output,
                cost: res.cost,
                logs,
            })
        }
        Err(_) => {
            Err(PrecompileFailure::Error { exit_status: ExitError::Other(Cow::Borrowed("error")) })
        }
    }
}
