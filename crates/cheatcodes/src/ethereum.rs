//! Cheatcode inspection for Ethereum evm2 execution.

use crate::{CheatsConfig, Error, Vm, dispatch};
use alloy_primitives::{Address, B256, Bytes, U256, map::AddressHashSet};
use alloy_sol_types::{SolInterface, SolValue};
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
        if interp.is_static()
            && matches!(
                &decoded,
                Vm::VmCalls::deal(_)
                    | Vm::VmCalls::warp(_)
                    | Vm::VmCalls::allowCheatcodes(_)
                    | Vm::VmCalls::store(_)
                    | Vm::VmCalls::setNonce(_)
                    | Vm::VmCalls::setNonceUnsafe(_)
            )
        {
            return (InstrStop::StateChangeDuringStaticCall, Bytes::new());
        }

        match decoded {
            Vm::VmCalls::assume(call) => {
                if call.condition {
                    (InstrStop::Return, Bytes::new())
                } else {
                    (InstrStop::Revert, Bytes::from_static(MAGIC_ASSUME))
                }
            }
            Vm::VmCalls::deal(call) => {
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
            Vm::VmCalls::warp(call) => {
                let host = interp.host();
                let mut block = *host.block();
                block.timestamp = call.newTimestamp;
                host.set_block(block);
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::allowCheatcodes(call) => {
                self.allow_caller(call.account);
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::getBlockNumber(_) => {
                (InstrStop::Return, interp.host().block().number.abi_encode().into())
            }
            Vm::VmCalls::getBlockTimestamp(_) => {
                (InstrStop::Return, interp.host().block().timestamp.abi_encode().into())
            }
            Vm::VmCalls::getChainId(_) => (
                InstrStop::Return,
                U256::from(interp.host().version().chain_id).abi_encode().into(),
            ),
            Vm::VmCalls::getNonce_0(call) => {
                let nonce = interp
                    .host()
                    .state_mut()
                    .account(&call.account, false)
                    .map(|account| account.nonce());
                match nonce {
                    Ok(nonce) => (InstrStop::Return, nonce.abi_encode().into()),
                    Err(error) => {
                        interp.host().set_error_code(error);
                        (InstrStop::FatalExternalError, Bytes::new())
                    }
                }
            }
            Vm::VmCalls::setNonce(call) => {
                Self::set_nonce(interp, call.account, call.newNonce, true)
            }
            Vm::VmCalls::setNonceUnsafe(call) => {
                Self::set_nonce(interp, call.account, call.newNonce, false)
            }
            Vm::VmCalls::load(call) => {
                let value = interp
                    .host()
                    .state_mut()
                    .storage_slot(&call.target, call.slot.into(), false)
                    .map(|slot| slot.current());
                match value {
                    Ok(value) => {
                        (InstrStop::Return, B256::from(value.to_be_bytes()).abi_encode().into())
                    }
                    Err(error) => {
                        interp.host().set_error_code(error);
                        (InstrStop::FatalExternalError, Bytes::new())
                    }
                }
            }
            Vm::VmCalls::store(call) => {
                if interp.host().precompiles().contains(&call.target) {
                    return (
                        InstrStop::Revert,
                        Error::encode(format!(
                            "cannot use precompile {} as an argument",
                            call.target
                        )),
                    );
                }
                let written = (|| {
                    let state = interp.host().state_mut();
                    state.account(&call.target, false)?;
                    state
                        .storage_slot(&call.target, call.slot.into(), false)
                        .map(|mut slot| slot.set(call.value.into()))
                })();
                match written {
                    Ok(()) => (InstrStop::Return, Bytes::new()),
                    Err(error) => {
                        interp.host().set_error_code(error);
                        (InstrStop::FatalExternalError, Bytes::new())
                    }
                }
            }
            _ => (
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: unsupported in evm2 execution")),
            ),
        }
    }

    fn set_nonce(
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        address: Address,
        nonce: u64,
        checked: bool,
    ) -> (InstrStop, Bytes) {
        let updated = interp.host().state_mut().account(&address, false).map(|mut account| {
            let current = account.nonce();
            if checked && nonce < current {
                Some(current)
            } else {
                account.set_nonce(nonce);
                None
            }
        });
        match updated {
            Ok(Some(current)) => (
                InstrStop::Revert,
                Error::encode(format!(
                    "new nonce ({nonce}) must be strictly equal to or higher than the account's current nonce ({current})"
                )),
            ),
            Ok(None) => (InstrStop::Return, Bytes::new()),
            Err(error) => {
                interp.host().set_error_code(error);
                (InstrStop::FatalExternalError, Bytes::new())
            }
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
