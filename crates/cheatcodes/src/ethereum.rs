//! Cheatcode inspection for Ethereum evm2 execution.

use crate::{
    CheatsConfig, Error, Vm,
    broadcast::{Broadcast, BroadcastableTransaction, BroadcastableTransactions},
    dispatch,
    expected_call::{self, ExpectedCallKind, ExpectedCallTracker, ExpectedCallType},
    expected_emit::{self, EmitMismatch, EmitValidation, ExpectedEmitTracker},
    fs::{ConfigCheatcode, get_artifact_code, get_artifact_selectors},
    prank::Prank,
    recorded_logs,
    script::Wallets,
};
use alloy_network::{Ethereum, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, keccak256, map::AddressHashSet};
use alloy_sol_types::{SolCall, SolInterface, SolValue};
use evm2::{
    EvmFeatures, Inspector,
    bytecode::Bytecode,
    evm::{AccountInfo, Database, inspector::CallAction},
    interpreter::{
        GasTracker, InstrStop, Interpreter, Message, MessageKind, MessageResult, MessageResultExt,
        derive_create_destination,
    },
};
use foundry_common::TransactionMaybeSigned;
use foundry_evm_core::{
    constants::{
        CALLER, CHEATCODE_ADDRESS, CHEATCODE_CONTRACT_HASH, MAGIC_ASSUME, MAGIC_SKIP,
        TEST_CONTRACT_ADDRESS,
    },
    eip2935::{HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE},
    ethereum::{FoundryEvmTypes, LocalState},
    evm::{EthEvmNetwork, TransactionRequestFor},
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
    expected_calls: ExpectedCallTracker,
    expected_emits: ExpectedEmitTracker,
    log_error: Option<Bytes>,
    recorded_logs: Option<Vec<Vm::Log>>,
    broadcast: Option<Broadcast>,
    broadcastable_transactions: BroadcastableTransactions<Ethereum>,
    wallets: Option<Wallets>,
    skip_payloads: Vec<Bytes>,
    expected_revert: Option<ExpectedRevert>,
}

type ExpectedCallArgs<'a> = (
    Address,
    &'a Bytes,
    Option<U256>,
    Option<u64>,
    Option<u64>,
    Option<ExpectedCallKind>,
    u64,
    ExpectedCallType,
);

