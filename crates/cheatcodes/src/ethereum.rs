//! Cheatcode inspection for Ethereum evm2 execution.

use crate::{CheatsConfig, Error, Vm, dispatch};
use alloy_primitives::{Address, Bytes, map::AddressHashSet};
use alloy_sol_types::SolInterface;
use evm2::{
    Inspector,
    bytecode::Bytecode,
    evm::{AccountInfo, Database},
    interpreter::{GasTracker, InstrStop, Interpreter, Message, MessageResult, MessageResultExt},
};
use foundry_evm_core::{
    constants::{CHEATCODE_ADDRESS, CHEATCODE_CONTRACT_HASH, MAGIC_ASSUME},
    ethereum::{FoundryEvmTypes, LocalState},
};
use std::sync::Arc;

/// Cheatcode state retained across accepted evm2 transactions.
#[derive(Clone, Debug)]
pub struct EthereumCheatcodes {
    config: Arc<CheatsConfig>,
    access_mode: CheatcodeAccessMode,
    allowed_callers: AddressHashSet,
}

/// Caller authorization policy for the active execution database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheatcodeAccessMode {
    /// Local execution permits cheatcodes from every caller.
    Local,
    /// Forked execution requires an authorized caller.
    Forked,
}

impl EthereumCheatcodes {
    /// Creates the cheatcode inspector for an execution session.
    pub fn new(config: Arc<CheatsConfig>, access_mode: CheatcodeAccessMode) -> Self {
        Self { config, access_mode, allowed_callers: AddressHashSet::default() }
    }

    /// Updates caller authorization when the active database changes.
    pub const fn set_access_mode(&mut self, access_mode: CheatcodeAccessMode) {
        self.access_mode = access_mode;
    }

    /// Grants a contract access to cheatcodes while a fork is active.
    pub fn allow_caller(&mut self, address: Address) -> bool {
        self.allowed_callers.insert(address)
    }

    /// Installs the cheatcode contract account used by Solidity code checks.
    pub fn install<D: Database + Clone>(&self, state: &mut LocalState<D>) {
        state.database_mut().insert_account_info(
            &CHEATCODE_ADDRESS,
            AccountInfo {
                code_hash: CHEATCODE_CONTRACT_HASH,
                code: Some(Bytecode::new_legacy(Bytes::from_static(&[0]))),
                ..Default::default()
            },
        );
    }

    fn apply(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
    ) -> (InstrStop, Bytes) {
        let decoded = match Vm::VmCalls::abi_decode(&message.input) {
            Ok(decoded) => decoded,
            Err(error) => return (InstrStop::Revert, Error::encode(error.to_string())),
        };
        let cheat = dispatch::metadata(&decoded);
        let name = dispatch::name(cheat);
        if self.access_mode == CheatcodeAccessMode::Forked
            && !self.allowed_callers.contains(&message.caller)
        {
            return (
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: cheatcode access denied for {}", message.caller)),
            );
        }
        if self.config.blocked_cheatcodes.contains(&cheat.func.selector_bytes) {
            return (
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: disabled during restricted execution")),
            );
        }

        match decoded {
            Vm::VmCalls::assume(call) => {
                if call.condition {
                    (InstrStop::Return, Bytes::new())
                } else {
                    (InstrStop::Revert, Bytes::from_static(MAGIC_ASSUME))
                }
            }
            Vm::VmCalls::deal(call) if !interp.is_static() => {
                let updated = interp
                    .host()
                    .state_mut()
                    .account(&call.account, false)
                    .map(|mut account| account.set_balance(call.newBalance));
                match updated {
                    Ok(()) => (InstrStop::Return, Bytes::new()),
                    Err(error) => {
                        interp.host().set_error_code(error);
                        (InstrStop::FatalExternalError, Bytes::new())
                    }
                }
            }
            Vm::VmCalls::warp(call) if !interp.is_static() => {
                let host = interp.host();
                let mut block = *host.block();
                block.timestamp = call.newTimestamp;
                host.set_block(block);
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::allowCheatcodes(call) if !interp.is_static() => {
                self.allow_caller(call.account);
                (InstrStop::Return, Bytes::new())
            }
            _ => (
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: unsupported in evm2 execution")),
            ),
        }
    }
}

impl Inspector<FoundryEvmTypes> for EthereumCheatcodes {
    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        if message.call_target != CHEATCODE_ADDRESS {
            return None;
        }

        let (stop, output) = self.apply(interp, message);
        Some(MessageResultExt {
            stop,
            gas: GasTracker::new(message.gas_limit),
            output,
            ..Default::default()
        })
    }
}
