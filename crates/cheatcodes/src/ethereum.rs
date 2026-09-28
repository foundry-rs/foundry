//! Cheatcode inspection for Ethereum evm2 execution.

use crate::{CheatsConfig, Error, Vm, dispatch, prank::Prank};
use alloy_primitives::{Address, B256, Bytes, U256, map::AddressHashSet};
use alloy_sol_types::{SolInterface, SolValue};
use evm2::{
    EvmFeatures, Inspector,
    bytecode::Bytecode,
    evm::{AccountInfo, Database},
    interpreter::{
        GasTracker, InstrStop, Interpreter, Message, MessageKind, MessageResult, MessageResultExt,
        derive_create_destination,
    },
};
use foundry_evm_core::{
    constants::{CHEATCODE_ADDRESS, CHEATCODE_CONTRACT_HASH, MAGIC_ASSUME, MAGIC_SKIP},
    ethereum::{FoundryEvmTypes, LocalState},
};
use std::{collections::BTreeMap, sync::Arc};

mod expect;
use expect::ExpectedRevert;

/// Cheatcode state retained across accepted evm2 transactions.
#[derive(Clone, Debug)]
pub struct EthereumCheatcodes {
    config: Arc<CheatsConfig>,
    access_mode: CheatcodeAccessMode,
    allowed_callers: AddressHashSet,
    pranks: BTreeMap<usize, Prank>,
    skip_payloads: Vec<Bytes>,
    expected_revert: Option<ExpectedRevert>,
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
        Self {
            config,
            access_mode,
            allowed_callers: AddressHashSet::default(),
            pranks: BTreeMap::new(),
            skip_payloads: Vec::new(),
            expected_revert: None,
        }
    }

    /// Updates caller authorization when the active database changes.
    pub const fn set_access_mode(&mut self, access_mode: CheatcodeAccessMode) {
        self.access_mode = access_mode;
    }

    /// Grants a contract access to cheatcodes while a fork is active.
    pub fn allow_caller(&mut self, address: Address) -> bool {
        self.allowed_callers.insert(address)
    }

    /// Drains genuine skip payloads recorded during the last execution.
    pub fn take_skip_payloads(&mut self) -> Vec<Bytes> {
        std::mem::take(&mut self.skip_payloads)
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
                    | Vm::VmCalls::coinbase(_)
                    | Vm::VmCalls::difficulty(_)
                    | Vm::VmCalls::prevrandao_0(_)
                    | Vm::VmCalls::prevrandao_1(_)
                    | Vm::VmCalls::fee(_)
                    | Vm::VmCalls::txGasPrice(_)
                    | Vm::VmCalls::allowCheatcodes(_)
                    | Vm::VmCalls::store(_)
                    | Vm::VmCalls::setNonce(_)
                    | Vm::VmCalls::setNonceUnsafe(_)
                    | Vm::VmCalls::prank_0(_)
                    | Vm::VmCalls::prank_1(_)
                    | Vm::VmCalls::prank_2(_)
                    | Vm::VmCalls::prank_3(_)
                    | Vm::VmCalls::startPrank_0(_)
                    | Vm::VmCalls::startPrank_1(_)
                    | Vm::VmCalls::startPrank_2(_)
                    | Vm::VmCalls::startPrank_3(_)
                    | Vm::VmCalls::stopPrank(_)
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
            Vm::VmCalls::skip_0(call) => self.skip(message, call.skipTest, ""),
            Vm::VmCalls::skip_1(call) => self.skip(message, call.skipTest, &call.reason),
            Vm::VmCalls::expectRevert_0(_) => self.expect_revert(message, None, false, None, 1),
            Vm::VmCalls::expectRevert_1(call) => self.expect_revert(
                message,
                Some(Bytes::copy_from_slice(call.revertData.as_ref())),
                false,
                None,
                1,
            ),
            Vm::VmCalls::expectRevert_2(call) => {
                self.expect_revert(message, Some(call.revertData), false, None, 1)
            }
            Vm::VmCalls::expectRevert_3(call) => {
                self.expect_revert(message, None, false, Some(call.reverter), 1)
            }
            Vm::VmCalls::expectRevert_4(call) => self.expect_revert(
                message,
                Some(Bytes::copy_from_slice(call.revertData.as_ref())),
                false,
                Some(call.reverter),
                1,
            ),
            Vm::VmCalls::expectRevert_5(call) => {
                self.expect_revert(message, Some(call.revertData), false, Some(call.reverter), 1)
            }
            Vm::VmCalls::expectRevert_6(call) => {
                self.expect_revert(message, None, false, None, call.count)
            }
            Vm::VmCalls::expectRevert_7(call) => self.expect_revert(
                message,
                Some(Bytes::copy_from_slice(call.revertData.as_ref())),
                false,
                None,
                call.count,
            ),
            Vm::VmCalls::expectRevert_8(call) => {
                self.expect_revert(message, Some(call.revertData), false, None, call.count)
            }
            Vm::VmCalls::expectRevert_9(call) => {
                self.expect_revert(message, None, false, Some(call.reverter), call.count)
            }
            Vm::VmCalls::expectRevert_10(call) => self.expect_revert(
                message,
                Some(Bytes::copy_from_slice(call.revertData.as_ref())),
                false,
                Some(call.reverter),
                call.count,
            ),
            Vm::VmCalls::expectRevert_11(call) => self.expect_revert(
                message,
                Some(call.revertData),
                false,
                Some(call.reverter),
                call.count,
            ),
            Vm::VmCalls::expectPartialRevert_0(call) => self.expect_revert(
                message,
                Some(Bytes::copy_from_slice(call.revertData.as_ref())),
                true,
                None,
                1,
            ),
            Vm::VmCalls::expectPartialRevert_1(call) => self.expect_revert(
                message,
                Some(Bytes::copy_from_slice(call.revertData.as_ref())),
                true,
                Some(call.reverter),
                1,
            ),
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
            Vm::VmCalls::coinbase(call) => {
                let host = interp.host();
                let mut block = *host.block();
                block.beneficiary = call.newCoinbase;
                host.set_block(block);
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::difficulty(call) => {
                if interp.spec() >= evm2::SpecId::MERGE {
                    return (
                        InstrStop::Revert,
                        Error::encode(
                            "`difficulty` is not supported after the Paris hard fork, use `prevrandao` instead; see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399",
                        ),
                    );
                }
                let host = interp.host();
                let mut block = *host.block();
                block.difficulty = call.newDifficulty;
                host.set_block(block);
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::prevrandao_0(call) => Self::set_prevrandao(interp, call.newPrevrandao),
            Vm::VmCalls::prevrandao_1(call) => {
                Self::set_prevrandao(interp, call.newPrevrandao.into())
            }
            Vm::VmCalls::fee(call) => {
                if call.newBasefee > U256::from(u64::MAX) {
                    return (InstrStop::Revert, Error::encode("base fee must be less than 2^64"));
                }
                let host = interp.host();
                let mut block = *host.block();
                block.basefee = call.newBasefee;
                host.set_block(block);
                host.ext_mut().basefee_override = Some(call.newBasefee);
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::txGasPrice(call) => {
                if call.newGasPrice > U256::from(u64::MAX) {
                    return (InstrStop::Revert, Error::encode("gas price must be less than 2^64"));
                }
                interp.host().ext_mut().gas_price_override = Some(call.newGasPrice);
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
            Vm::VmCalls::prank_0(call) => {
                self.start_prank(interp, message, call.msgSender, None, true, false)
            }
            Vm::VmCalls::prank_1(call) => {
                self.start_prank(interp, message, call.msgSender, Some(call.txOrigin), true, false)
            }
            Vm::VmCalls::prank_2(call) => {
                self.start_prank(interp, message, call.msgSender, None, true, call.delegateCall)
            }
            Vm::VmCalls::prank_3(call) => self.start_prank(
                interp,
                message,
                call.msgSender,
                Some(call.txOrigin),
                true,
                call.delegateCall,
            ),
            Vm::VmCalls::startPrank_0(call) => {
                self.start_prank(interp, message, call.msgSender, None, false, false)
            }
            Vm::VmCalls::startPrank_1(call) => {
                self.start_prank(interp, message, call.msgSender, Some(call.txOrigin), false, false)
            }
            Vm::VmCalls::startPrank_2(call) => {
                self.start_prank(interp, message, call.msgSender, None, false, call.delegateCall)
            }
            Vm::VmCalls::startPrank_3(call) => self.start_prank(
                interp,
                message,
                call.msgSender,
                Some(call.txOrigin),
                false,
                call.delegateCall,
            ),
            Vm::VmCalls::stopPrank(_) => {
                self.pranks.remove(&usize::from(message.depth.saturating_sub(1)));
                (InstrStop::Return, Bytes::new())
            }
            _ => (
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: unsupported in evm2 execution")),
            ),
        }
    }

    fn skip(
        &mut self,
        message: &Message<FoundryEvmTypes>,
        skip_test: bool,
        reason: &str,
    ) -> (InstrStop, Bytes) {
        if !skip_test {
            return (InstrStop::Return, Bytes::new());
        }
        if message.depth > 1 {
            return (InstrStop::Revert, Error::encode("`skip` can only be used at test level"));
        }
        let payload = Bytes::from([MAGIC_SKIP, reason.as_bytes()].concat());
        self.skip_payloads.push(payload.clone());
        (InstrStop::Revert, payload)
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

    fn set_prevrandao(
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        value: B256,
    ) -> (InstrStop, Bytes) {
        if interp.spec() < evm2::SpecId::MERGE {
            return (
                InstrStop::Revert,
                Error::encode(
                    "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399",
                ),
            );
        }
        let host = interp.host();
        let mut block = *host.block();
        block.prevrandao = U256::from_be_slice(value.as_slice());
        host.set_block(block);
        (InstrStop::Return, Bytes::new())
    }

    fn start_prank(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        new_caller: Address,
        new_origin: Option<Address>,
        single_call: bool,
        delegate_call: bool,
    ) -> (InstrStop, Bytes) {
        if message.depth == 0 {
            return (
                InstrStop::Revert,
                Error::encode("top-level prank is unsupported in evm2 execution"),
            );
        }
        let depth = usize::from(message.depth.saturating_sub(1));
        if let Some(prank) = self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank) {
            if !prank.used {
                return (
                    InstrStop::Revert,
                    Error::encode("cannot overwrite a prank until it is applied at least once"),
                );
            }
            if single_call != prank.single_call {
                return (
                    InstrStop::Revert,
                    Error::encode(
                        "cannot override an ongoing prank with a single vm.prank; use vm.startPrank to override the current prank",
                    ),
                );
            }
        }
        let loaded =
            interp.host().state_mut().account(&new_caller, false).and_then(|mut account| {
                account.touch();
                if delegate_call {
                    account.load_code().map(|code| !code.is_empty())
                } else {
                    Ok(true)
                }
            });
        match loaded {
            Ok(true) => {}
            Ok(false) => {
                return (
                    InstrStop::Revert,
                    Error::encode("cannot `prank` delegate call from an EOA"),
                );
            }
            Err(error) => {
                interp.host().set_error_code(error);
                return (InstrStop::FatalExternalError, Bytes::new());
            }
        }
        let context = interp.host().ext();
        let origin =
            context.origin_override.or(context.transaction_origin).unwrap_or(message.caller);
        self.pranks.insert(
            depth,
            Prank::new(
                message.caller,
                origin,
                new_caller,
                new_origin,
                depth,
                single_call,
                delegate_call,
            ),
        );
        (InstrStop::Return, Bytes::new())
    }

    fn apply_prank(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) {
        if message.depth == 0 {
            return;
        }
        let depth = usize::from(message.depth.saturating_sub(1));
        let Some(prank) = self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank) else {
            return;
        };
        let delegate_call = prank.delegate_call
            && depth == prank.depth
            && message.kind == MessageKind::DelegateCall;
        if !delegate_call && message.caller != prank.prank_caller {
            return;
        }
        if delegate_call {
            message.destination = prank.new_caller;
            message.caller = prank.new_caller;
        } else if depth == prank.depth {
            message.caller = prank.new_caller;
        }
        if let Some(origin) = prank.new_origin {
            interp.host().ext_mut().origin_override = Some(origin);
        }
        if (depth == prank.depth || prank.new_origin.is_some())
            && let Some(applied) = prank.first_time_applied()
        {
            self.pranks.insert(prank.depth, applied);
        }
    }

    fn finish_prank(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
    ) {
        if message.depth == 0 {
            return;
        }
        let depth = usize::from(message.depth.saturating_sub(1));
        let Some(prank) = self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank) else {
            return;
        };
        if depth != prank.depth {
            return;
        }
        interp.host().ext_mut().origin_override = Some(prank.prank_origin);
        if prank.single_call {
            self.pranks.remove(&depth);
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
            self.apply_prank(interp, message);
            self.observe_revert_depth(message);
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

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        if message.call_target != CHEATCODE_ADDRESS {
            self.finish_prank(interp, message);
            self.finish_expected_revert(message, result, false);
        }
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        self.observe_revert_depth(message);
        if message.depth == 0 {
            return None;
        }
        let depth = usize::from(message.depth.saturating_sub(1));
        let prank = self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank)?;
        if message.caller != prank.prank_caller {
            return None;
        }
        if depth == prank.depth && interp.host().feature(EvmFeatures::EIP8037) {
            // The parent opcode already charged state gas for the original destination.
            return Some(MessageResultExt {
                stop: InstrStop::Revert,
                gas: GasTracker::new(message.gas_limit),
                output: Error::encode(
                    "prank before CREATE with EIP-8037 is unsupported in evm2 execution",
                ),
                ..Default::default()
            });
        }
        if depth == prank.depth {
            let nonce =
                interp.host().state_mut().account(&prank.new_caller, false).map(|a| a.nonce());
            let nonce = match nonce {
                Ok(nonce) => nonce,
                Err(error) => {
                    interp.host().set_error_code(error);
                    return Some(MessageResultExt {
                        stop: InstrStop::FatalExternalError,
                        gas: GasTracker::new(message.gas_limit),
                        ..Default::default()
                    });
                }
            };
            message.caller = prank.new_caller;
            message.destination = derive_create_destination(
                message.kind,
                &message.caller,
                &message.salt,
                &message.input,
                nonce,
            );
            message.call_target = message.destination;
        }
        if let Some(origin) = prank.new_origin {
            interp.host().ext_mut().origin_override = Some(origin);
        }
        if (depth == prank.depth || prank.new_origin.is_some())
            && let Some(applied) = prank.first_time_applied()
        {
            self.pranks.insert(prank.depth, applied);
        }
        None
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        self.finish_prank(interp, message);
        self.finish_expected_revert(message, result, true);
    }
}