type ExpectedEmitArgs = ([bool; 5], Option<Address>, bool, u64);

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
            allowed_callers: [CHEATCODE_ADDRESS, TEST_CONTRACT_ADDRESS, CALLER]
                .into_iter()
                .collect(),
            pranks: BTreeMap::new(),
            expected_calls: Default::default(),
            expected_emits: Default::default(),
            log_error: None,
            recorded_logs: None,
            broadcast: None,
            broadcastable_transactions: Default::default(),
            wallets: None,
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

    /// Supplies script signers for broadcast sender selection.
    pub fn set_wallets(&mut self, wallets: Wallets) {
        self.wallets = Some(wallets);
    }

    /// Drains transactions collected during script execution.
    pub fn take_broadcastable_transactions(&mut self) -> BroadcastableTransactions<Ethereum> {
        std::mem::take(&mut self.broadcastable_transactions)
    }

    /// Observes an EVM log for active cheatcode recording.
    pub fn observe_log(&mut self, log: &alloy_primitives::Log) {
        recorded_logs::record(&mut self.recorded_logs, log);
        if !self.expected_emits.is_empty()
            && let Some(error) = expected_emit::observe(&mut self.expected_emits, log)
        {
            self.log_error = Some(Error::encode(error));
        }
    }

    /// Stops the frame after an immediately invalid log expectation.
    pub const fn finish_log_step(&self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        if self.log_error.is_some() {
            interp.set_stop(InstrStop::Revert);
        }
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
        decoded: Vm::VmCalls,
    ) -> (InstrStop, Bytes) {
        let name = dispatch::name(dispatch::metadata(&decoded));
        if let Some((target, calldata, value, gas, min_gas, kind, count, call_type)) =
            Self::expected_call_args(&decoded)
        {
            return Self::encoded_result(
                expected_call::expect_call(
                    &mut self.expected_calls,
                    target,
                    calldata.clone(),
                    value,
                    gas,
                    min_gas,
                    kind,
                    count,
                    call_type,
                )
                .map(|()| Vec::new()),
            );
        }
        if let Some((checks, emitter, anonymous, count)) = Self::expected_emit_args(&decoded) {
            expected_emit::register(
                &mut self.expected_emits,
                usize::from(message.depth.saturating_sub(1)),
                checks,
                emitter,
                anonymous,
                count,
            );
            return (InstrStop::Return, Bytes::new());
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
            Vm::VmCalls::etch(call) => Self::etch(interp, call.target, call.newRuntimeBytecode),
            Vm::VmCalls::getCode(call) => {
                Self::artifact_result(get_artifact_code(&self.config, &call.artifactPath, false))
            }
            Vm::VmCalls::getDeployedCode(call) => {
                Self::artifact_result(get_artifact_code(&self.config, &call.artifactPath, true))
            }
            Vm::VmCalls::getSelectors(call) => {
                Self::artifact_result(get_artifact_selectors(&self.config, &call.artifactPath))
            }
            Vm::VmCalls::exists(call) => Self::encoded_result(call.apply_config(&self.config)),
            Vm::VmCalls::isDir(call) => Self::encoded_result(call.apply_config(&self.config)),
            Vm::VmCalls::isFile(call) => Self::encoded_result(call.apply_config(&self.config)),
            Vm::VmCalls::projectRoot(call) => Self::encoded_result(call.apply_config(&self.config)),
            Vm::VmCalls::currentFilePath(call) => {
                Self::encoded_result(call.apply_config(&self.config))
            }
            Vm::VmCalls::unixTime(call) => Self::encoded_result(call.apply_config(&self.config)),
            Vm::VmCalls::readFile(call) => Self::encoded_result(call.apply_config(&self.config)),
            Vm::VmCalls::readFileBinary(call) => {
                Self::encoded_result(call.apply_config(&self.config))
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
            Vm::VmCalls::broadcast_0(_) => self.start_broadcast(interp, message, None, true),
            Vm::VmCalls::broadcast_1(call) => {
                self.start_broadcast(interp, message, Some(call.signer), true)
            }
            Vm::VmCalls::broadcast_2(call) => {
                self.start_broadcast_key(interp, message, &call.privateKey, true)
            }
            Vm::VmCalls::startBroadcast_0(_) => self.start_broadcast(interp, message, None, false),
            Vm::VmCalls::startBroadcast_1(call) => {
                self.start_broadcast(interp, message, Some(call.signer), false)
            }
            Vm::VmCalls::startBroadcast_2(call) => {
                self.start_broadcast_key(interp, message, &call.privateKey, false)
            }
            Vm::VmCalls::stopBroadcast(_) => {
                if self.broadcast.take().is_some() {
                    (InstrStop::Return, Bytes::new())
                } else {
                    (InstrStop::Revert, Error::encode("no broadcast in progress to stop"))
                }
            }
            Vm::VmCalls::recordLogs(_) => {
                self.recorded_logs = Some(Vec::new());
                (InstrStop::Return, Bytes::new())
            }
            Vm::VmCalls::getRecordedLogs(_) => (
                InstrStop::Return,
                recorded_logs::take(&mut self.recorded_logs).abi_encode().into(),
            ),
            Vm::VmCalls::getRecordedLogsJson(_) => {
                Self::encoded_result(recorded_logs::take_json(&mut self.recorded_logs))
            }
            _ => (
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: unsupported in evm2 execution")),
            ),
        }
    }

    fn deployment_args(
        decoded: &Vm::VmCalls,
    ) -> Option<(&str, Option<&Bytes>, U256, Option<B256>)> {
        match decoded {
            Vm::VmCalls::deployCode_0(call) => Some((&call.artifactPath, None, U256::ZERO, None)),
            Vm::VmCalls::deployCode_1(call) => {
                Some((&call.artifactPath, Some(&call.constructorArgs), U256::ZERO, None))
            }
            Vm::VmCalls::deployCode_2(call) => Some((&call.artifactPath, None, call.value, None)),
            Vm::VmCalls::deployCode_3(call) => {
                Some((&call.artifactPath, Some(&call.constructorArgs), call.value, None))
            }
            Vm::VmCalls::deployCode_4(call) => {
                Some((&call.artifactPath, None, U256::ZERO, Some(call.salt)))
            }
            Vm::VmCalls::deployCode_5(call) => {
                Some((&call.artifactPath, Some(&call.constructorArgs), U256::ZERO, Some(call.salt)))
            }
            Vm::VmCalls::deployCode_6(call) => {
                Some((&call.artifactPath, None, call.value, Some(call.salt)))
            }
            Vm::VmCalls::deployCode_7(call) => {
                Some((&call.artifactPath, Some(&call.constructorArgs), call.value, Some(call.salt)))
            }
            _ => None,
        }
    }

    const fn expected_call_args(decoded: &Vm::VmCalls) -> Option<ExpectedCallArgs<'_>> {
        match decoded {
            Vm::VmCalls::expectCall_0(call) => Some((
                call.callee,
                &call.data,
                None,
                None,
                None,
                None,
                1,
                ExpectedCallType::NonCount,
            )),
            Vm::VmCalls::expectCall_1(call) => Some((
                call.callee,
                &call.data,
                None,
                None,
                None,
                None,
                call.count,
                ExpectedCallType::Count,
            )),
            Vm::VmCalls::expectCall_2(call) => Some((
                call.callee,
                &call.data,
                Some(call.msgValue),
                None,
                None,
                None,
                1,
                ExpectedCallType::NonCount,
            )),
            Vm::VmCalls::expectCall_3(call) => Some((
                call.callee,
                &call.data,
                Some(call.msgValue),
                None,
                None,
                None,
                call.count,
                ExpectedCallType::Count,
            )),
            Vm::VmCalls::expectCall_4(call) => Some((
                call.callee,
                &call.data,
                Some(call.msgValue),
                Some(call.gas),
                None,
                None,
                1,
                ExpectedCallType::NonCount,
            )),
            Vm::VmCalls::expectCall_5(call) => Some((
                call.callee,
                &call.data,
                Some(call.msgValue),
                Some(call.gas),
                None,
                None,
                call.count,
                ExpectedCallType::Count,
            )),
            Vm::VmCalls::expectCallMinGas_0(call) => Some((
                call.callee,
                &call.data,
                Some(call.msgValue),
                None,
                Some(call.minGas),
                None,
                1,
                ExpectedCallType::NonCount,
            )),
            Vm::VmCalls::expectCallMinGas_1(call) => Some((
                call.callee,
                &call.data,
                Some(call.msgValue),
                None,
                Some(call.minGas),
                None,
                call.count,
                ExpectedCallType::Count,
            )),
            Vm::VmCalls::expectDelegateCall(call) => Some((
                call.callee,
                &call.data,
                None,
                None,
                None,
                Some(ExpectedCallKind::DelegateCall),
                1,
                ExpectedCallType::NonCount,
            )),
            _ => None,
        }
    }

    const fn expected_emit_args(decoded: &Vm::VmCalls) -> Option<ExpectedEmitArgs> {
        match decoded {
            Vm::VmCalls::expectEmit_0(call) => Some((
                [true, call.checkTopic1, call.checkTopic2, call.checkTopic3, call.checkData],
                None,
                false,
                1,
            )),
            Vm::VmCalls::expectEmit_1(call) => Some((
                [true, call.checkTopic1, call.checkTopic2, call.checkTopic3, call.checkData],
                Some(call.emitter),
                false,
                1,
            )),
            Vm::VmCalls::expectEmit_2(_) => Some(([true; 5], None, false, 1)),
            Vm::VmCalls::expectEmit_3(call) => Some(([true; 5], Some(call.emitter), false, 1)),
            Vm::VmCalls::expectEmit_4(call) => Some((
                [true, call.checkTopic1, call.checkTopic2, call.checkTopic3, call.checkData],
                None,
                false,
                call.count,
            )),
            Vm::VmCalls::expectEmit_5(call) => Some((
                [true, call.checkTopic1, call.checkTopic2, call.checkTopic3, call.checkData],
                Some(call.emitter),
                false,
                call.count,
            )),
            Vm::VmCalls::expectEmit_6(call) => Some(([true; 5], None, false, call.count)),
            Vm::VmCalls::expectEmit_7(call) => {
                Some(([true; 5], Some(call.emitter), false, call.count))
            }
            Vm::VmCalls::expectEmitAnonymous_0(call) => Some((
                [
                    call.checkTopic0,
                    call.checkTopic1,
                    call.checkTopic2,
                    call.checkTopic3,
                    call.checkData,
                ],
                None,
                true,
                1,
            )),
            Vm::VmCalls::expectEmitAnonymous_1(call) => Some((
                [
                    call.checkTopic0,
                    call.checkTopic1,
                    call.checkTopic2,
                    call.checkTopic3,
                    call.checkData,
                ],
                Some(call.emitter),
                true,
                1,
            )),
            Vm::VmCalls::expectEmitAnonymous_2(_) => Some(([true; 5], None, true, 1)),
            Vm::VmCalls::expectEmitAnonymous_3(call) => {
                Some(([true; 5], Some(call.emitter), true, 1))
            }
            _ => None,
        }
    }

    fn is_deployment_call(input: &[u8]) -> bool {
        input.get(..4).is_some_and(|selector| {
            [
                Vm::deployCode_0Call::SELECTOR,
                Vm::deployCode_1Call::SELECTOR,
                Vm::deployCode_2Call::SELECTOR,
                Vm::deployCode_3Call::SELECTOR,
                Vm::deployCode_4Call::SELECTOR,
                Vm::deployCode_5Call::SELECTOR,
                Vm::deployCode_6Call::SELECTOR,
                Vm::deployCode_7Call::SELECTOR,
            ]
            .iter()
            .any(|candidate| candidate.as_slice() == selector)
        })
    }

    fn deploy_code(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        path: &str,
        args: Option<&Bytes>,
        value: U256,
        salt: Option<B256>,
    ) -> CallAction<FoundryEvmTypes> {
        let mut init_code = match get_artifact_code(&self.config, path, false) {
            Ok(code) => code.to_vec(),
            Err(error) => {
                return CallAction::Override(Self::result(
                    message,
                    InstrStop::Revert,
                    Error::encode(error.to_string()),
                ));
            }
        };
        if let Some(args) = args {
            init_code.extend_from_slice(args);
        }
        let init_code = Bytes::from(init_code);
        let nonce = match interp.host().state_mut().account_info_untracked(&message.caller) {
            Ok(account) => account.map_or(0, |account| account.nonce),
            Err(error) => {
                interp.host().set_error_code(error);
                return CallAction::Override(Self::result(
                    message,
                    InstrStop::FatalExternalError,
                    Bytes::new(),
                ));
            }
        };
        let kind = if salt.is_some() { MessageKind::Create2 } else { MessageKind::Create };
        let salt = salt.unwrap_or_default();
        let destination =
            derive_create_destination(kind, &message.caller, &salt, &init_code, nonce);
        CallAction::Execute(Box::new(Message::<FoundryEvmTypes> {
            kind,
            depth: message.depth,
            gas_limit: message.gas_limit,
            reservoir: message.reservoir,
            destination,
            call_target: destination,
            caller: message.caller,
            input: init_code.clone(),
            value,
            code: Bytecode::new_legacy(init_code),
            code_address: destination,
            salt,
            ..Default::default()
        }))
    }

    fn result(
        message: &Message<FoundryEvmTypes>,
        stop: InstrStop,
        output: Bytes,
    ) -> MessageResult<FoundryEvmTypes> {
        MessageResultExt {
            stop,
            gas: GasTracker::new(message.gas_limit),
            output,
            ..Default::default()
        }
    }

    fn etch(
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        target: Address,
        code: Bytes,
    ) -> (InstrStop, Bytes) {
        if interp.host().precompiles().contains(&target) {
            return (
                InstrStop::Revert,
                Error::encode(format!("cannot use precompile {target} as an argument")),
            );
        }
        let code = match Bytecode::new_raw_checked(code) {
            Ok(code) => code,
            Err(error) => {
                return (
                    InstrStop::Revert,
                    Error::encode(format!("failed to create bytecode: {error}")),
                );
            }
        };
        let updated = (|| {
            let state = interp.host().state_mut();
            let old_hash = state.account(&target, false)?.code_hash();
            if target == HISTORY_STORAGE_ADDRESS
                && old_hash == keccak256(&HISTORY_STORAGE_CODE)
                && code.hash_slow() != old_hash
            {
                state.storage(&target).wipe_journaled();
            }
            state.account(&target, false).map(|mut account| account.set_code_slow(code))
        })();
        match updated {
            Ok(()) => (InstrStop::Return, Bytes::new()),
            Err(error) => {
                interp.host().set_error_code(error);
                (InstrStop::FatalExternalError, Bytes::new())
            }
        }
    }

    fn artifact_result<T: SolValue>(result: crate::Result<T>) -> (InstrStop, Bytes) {
        match result {
            Ok(value) => (InstrStop::Return, value.abi_encode().into()),
            Err(error) => (InstrStop::Revert, Error::encode(error.to_string())),
        }
    }

    fn encoded_result(result: crate::Result) -> (InstrStop, Bytes) {
        match result {
            Ok(value) => (InstrStop::Return, value.into()),
            Err(error) => (InstrStop::Revert, Error::encode(error.to_string())),
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

    fn start_broadcast(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        explicit_origin: Option<Address>,
        single_call: bool,
    ) -> (InstrStop, Bytes) {
        if message.depth == 0 {
            return (InstrStop::Revert, Error::encode("top-level broadcast is unsupported"));
        }
        let depth = usize::from(message.depth - 1);
        if self.pranks.range(..=depth).next_back().is_some() {
            return (
                InstrStop::Revert,
                Error::encode(
                    "you have an active prank; broadcasting and pranks are not compatible",
                ),
            );
        }
        if self.broadcast.is_some() {
            return (InstrStop::Revert, Error::encode("a broadcast is active already"));
        }

        let mut origin = explicit_origin;
        if origin.is_none()
            && let Some(wallets) = &self.wallets
        {
            let mut wallets = wallets.inner.lock();
            if let Some(provided_sender) = wallets.provided_sender {
                origin = Some(provided_sender);
            } else {
                match wallets.multi_wallet.signers() {
                    Ok(signers) if signers.len() == 1 => {
                        origin = signers.keys().next().copied();
                    }
                    Ok(_) => {}
                    Err(error) => return (InstrStop::Revert, Error::encode(error.to_string())),
                }
            }
        }
        let context = interp.host().ext();
        let original_origin = context
            .origin_override
            .or(context.transaction_origin)
            .unwrap_or(self.config.evm_opts.sender);
        let new_origin = origin.unwrap_or(original_origin);
        let loaded = interp.host().state_mut().account(&new_origin, false).map(|mut account| {
            account.touch();
        });
        if let Err(error) = loaded {
            interp.host().set_error_code(error);
            return (InstrStop::FatalExternalError, Bytes::new());
        }
        self.broadcast = Some(Broadcast {
            new_origin,
            original_caller: message.caller,
            original_origin,
            depth,
            single_call,
            deploy_from_code: false,
        });
        (InstrStop::Return, Bytes::new())
    }

    fn start_broadcast_key(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        private_key: &U256,
        single_call: bool,
    ) -> (InstrStop, Bytes) {
        let wallet = match crate::crypto::parse_wallet(private_key) {
            Ok(wallet) => wallet,
            Err(error) => return (InstrStop::Revert, Error::encode(error.to_string())),
        };
        let result = self.start_broadcast(interp, message, Some(wallet.address()), single_call);
        if result.0 == InstrStop::Return
            && let Some(wallets) = &self.wallets
        {
            wallets.add_local_signer(wallet);
        }
        result
    }

    fn apply_broadcast_call(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<(InstrStop, Bytes)> {
        let broadcast = self.broadcast.as_ref()?;
        if message.depth == 0
            || usize::from(message.depth - 1) != broadcast.depth
            || message.caller != broadcast.original_caller
        {
            return None;
        }
        if message.kind == MessageKind::StaticCall || message.caller_is_static {
            if broadcast.single_call {
                return Some((
                    InstrStop::Revert,
                    Error::encode(
                        "`staticcall`s are not allowed after `broadcast`; use `startBroadcast` instead",
                    ),
                ));
            }
            return None;
        }
        if message.kind != MessageKind::Call {
            return Some((
                InstrStop::Revert,
                Error::encode("broadcasting this call kind is not yet supported in evm2 execution"),
            ));
        }

        let origin = broadcast.new_origin;
        let nonce = interp.host().state_mut().account(&origin, false).map(|mut account| {
            let nonce = account.nonce();
            nonce.checked_add(1).map(|next_nonce| {
                account.set_nonce(next_nonce);
                nonce
            })
        });
        let nonce = match nonce {
            Ok(Some(nonce)) => nonce,
            Ok(None) => {
                return Some((InstrStop::Revert, Error::encode("broadcast nonce overflow")));
            }
            Err(error) => {
                interp.host().set_error_code(error);
                return Some((InstrStop::FatalExternalError, Bytes::new()));
            }
        };
        let transaction = TransactionRequestFor::<EthEvmNetwork>::default()
            .with_from(origin)
            .with_to(message.call_target)
            .with_value(message.value)
            .with_input(message.input.clone())
            .with_nonce(nonce)
            .with_chain_id(interp.host().version().chain_id);
        self.broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc: self.config.evm_opts.fork_url.clone(),
            transaction: TransactionMaybeSigned::new(transaction),
        });
        message.caller = origin;
        interp.host().ext_mut().origin_override = Some(origin);
        None
    }

    fn apply_broadcast_create(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        let broadcast = self.broadcast.as_ref()?;
        if message.depth == 0
            || usize::from(message.depth - 1) != broadcast.depth
            || message.caller != broadcast.original_caller
        {
            return None;
        }
        if message.kind != MessageKind::Create {
            return Some(Self::result(
                message,
                InstrStop::Revert,
                Error::encode("broadcast CREATE2 is not yet supported in evm2 execution"),
            ));
        }
        if interp.host().feature(EvmFeatures::EIP8037) {
            return Some(Self::result(
                message,
                InstrStop::Revert,
                Error::encode("broadcast CREATE with EIP-8037 is unsupported in evm2 execution"),
            ));
        }

        let origin = broadcast.new_origin;
        let nonce = interp.host().state_mut().account(&origin, false).map(|a| a.nonce());
        let nonce = match nonce {
            Ok(nonce) => nonce,
            Err(error) => {
                interp.host().set_error_code(error);
                return Some(Self::result(message, InstrStop::FatalExternalError, Bytes::new()));
            }
        };
        let transaction = TransactionRequestFor::<EthEvmNetwork>::default()
            .with_from(origin)
            .with_kind(TxKind::Create)
            .with_value(message.value)
            .with_input(message.input.clone())
            .with_nonce(nonce)
            .with_chain_id(interp.host().version().chain_id);
        self.broadcastable_transactions.push_back(BroadcastableTransaction {
            rpc: self.config.evm_opts.fork_url.clone(),
            transaction: TransactionMaybeSigned::new(transaction),
        });
        message.caller = origin;
        message.destination =
            derive_create_destination(message.kind, &origin, &message.salt, &message.input, nonce);
        message.call_target = message.destination;
        interp.host().ext_mut().origin_override = Some(origin);
        None
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

    fn finish_log_error(&mut self, result: &mut MessageResult<FoundryEvmTypes>) {
        if let Some(error) = self.log_error.take() {
            result.stop = InstrStop::Revert;
            result.output = error;
        }
    }

    fn finish_expected_emit(
        &mut self,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        if !result.is_success() {
            return;
        }
        let failure = expected_emit::validate(
            &mut self.expected_emits,
            usize::from(message.depth),
            message.kind == MessageKind::StaticCall || message.caller_is_static,
        );
        let Some(failure) = failure else { return };
        result.stop = InstrStop::Revert;
        result.output = match failure {
            EmitValidation::Missing(expected) => {
                let message = match expected.mismatch_error {
                    Some(EmitMismatch::Log { actual }) => expected.log.as_ref().map_or_else(
                        || "log != expected log".to_string(),
                        |log| {
                            expected_emit::get_emit_mismatch_message(
                                expected.checks,
                                log,
                                &actual,
                                expected.anonymous,
                                None,
                                None,
                            )
                        },
                    ),
                    Some(EmitMismatch::Emitter { expected, actual }) => {
                        format!("log emitter mismatch: expected={expected:#x}, got={actual:#x}")
                    }
                    None => "log != expected log".to_string(),
                };
                message.abi_encode().into()
            }
            EmitValidation::WrongCount { expected, actual } => {
                Error::encode(format!("log emitted {actual} times, expected {expected}"))
            }
        };
    }
}

impl Inspector<FoundryEvmTypes> for EthereumCheatcodes {
    fn call_action(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> CallAction<FoundryEvmTypes> {
        if message.call_target != CHEATCODE_ADDRESS {
            let kind = match message.kind {
                MessageKind::Call => ExpectedCallKind::Call,
                MessageKind::CallCode => ExpectedCallKind::CallCode,
                MessageKind::DelegateCall => ExpectedCallKind::DelegateCall,
                MessageKind::StaticCall => ExpectedCallKind::StaticCall,
                _ => unreachable!("CREATE messages use the create hook"),
            };
            expected_call::observe_call(
                &mut self.expected_calls,
                message.code_address,
                &message.input,
                matches!(message.kind, MessageKind::Call | MessageKind::CallCode)
                    .then_some(message.value),
                message.gas_limit,
                kind,
            );
            if let Some((stop, output)) = self.apply_broadcast_call(interp, message) {
                return CallAction::Override(Self::result(message, stop, output));
            }
            self.apply_prank(interp, message);
            self.observe_revert_depth(message);
            return CallAction::Continue;
        }

        let decoded = match Vm::VmCalls::abi_decode(&message.input) {
            Ok(decoded) => decoded,
            Err(error) => {
                return CallAction::Override(Self::result(
                    message,
                    InstrStop::Revert,
                    Error::encode(error.to_string()),
                ));
            }
        };
        let cheat = dispatch::metadata(&decoded);
        let name = dispatch::name(cheat);
        let failure = if self.access_mode == CheatcodeAccessMode::Forked
            && !self.allowed_callers.contains(&message.caller)
        {
            Some((
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: cheatcode access denied for {}", message.caller)),
            ))
        } else if self.config.blocked_cheatcodes.contains(&cheat.func.selector_bytes) {
            Some((
                InstrStop::Revert,
                Error::encode(format!("vm.{name}: disabled during restricted execution")),
            ))
        } else if interp.is_static()
            && matches!(
                &decoded,
                Vm::VmCalls::deal(_)
                    | Vm::VmCalls::etch(_)
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
                    | Vm::VmCalls::broadcast_0(_)
                    | Vm::VmCalls::broadcast_1(_)
                    | Vm::VmCalls::broadcast_2(_)
                    | Vm::VmCalls::startBroadcast_0(_)
                    | Vm::VmCalls::startBroadcast_1(_)
                    | Vm::VmCalls::startBroadcast_2(_)
                    | Vm::VmCalls::stopBroadcast(_)
                    | Vm::VmCalls::deployCode_0(_)
                    | Vm::VmCalls::deployCode_1(_)
                    | Vm::VmCalls::deployCode_2(_)
                    | Vm::VmCalls::deployCode_3(_)
                    | Vm::VmCalls::deployCode_4(_)
                    | Vm::VmCalls::deployCode_5(_)
                    | Vm::VmCalls::deployCode_6(_)
                    | Vm::VmCalls::deployCode_7(_)
            )
        {
            Some((InstrStop::StateChangeDuringStaticCall, Bytes::new()))
        } else {
            None
        };
        if let Some((stop, output)) = failure {
            return CallAction::Override(Self::result(message, stop, output));
        }
        if let Some((path, args, value, salt)) = Self::deployment_args(&decoded) {
            return self.deploy_code(interp, message, path, args, value, salt);
        }
        let (stop, output) = self.apply(interp, message, decoded);
        CallAction::Override(Self::result(message, stop, output))
    }

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        self.finish_log_error(result);
        if message.call_target == CHEATCODE_ADDRESS {
            if Self::is_deployment_call(&message.input)
                && let Some(address) = result.created_address.filter(|_| result.is_success())
            {
                result.output = address.abi_encode().into();
                result.created_address = None;
                result.stop = InstrStop::Return;
            }
        } else {
            if let Some(broadcast) = &self.broadcast
                && message.depth > 0
                && usize::from(message.depth - 1) == broadcast.depth
                && message.caller == broadcast.new_origin
            {
                interp.host().ext_mut().origin_override = Some(broadcast.original_origin);
                if broadcast.single_call {
                    self.broadcast = None;
                }
            }
            self.finish_prank(interp, message);
            self.finish_expected_revert(message, result, false);
            self.finish_expected_emit(message, result);
            if message.depth == 0
                && result.is_success()
                && let Some(reason) = expected_call::first_unmet_call(&self.expected_calls)
            {
                result.stop = InstrStop::Revert;
                result.output = Error::encode(reason);
            }
        }
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        self.observe_revert_depth(message);
        if let Some(result) = self.apply_broadcast_create(interp, message) {
            return Some(result);
        }
        if message.depth > 0 {
            let depth = usize::from(message.depth - 1);
            if let Some(prank) = self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank)
                && message.caller == prank.prank_caller
            {
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
                    let nonce = interp
                        .host()
                        .state_mut()
                        .account(&prank.new_caller, false)
                        .map(|a| a.nonce());
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
            }
        }
        if message.depth == 0 || self.allowed_callers.contains(&message.caller) {
            self.allowed_callers.insert(message.destination);
        }
        None
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        self.finish_log_error(result);
        if let Some(broadcast) = &self.broadcast
            && message.depth > 0
            && usize::from(message.depth - 1) == broadcast.depth
            && message.caller == broadcast.new_origin
        {
            interp.host().ext_mut().origin_override = Some(broadcast.original_origin);
            if broadcast.single_call {
                self.broadcast = None;
            }
        }
        self.finish_prank(interp, message);
        self.finish_expected_revert(message, result, true);
    }
}
