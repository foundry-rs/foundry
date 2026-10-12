//! Storage callbacks executed by a distinct owned EVM in the suspended transaction.

use super::Cheatcodes;
use crate::StorageHook;
use alloy_primitives::{Address, Bytes, U256, map::HashMap};
use evm2::{
    Evm, ExecutionConfig, Precompiles,
    ethereum::ethereum_tx_registry,
    evm::{EmptyDB, State},
    interpreter::{Host, InstrStop, Interpreter, Message, op},
};
use foundry_evm_core::{constants::CHEATCODE_ADDRESS, ethereum::FoundryEvmTypes};
use std::mem;

#[derive(Clone, Debug, Default)]
pub(super) struct StorageHooks {
    pub(super) loads: HashMap<Address, StorageHook>,
    pub(super) stores: HashMap<Address, StorageHook>,
    pending: Option<(StorageHook, Address, U256, Option<U256>)>,
    pub(super) active: bool,
    pub(super) failure: Option<(u16, Bytes)>,
}

impl Cheatcodes {
    pub(super) fn capture_storage_hook(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
    ) {
        self.storage_hooks.pending = None;
        if self.storage_hooks.active {
            return;
        }
        let address = interp.message().destination;
        let hook = match interp.opcode() {
            op::SLOAD => self.storage_hooks.loads.get(&address),
            op::SSTORE => self.storage_hooks.stores.get(&address),
            _ => None,
        };
        if let Some(hook) = hook.copied()
            && let Some(slot) = interp.stack().peek(0)
        {
            let old = if interp.opcode() == op::SSTORE {
                match interp.host().state_mut().storage_slot_untracked(&address, &slot) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        let _ = interp.fail(error);
                        return;
                    }
                }
            } else {
                None
            };
            self.storage_hooks.pending = Some((hook, address, slot, old));
        }
    }

    pub(super) fn invoke_storage_hook(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
    ) {
        let Some((hook, address, slot, old)) = self.storage_hooks.pending.take() else { return };
        if interp.result().is_err() {
            return;
        }
        let value = if old.is_some() {
            interp.host().state().get_storage(&address, &slot).unwrap_or_default()
        } else {
            interp.stack().peek(0).expect("successful SLOAD result")
        };
        let mut input = Vec::with_capacity(132);
        input.extend_from_slice(&hook.callback_selector);
        input.extend_from_slice(address.into_word().as_slice());
        input.extend_from_slice(&slot.to_be_bytes::<32>());
        if let Some(old) = old {
            input.extend_from_slice(&old.to_be_bytes::<32>());
        }
        input.extend_from_slice(&value.to_be_bytes::<32>());
        let tx_env = interp.tx_env();
        let depth = interp.message().depth.saturating_add(1);
        let gas_limit = interp.gas().remaining();
        let host = interp.host();
        let source = self.forks.identity();
        let checkpoint = host.state().checkpoint();
        let code =
            match host.state_mut().account(&hook.callback_target).and_then(|mut a| a.load_code()) {
                Ok(code) => code,
                Err(error) => {
                    let _ = interp.fail(error);
                    return;
                }
            };
        let spec = host.spec_id();
        let mut child = Evm::<FoundryEvmTypes>::new_with_execution_config_and_ext(
            ExecutionConfig::for_spec_and_version(spec, *host.version()),
            spec,
            *host.block(),
            ethereum_tx_registry(spec),
            EmptyDB::default(),
            Precompiles::base(spec),
            *host.ext(),
        );
        *child.state_mut() = mem::replace(host.state_mut(), State::new(EmptyDB::default()));
        let expected_revert = self.expected_revert.take();
        let expected_calls = mem::take(&mut self.expected_calls);
        let expected_emits = mem::take(&mut self.expected_emits);
        let mocked_calls = mem::take(&mut self.mocked_calls);
        let mocked_functions = mem::take(&mut self.mocked_functions);
        self.storage_hooks.active = true;
        child.set_inspector(mem::take(self));
        let mut message = Message::<FoundryEvmTypes> {
            depth,
            gas_limit,
            caller: CHEATCODE_ADDRESS,
            destination: hook.callback_target,
            call_target: hook.callback_target,
            code_address: hook.callback_target,
            code,
            input: Bytes::from(input),
            ..Default::default()
        };
        let result = Host::execute_message(&mut child, tx_env, &mut message);
        let inspector = child.clear_inspector().expect("storage callback inspector");
        // SAFETY: The child owns the Self supplied above; Self borrows no host state.
        *self = *unsafe { Box::from_raw(Box::into_raw(inspector).cast::<Self>()) };
        self.storage_hooks.active = false;
        self.expected_revert = expected_revert;
        self.expected_calls = expected_calls;
        self.expected_emits = expected_emits;
        self.mocked_calls = mocked_calls;
        self.mocked_functions = mocked_functions;
        host.set_block(*child.block());
        *host.ext_mut() = *child.ext();
        *host.state_mut() = mem::replace(child.state_mut(), State::new(EmptyDB::default()));
        if self.forks.identity() == source {
            host.state_mut().restore_access_warmth(&checkpoint);
        }
        match result {
            Ok(result) if result.stop.is_success() => {}
            Ok(result) => {
                self.storage_hooks.failure = Some((depth.saturating_sub(1), result.output));
                interp.set_stop(InstrStop::Revert);
            }
            Err(error) => {
                let _ = interp.fail(error);
            }
        }
    }
}
