//! support for writing scripts with solidity

use crate::{
    sputnik::{
        cheatcodes::{
            cheatcode_handler::{CHEATCODE_ADDRESS, CONSOLE_ADDRESS},
            memory_stackstate_owned::MemoryStackStateOwned,
        },
        script::handler::{
            ScriptExecutionHandler, ScriptHandler, ScriptStackExecutor, ScriptStackState,
        },
        Executor,
    },
    Evm,
};
use ethers_core::types::Address;
use once_cell::sync::Lazy;
use sputnik::{
    backend::Backend,
    executor::stack::{PrecompileSet, StackExecutor, StackSubstateMetadata},
    Config,
};

pub mod handler;

/// Address where the forge script vm listens for
// `Address::from_slice(&keccak256("forge sol script")[12..])`
pub static FORGE_SCRIPT_ADDRESS: Lazy<Address> = Lazy::new(|| {
    Address::from_slice(&hex::decode("cc72bd077e2b77a8eee22a99520a6a503a73dc65").unwrap())
});

impl<'a, 'b, B: Backend, P: PrecompileSet + 'b>
    Executor<ScriptStackState<'a, B>, ScriptExecutionHandler<'a, 'b, B, P>>
{
    /// Instantiates a forge script [`Executor`]
    pub fn script_executor(
        backend: B,
        gas_limit: u64,
        config: &'a Config,
        precompiles: &'b P,
        enable_trace: bool,
        debug: bool,
    ) -> Self {
        // create the memory stack state (owned, so that we can modify the backend via
        // self.state_mut on the transact_call fn)
        let metadata = StackSubstateMetadata::new(gas_limit, config);
        let state = MemoryStackStateOwned::new(metadata, backend, enable_trace, debug);

        // create the executor and wrap it with the cheatcode handler
        let stack_executor = StackExecutor::new_with_precompiles(state, config, precompiles);
        let executor = ScriptHandler::new(stack_executor);

        let executor = ScriptExecutionHandler::new(executor);

        let mut evm = Executor::from_executor(executor, gas_limit);

        // Need to create a non-empty contract at the cheat code address so that the EVM backend
        // thinks that something exists there.
        evm.initialize_contracts([
            (*FORGE_SCRIPT_ADDRESS, vec![0u8; 1].into()),
            (*CHEATCODE_ADDRESS, vec![0u8; 1].into()),
            (*CONSOLE_ADDRESS, vec![0u8; 1].into()),
        ]);

        evm
    }
}

#[cfg(any(test, feature = "sputnik-helpers"))]
pub mod helpers {
    use super::*;
    use ethers::types::H160;
    use sputnik::backend::{MemoryBackend, MemoryVicinity};
    use std::collections::BTreeMap;

    use crate::sputnik::{
        cheatcodes::cheatcode_handler::{CheatcodeExecutionHandler, CheatcodeStackState},
        helpers::{new_backend, new_vicinity, VICINITY},
        script::handler::{ScriptStackExecutor, ScriptStackState},
        Executor, PrecompileFn, PRECOMPILES_MAP,
    };
    use once_cell::sync::Lazy;
    use sputnik::Config;

    pub static CFG: Lazy<Config> = Lazy::new(Config::london);

    pub const GAS_LIMIT: u64 = u64::MAX;

    pub type SputnikScriptVM<'a, B> = Executor<
        // state
        ScriptStackState<'a, B>,
        // actual stack executor
        ScriptExecutionHandler<'a, 'a, B, BTreeMap<Address, PrecompileFn>>,
    >;

    /// Instantiates a Sputnik EVM with enabled cheatcodes + FFI and a simple non-forking in memory
    /// backend and tracing disabled
    pub fn script_vm<'a>() -> SputnikScriptVM<'a, MemoryBackend<'a>> {
        let backend = new_backend(&*VICINITY, Default::default());
        Executor::script_executor(backend, GAS_LIMIT, &*CFG, &*PRECOMPILES_MAP, false, false)
    }
}
