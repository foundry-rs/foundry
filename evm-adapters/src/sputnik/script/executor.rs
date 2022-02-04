//! The executor implementation for forge solidity scripts

use crate::{
    sputnik::{
        cheatcodes::memory_stackstate_owned::MemoryStackStateOwned,
        script::handler::{ScriptHandler, ScriptStackExecutor, ScriptStackState},
        utils::convert_log,
        SputnikExecutor,
    },
    Address, CallTraceArena, DebugArena, U256,
};
use ethers::prelude::{H160, H256};
use ethers_core::abi::{RawLog, Token};
use sputnik::{
    backend::Backend,
    executor::stack::{PrecompileSet, StackExecutor, StackState},
    gasometer, Capture, Config, Context, CreateScheme, ExitReason, ExitRevert, Transfer,
};

/// This is basically a wrapper around the [`ScriptHandler`] which sole purpose is to implement the
/// [`SputnikExecutor`]
pub struct ScriptExecutor<H> {
    handler: ScriptHandler<H>,
}

impl<'a, 'b, B, Precompile> SputnikExecutor<ScriptStackState<'a, B>>
    for ScriptExecutor<ScriptStackExecutor<'a, 'b, B, Precompile>>
where
    B: Backend,
    Precompile: PrecompileSet,
{
    fn config(&self) -> &Config {
        self.handler.handler.config()
    }

    fn state(&self) -> &ScriptStackState<'a, B> {
        self.handler.handler.state()
    }

    fn state_mut(&mut self) -> &mut ScriptStackState<'a, B> {
        self.handler.handler.state_mut()
    }

    fn expected_revert(&self) -> Option<&[u8]> {
        self.handler.handler.state().expected_revert.as_deref()
    }

    fn set_tracing_enabled(&mut self, enabled: bool) -> bool {
        let curr = self.state_mut().trace_enabled;
        self.state_mut().trace_enabled = enabled;
        curr
    }

    fn tracing_enabled(&self) -> bool {
        self.state().trace_enabled
    }

    fn debug_calls(&self) -> Vec<DebugArena> {
        self.state().debug_steps.clone()
    }

    fn all_logs(&self) -> Vec<String> {
        self.handler.handler.state().all_logs.clone()
    }

    fn gas_left(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().gas())
    }

    fn gas_used(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().total_used_gas())
    }

    fn gas_refund(&self) -> U256 {
        U256::from(self.state().metadata().gasometer().refunded_gas())
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
        // reset all_logs because its a new call
        self.state_mut().all_logs = vec![];

        let transaction_cost = gasometer::call_transaction_cost(&data, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return (e.into(), Vec::new()),
        }

        // Initialize initial addresses for EIP-2929
        if self.config().increase_state_access_gas {
            let addresses = core::iter::once(caller).chain(core::iter::once(address));
            self.state_mut().metadata_mut().access_addresses(addresses);

            self.handler.handler.initialize_with_access_list(access_list);
        }

        self.state_mut().inc_nonce(caller);

        let context = Context { caller, address, apparent_value: value };

        match self.call_inner(
            address,
            Some(Transfer { source: caller, target: address, value }),
            data,
            Some(gas_limit),
            false,
            false,
            false,
            context,
        ) {
            Capture::Exit((s, v)) => {
                self.state_mut().increment_call_index();

                // check if all expected calls were made
                if let Some((address, expecteds)) =
                    self.state().expected_calls.iter().find(|(_, expecteds)| !expecteds.is_empty())
                {
                    return (
                        ExitReason::Revert(ExitRevert::Reverted),
                        ethers::abi::encode(&[Token::String(format!(
                            "Expected a call to 0x{} with data {}, but got none",
                            address,
                            ethers::types::Bytes::from(expecteds[0].clone())
                        ))]),
                    )
                }

                if !self.state().expected_emits.is_empty() {
                    return (
                        ExitReason::Revert(ExitRevert::Reverted),
                        ethers::abi::encode(&[Token::String(
                            "Expected an emit, but no logs were emitted afterward".to_string(),
                        )]),
                    )
                }
                (s, v)
            }
            Capture::Trap(_) => {
                self.state_mut().increment_call_index();
                unreachable!()
            }
        }
    }

    fn transact_create(
        &mut self,
        caller: H160,
        value: U256,
        init_code: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> ExitReason {
        // reset all_logs because its a new call
        self.state_mut().all_logs = vec![];

        let transaction_cost = gasometer::create_transaction_cost(&init_code, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return e.into(),
        };
        self.handler.handler.initialize_with_access_list(access_list);

        match self.handler.create_inner(
            caller,
            CreateScheme::Legacy { caller },
            value,
            init_code,
            Some(gas_limit),
            false,
        ) {
            Capture::Exit((s, _, _)) => {
                self.state_mut().increment_call_index();
                s
            }
            Capture::Trap(_) => {
                self.state_mut().increment_call_index();
                unreachable!()
            }
        }
    }

    fn create_address(&self, scheme: CreateScheme) -> Address {
        self.handler.handler.create_address(scheme)
    }

    fn raw_logs(&self) -> Vec<RawLog> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter().map(|log| RawLog { topics: log.topics, data: log.data }).collect()
    }

    fn traces(&self) -> Vec<CallTraceArena> {
        self.state().traces.clone()
    }

    fn reset_traces(&mut self) {
        self.state_mut().reset_traces();
    }

    fn logs(&self) -> Vec<String> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter().filter_map(convert_log).collect()
    }

    fn clear_logs(&mut self) {
        self.state_mut().substate.logs_mut().clear()
    }
}
