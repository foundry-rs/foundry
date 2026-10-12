//! Ethereum cheatcodes operating directly on the live EVM2 host.

use crate::{
    BroadcastableTransaction, Cheatcode, CheatsConfig, Error, Vm, Wallets,
    evm::{
        mock::{self, MockCallDataContext, MockCallReturnData},
        prank::Prank,
    },
    expected_emit::{self, ExpectedEmitTracker},
    script::Broadcast,
    test::{
        expect::{
            self, ExpectedCallScheme, ExpectedCallTracker, ExpectedRevert, ExpectedRevertKind,
        },
        revert_handlers,
    },
};
use alloy_consensus::BlobTransactionSidecarVariant;
use alloy_eips::eip7702::SignedAuthorization;
use alloy_primitives::{Address, B256, Bytes, Log, U256, keccak256, map::HashMap};
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use evm2::{
    Inspector, SpecId,
    bytecode::Bytecode,
    interpreter::{GasTracker, InstrStop, Interpreter, Message, MessageKind, MessageResult},
};
use evm2_inspectors::tracing::TracingInspector;
use foundry_common::fmt::ConsoleFmt;
use foundry_evm_core::{
    abi::console,
    constants::{
        CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, DEFAULT_CREATE2_DEPLOYER_CODEHASH,
        GLOBAL_FAIL_SLOT, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME, MAGIC_SKIP,
    },
    ethereum::FoundryEvmTypes,
    state_snapshot::StateSnapshots,
};
use foundry_evm_coverage::EthereumCoverageCollector;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

mod snapshot;
use snapshot::{Snapshot, SnapshotFrame};

mod artifacts;

mod isolation;

mod replay;

mod broadcast;

mod delegation;

mod storage_hooks;

mod forks;
use forks::Forks;

pub(crate) enum ApplyError {
    Cheat(Error),
    Database(evm2::DatabaseError),
    Execution(evm2::ExecutionError),
}

impl From<Error> for ApplyError {
    fn from(error: Error) -> Self {
        Self::Cheat(error)
    }
}

impl From<evm2::DatabaseError> for ApplyError {
    fn from(error: evm2::DatabaseError) -> Self {
        Self::Database(error)
    }
}

/// Inspector state owned by an Ethereum execution session.
#[derive(Clone, Debug, Default)]
pub struct Cheatcodes {
    config: Arc<CheatsConfig>,
    /// All observed logs, including console messages and logs from reverted frames.
    pub logs: Vec<Log>,
    pranks: BTreeMap<usize, Prank>,
    pub(crate) expected_revert: Option<ExpectedRevert>,
    pub(crate) expected_calls: ExpectedCallTracker,
    pub(crate) mocked_calls:
        HashMap<Address, BTreeMap<MockCallDataContext, VecDeque<MockCallReturnData>>>,
    pub(crate) mocked_functions: HashMap<Address, HashMap<Bytes, Address>>,
    pub(crate) expected_emits: ExpectedEmitTracker,
    pending_emit_failure: Option<&'static str>,
    emit_failure: Option<(u16, &'static str)>,
    /// Skip payloads actually emitted by vm.skip, used to distinguish user-crafted reverts.
    pub skip_payloads: Vec<Bytes>,
    /// Assertion failures observed before a snapshot restoration, which must not erase them.
    pub snapshot_failure: bool,
    /// First reverting call before expectation hooks rewrite its result.
    pub reverter: Option<Address>,
    snapshots: StateSnapshots<Snapshot>,
    transaction_epoch: u64,
    snapshot_frames: Vec<SnapshotFrame>,
    deploy_code_depth: Option<u16>,
    isolated_root: Option<Message<FoundryEvmTypes>>,
    replay_depth: Option<u16>,
    isolation_depth: Option<u16>,
    isolated_snapshot_restored: bool,
    forks: Forks,
    broadcast: Option<Broadcast>,
    active_delegations: Vec<SignedAuthorization>,
    active_blob_sidecar: Option<BlobTransactionSidecarVariant>,
    dynamic_gas_limit: bool,
    storage_hooks: storage_hooks::StorageHooks,
    /// Wallets available to script broadcasting, shared with its publishing owner.
    pub wallets: Option<Wallets>,
    /// Transaction intents collected by the native script execution.
    pub transactions: VecDeque<BroadcastableTransaction>,
    /// Native call-tree collection, transferred with the inspector into isolated execution.
    pub tracing: Option<TracingInspector>,
    /// Bytecode observations collected only for source coverage consumers.
    pub line_coverage: Option<EthereumCoverageCollector>,
    /// Ephemeral script address whose ADDRESS/CALLER use is guarded during script execution.
    pub script_address: Option<Address>,
    script_failure: Option<(u16, Bytes)>,
}

impl Cheatcodes {
    /// Creates native cheatcode state with the shared Foundry policy.
    pub fn new(config: Arc<CheatsConfig>) -> Self {
        Self { config, ..Default::default() }
    }

    /// Applies effective per-test policy while retaining the owning artifact context.
    pub fn reconfigure(&mut self, config: &foundry_config::Config) {
        let mut opts = self.config.evm_opts.clone();
        opts.always_use_create_2_factory = config.always_use_create_2_factory;
        self.config = Arc::new(self.config.clone_with(config, opts));
    }

    fn apply(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        call: Vm::VmCalls,
    ) -> Result<Bytes, ApplyError> {
        match call {
            Vm::VmCalls::registerSstoreHook(call) => {
                self.storage_hooks.stores.insert(
                    call.target,
                    crate::StorageHook {
                        callback_target: message.caller,
                        callback_selector: call.callback.0,
                    },
                );
            }
            Vm::VmCalls::registerSloadHook(call) => {
                self.storage_hooks.loads.insert(
                    call.target,
                    crate::StorageHook {
                        callback_target: message.caller,
                        callback_selector: call.callback.0,
                    },
                );
            }
            Vm::VmCalls::addr(call) => {
                return Ok(crate::wallet::parse_wallet(&call.privateKey)?
                    .address()
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::broadcast_0(_) | Vm::VmCalls::startBroadcast_0(_) => {
                let single = matches!(call, Vm::VmCalls::broadcast_0(_));
                self.start_broadcast(interp, message, None, single)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::broadcast_1(call) => {
                self.start_broadcast(interp, message, Some(call.signer), true)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::startBroadcast_1(call) => {
                self.start_broadcast(interp, message, Some(call.signer), false)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::broadcast_2(call) => {
                let wallet = crate::wallet::parse_wallet(&call.privateKey)?;
                self.start_broadcast(interp, message, Some(wallet.address()), true)?;
                if let Some(wallets) = &self.wallets {
                    wallets.add_local_signer(wallet);
                }
                return Ok(Bytes::new());
            }
            Vm::VmCalls::startBroadcast_2(call) => {
                let wallet = crate::wallet::parse_wallet(&call.privateKey)?;
                self.start_broadcast(interp, message, Some(wallet.address()), false)?;
                if let Some(wallets) = &self.wallets {
                    wallets.add_local_signer(wallet);
                }
                return Ok(Bytes::new());
            }
            Vm::VmCalls::stopBroadcast(_) => {
                self.broadcast
                    .take()
                    .ok_or_else(|| Error::from("no broadcast in progress to stop"))?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::signDelegation_0(call) => {
                return self.sign_delegation(
                    interp,
                    call.privateKey,
                    call.implementation,
                    None,
                    false,
                    false,
                );
            }
            Vm::VmCalls::signDelegation_1(call) => {
                return self.sign_delegation(
                    interp,
                    call.privateKey,
                    call.implementation,
                    Some(call.nonce),
                    false,
                    false,
                );
            }
            Vm::VmCalls::signDelegation_2(call) => {
                return self.sign_delegation(
                    interp,
                    call.privateKey,
                    call.implementation,
                    None,
                    call.crossChain,
                    false,
                );
            }
            Vm::VmCalls::signAndAttachDelegation_0(call) => {
                return self.sign_delegation(
                    interp,
                    call.privateKey,
                    call.implementation,
                    None,
                    false,
                    true,
                );
            }
            Vm::VmCalls::signAndAttachDelegation_1(call) => {
                return self.sign_delegation(
                    interp,
                    call.privateKey,
                    call.implementation,
                    Some(call.nonce),
                    false,
                    true,
                );
            }
            Vm::VmCalls::signAndAttachDelegation_2(call) => {
                return self.sign_delegation(
                    interp,
                    call.privateKey,
                    call.implementation,
                    None,
                    call.crossChain,
                    true,
                );
            }
            Vm::VmCalls::attachBlob(call) => {
                if interp.spec() < SpecId::CANCUN {
                    return Err(Error::from("`attachBlob` is not supported before the Cancun hard fork; see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844").into());
                }
                self.active_blob_sidecar = Some(crate::script::build_blob_sidecar(
                    &call.blob,
                    interp.spec() >= SpecId::OSAKA,
                )?);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::attachDelegation_0(call) => {
                self.attach_delegation(interp, &call.signedDelegation, false)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::attachDelegation_1(call) => {
                self.attach_delegation(interp, &call.signedDelegation, call.crossChain)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::transact_0(call) => {
                self.transact(interp, None, call.txHash, message.depth)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::transact_1(call) => {
                self.transact(interp, Some(call.forkId), call.txHash, message.depth)?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::allowCheatcodes(call) => {
                self.forks.cheatcode_access.insert(call.account);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::activeFork(_) => {
                return Ok(self
                    .forks
                    .active
                    .ok_or_else(|| Error::from("no active fork"))?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::createFork_0(call) => {
                return Ok(self
                    .forks
                    .create(&self.config, &call.urlOrAlias, None)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::createFork_1(call) => {
                return Ok(self
                    .forks
                    .create(&self.config, &call.urlOrAlias, Some(call.blockNumber.saturating_to()))?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::createFork_2(call) => {
                let id = self.forks.create_at_transaction(
                    &self.config,
                    &call.urlOrAlias,
                    call.txHash,
                    interp.host(),
                )?;
                return Ok(id.abi_encode().into());
            }
            Vm::VmCalls::createSelectFork_0(call) => {
                let id = self.forks.create(&self.config, &call.urlOrAlias, None)?;
                self.forks.select(interp.host(), id, message.caller, &mut self.snapshot_frames)?;
                return Ok(id.abi_encode().into());
            }
            Vm::VmCalls::createSelectFork_1(call) => {
                let id = self.forks.create(
                    &self.config,
                    &call.urlOrAlias,
                    Some(call.blockNumber.saturating_to()),
                )?;
                self.forks.select(interp.host(), id, message.caller, &mut self.snapshot_frames)?;
                return Ok(id.abi_encode().into());
            }
            Vm::VmCalls::createSelectFork_2(call) => {
                let id = self.forks.create_at_transaction(
                    &self.config,
                    &call.urlOrAlias,
                    call.txHash,
                    interp.host(),
                )?;
                self.forks.select(interp.host(), id, message.caller, &mut self.snapshot_frames)?;
                return Ok(id.abi_encode().into());
            }
            Vm::VmCalls::selectFork(call) => {
                self.forks.select(
                    interp.host(),
                    call.forkId,
                    message.caller,
                    &mut self.snapshot_frames,
                )?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::rollFork_0(call) => {
                self.forks.roll(
                    interp.host(),
                    None,
                    call.blockNumber.saturating_to(),
                    message.caller,
                    &mut self.snapshot_frames,
                )?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::rollFork_2(call) => {
                self.forks.roll(
                    interp.host(),
                    Some(call.forkId),
                    call.blockNumber.saturating_to(),
                    message.caller,
                    &mut self.snapshot_frames,
                )?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::rollFork_1(call) => {
                self.forks.roll_to_transaction(
                    &self.config,
                    interp.host(),
                    None,
                    call.txHash,
                    message.caller,
                    &mut self.snapshot_frames,
                )?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::rollFork_3(call) => {
                self.forks.roll_to_transaction(
                    &self.config,
                    interp.host(),
                    Some(call.forkId),
                    call.txHash,
                    message.caller,
                    &mut self.snapshot_frames,
                )?;
                return Ok(Bytes::new());
            }
            Vm::VmCalls::makePersistent_0(call) => {
                self.forks.persistent.insert(call.account);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::makePersistent_1(call) => {
                self.forks.persistent.extend([call.account0, call.account1]);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::makePersistent_2(call) => {
                self.forks.persistent.extend([call.account0, call.account1, call.account2]);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::makePersistent_3(call) => {
                self.forks.persistent.extend(call.accounts);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::revokePersistent_0(call) => {
                self.forks.persistent.remove(&call.account);
                return Ok(Bytes::new());
            }
            Vm::VmCalls::revokePersistent_1(call) => {
                for account in call.accounts {
                    self.forks.persistent.remove(&account);
                }
                return Ok(Bytes::new());
            }
            Vm::VmCalls::isPersistent(call) => {
                return Ok(self.forks.persistent.contains(&call.account).abi_encode().into());
            }
            Vm::VmCalls::computeCreateAddress(call) => {
                if call.nonce > U256::from(u64::MAX) {
                    return Err(Error::from("nonce must be less than 2^64").into());
                }
                return Ok(call.deployer.create(call.nonce.to()).abi_encode().into());
            }
            Vm::VmCalls::computeCreate2Address_0(call) => {
                return Ok(call.deployer.create2(call.salt, call.initCodeHash).abi_encode().into());
            }
            Vm::VmCalls::computeCreate2Address_1(call) => {
                return Ok(DEFAULT_CREATE2_DEPLOYER
                    .create2(call.salt, call.initCodeHash)
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::getCode(call) => {
                return Ok(crate::fs::get_artifact_code(&self.config, &call.artifactPath, false)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::getDeployedCode(call) => {
                return Ok(crate::fs::get_artifact_code(&self.config, &call.artifactPath, true)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::deployCode_0(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    None,
                    U256::ZERO,
                    None,
                );
            }
            Vm::VmCalls::deployCode_1(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    Some(&call.constructorArgs),
                    U256::ZERO,
                    None,
                );
            }
            Vm::VmCalls::deployCode_2(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    None,
                    call.value,
                    None,
                );
            }
            Vm::VmCalls::deployCode_3(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    Some(&call.constructorArgs),
                    call.value,
                    None,
                );
            }
            Vm::VmCalls::deployCode_4(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    None,
                    U256::ZERO,
                    Some(call.salt),
                );
            }
            Vm::VmCalls::deployCode_5(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    Some(&call.constructorArgs),
                    U256::ZERO,
                    Some(call.salt),
                );
            }
            Vm::VmCalls::deployCode_6(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    None,
                    call.value,
                    Some(call.salt),
                );
            }
            Vm::VmCalls::deployCode_7(call) => {
                return self.deploy_code(
                    interp,
                    message,
                    &call.artifactPath,
                    Some(&call.constructorArgs),
                    call.value,
                    Some(call.salt),
                );
            }
            Vm::VmCalls::deal(call) => {
                interp.host().state_mut().account(&call.account)?.set_balance(call.newBalance);
            }
            Vm::VmCalls::etch(call) => {
                if interp.host().precompiles().contains(&call.target) {
                    return Err(Error::from(format!(
                        "cannot use precompile {} as an argument",
                        call.target
                    ))
                    .into());
                }
                let code = Bytecode::new_raw_checked(call.newRuntimeBytecode)
                    .map_err(|error| Error::from(format!("failed to create bytecode: {error}")))?;
                let hash = keccak256(code.original_bytes());
                interp.host().state_mut().account(&call.target)?.set_code(hash, code);
            }
            Vm::VmCalls::load(call) => {
                let value = interp
                    .host()
                    .state_mut()
                    .storage_slot(&call.target, call.slot.into())?
                    .current();
                return Ok(B256::from(value.to_be_bytes()).abi_encode().into());
            }
            Vm::VmCalls::store(call) => {
                if interp.host().precompiles().contains(&call.target) {
                    return Err(Error::from(format!(
                        "cannot use precompile {} as an argument",
                        call.target
                    ))
                    .into());
                }
                let state = interp.host().state_mut();
                state.account(&call.target)?;
                state.storage_slot(&call.target, call.slot.into())?.set(call.value.into());
            }
            Vm::VmCalls::getNonce_0(call) => {
                return Ok(interp
                    .host()
                    .state_mut()
                    .account(&call.account)?
                    .nonce()
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::setNonce(call) => {
                let mut account = interp.host().state_mut().account(&call.account)?;
                if call.newNonce < account.nonce() {
                    return Err(Error::from(
                        "new nonce must be equal to or higher than the account's current nonce",
                    )
                    .into());
                }
                account.set_nonce(call.newNonce);
            }
            Vm::VmCalls::setNonceUnsafe(call) => {
                interp.host().state_mut().account(&call.account)?.set_nonce(call.newNonce);
            }
            Vm::VmCalls::warp(call) => {
                let host = interp.host();
                let mut block = *host.block();
                block.timestamp = call.newTimestamp;
                host.set_block(block);
            }
            Vm::VmCalls::roll(call) => {
                let host = interp.host();
                let mut block = *host.block();
                block.number = call.newHeight;
                host.set_block(block);
            }
            Vm::VmCalls::coinbase(call) => {
                let host = interp.host();
                let mut block = *host.block();
                block.beneficiary = call.newCoinbase;
                host.set_block(block);
            }
            Vm::VmCalls::chainId(call) => {
                interp.host().ext_mut().chain_id_override = Some(
                    call.newChainId
                        .try_into()
                        .map_err(|_| Error::from("chain ID must be less than 2^64"))?,
                );
            }
            Vm::VmCalls::fee(call) => {
                if call.newBasefee > U256::from(u64::MAX) {
                    return Err(Error::from("base fee must be less than 2^64").into());
                }
                let host = interp.host();
                let mut block = *host.block();
                block.basefee = call.newBasefee;
                host.set_block(block);
                host.ext_mut().basefee_override = Some(call.newBasefee);
            }
            Vm::VmCalls::txGasPrice(call) => {
                if call.newGasPrice > U256::from(u64::MAX) {
                    return Err(Error::from("gas price must be less than 2^64").into());
                }
                interp.host().ext_mut().gas_price_override = Some(call.newGasPrice);
            }
            Vm::VmCalls::difficulty(call) => {
                if interp.spec() >= SpecId::MERGE {
                    return Err(Error::from(
                        "`difficulty` is not supported after the Paris hard fork, use `prevrandao` instead; see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399",
                    ).into());
                }
                let host = interp.host();
                let mut block = *host.block();
                block.difficulty = call.newDifficulty;
                host.set_block(block);
            }
            Vm::VmCalls::prevrandao_0(call) => Self::prevrandao(interp, call.newPrevrandao.into())?,
            Vm::VmCalls::prevrandao_1(call) => Self::prevrandao(interp, call.newPrevrandao)?,
            Vm::VmCalls::getBlockNumber(_) => {
                return Ok(interp.host().block().number.abi_encode().into());
            }
            Vm::VmCalls::getBlockTimestamp(_) => {
                return Ok(interp.host().block().timestamp.abi_encode().into());
            }
            Vm::VmCalls::getChainId(_) => {
                let host = interp.host();
                return Ok(U256::from(
                    host.ext().chain_id_override.unwrap_or(host.version().chain_id),
                )
                .abi_encode()
                .into());
            }
            Vm::VmCalls::prank_0(call) => {
                self.prank(interp, message, call.msgSender, None, true, false)?
            }
            Vm::VmCalls::prank_1(call) => {
                self.prank(interp, message, call.msgSender, Some(call.txOrigin), true, false)?
            }
            Vm::VmCalls::prank_2(call) => {
                self.prank(interp, message, call.msgSender, None, true, call.delegateCall)?
            }
            Vm::VmCalls::prank_3(call) => self.prank(
                interp,
                message,
                call.msgSender,
                Some(call.txOrigin),
                true,
                call.delegateCall,
            )?,
            Vm::VmCalls::startPrank_0(call) => {
                self.prank(interp, message, call.msgSender, None, false, false)?
            }
            Vm::VmCalls::startPrank_1(call) => {
                self.prank(interp, message, call.msgSender, Some(call.txOrigin), false, false)?
            }
            Vm::VmCalls::startPrank_2(call) => {
                self.prank(interp, message, call.msgSender, None, false, call.delegateCall)?
            }
            Vm::VmCalls::startPrank_3(call) => self.prank(
                interp,
                message,
                call.msgSender,
                Some(call.txOrigin),
                false,
                call.delegateCall,
            )?,
            Vm::VmCalls::stopPrank(_) => {
                self.pranks.remove(&usize::from(message.depth.saturating_sub(1)));
            }
            Vm::VmCalls::snapshot(_) | Vm::VmCalls::snapshotState(_) => {
                return Ok(self.snapshot(interp.host()).abi_encode().into());
            }
            Vm::VmCalls::revertTo(call) => {
                return Ok(self
                    .restore_snapshot(interp.host(), call.snapshotId, false)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::revertToState(call) => {
                return Ok(self
                    .restore_snapshot(interp.host(), call.snapshotId, false)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::revertToAndDelete(call) => {
                return Ok(self
                    .restore_snapshot(interp.host(), call.snapshotId, true)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::revertToStateAndDelete(call) => {
                return Ok(self
                    .restore_snapshot(interp.host(), call.snapshotId, true)?
                    .abi_encode()
                    .into());
            }
            Vm::VmCalls::deleteSnapshot(call) => {
                return Ok(self.snapshots.remove_at(call.snapshotId).is_some().abi_encode().into());
            }
            Vm::VmCalls::deleteStateSnapshot(call) => {
                return Ok(self.snapshots.remove_at(call.snapshotId).is_some().abi_encode().into());
            }
            Vm::VmCalls::deleteSnapshots(_) | Vm::VmCalls::deleteStateSnapshots(_) => {
                self.snapshots.clear();
            }
            call => {
                macro_rules! dispatch {
                    ($($variant:ident),*) => {
                        match call {
                            $(Vm::VmCalls::$variant(cheat) => cheat.apply_evm2(self, interp),)*
                            call => unsupported_call(call),
                        }
                    };
                }
                return dispatch!(
                    _expectCheatcodeRevert_0,
                    _expectCheatcodeRevert_1,
                    _expectCheatcodeRevert_2,
                    assertApproxEqAbsDecimal_0,
                    assertApproxEqAbsDecimal_1,
                    assertApproxEqAbsDecimal_2,
                    assertApproxEqAbsDecimal_3,
                    assertApproxEqAbs_0,
                    assertApproxEqAbs_1,
                    assertApproxEqAbs_2,
                    assertApproxEqAbs_3,
                    assertApproxEqRelDecimal_0,
                    assertApproxEqRelDecimal_1,
                    assertApproxEqRelDecimal_2,
                    assertApproxEqRelDecimal_3,
                    assertApproxEqRel_0,
                    assertApproxEqRel_1,
                    assertApproxEqRel_2,
                    assertApproxEqRel_3,
                    assertEqDecimal_0,
                    assertEqDecimal_1,
                    assertEqDecimal_2,
                    assertEqDecimal_3,
                    assertEq_0,
                    assertEq_1,
                    assertEq_10,
                    assertEq_11,
                    assertEq_12,
                    assertEq_13,
                    assertEq_14,
                    assertEq_15,
                    assertEq_16,
                    assertEq_17,
                    assertEq_18,
                    assertEq_19,
                    assertEq_2,
                    assertEq_20,
                    assertEq_21,
                    assertEq_22,
                    assertEq_23,
                    assertEq_24,
                    assertEq_25,
                    assertEq_26,
                    assertEq_27,
                    assertEq_3,
                    assertEq_4,
                    assertEq_5,
                    assertEq_6,
                    assertEq_7,
                    assertEq_8,
                    assertEq_9,
                    assertFalse_0,
                    assertFalse_1,
                    assertGeDecimal_0,
                    assertGeDecimal_1,
                    assertGeDecimal_2,
                    assertGeDecimal_3,
                    assertGe_0,
                    assertGe_1,
                    assertGe_2,
                    assertGe_3,
                    assertGtDecimal_0,
                    assertGtDecimal_1,
                    assertGtDecimal_2,
                    assertGtDecimal_3,
                    assertGt_0,
                    assertGt_1,
                    assertGt_2,
                    assertGt_3,
                    assertLeDecimal_0,
                    assertLeDecimal_1,
                    assertLeDecimal_2,
                    assertLeDecimal_3,
                    assertLe_0,
                    assertLe_1,
                    assertLe_2,
                    assertLe_3,
                    assertLtDecimal_0,
                    assertLtDecimal_1,
                    assertLtDecimal_2,
                    assertLtDecimal_3,
                    assertLt_0,
                    assertLt_1,
                    assertLt_2,
                    assertLt_3,
                    assertNotEqDecimal_0,
                    assertNotEqDecimal_1,
                    assertNotEqDecimal_2,
                    assertNotEqDecimal_3,
                    assertNotEq_0,
                    assertNotEq_1,
                    assertNotEq_10,
                    assertNotEq_11,
                    assertNotEq_12,
                    assertNotEq_13,
                    assertNotEq_14,
                    assertNotEq_15,
                    assertNotEq_16,
                    assertNotEq_17,
                    assertNotEq_18,
                    assertNotEq_19,
                    assertNotEq_2,
                    assertNotEq_20,
                    assertNotEq_21,
                    assertNotEq_22,
                    assertNotEq_23,
                    assertNotEq_24,
                    assertNotEq_25,
                    assertNotEq_26,
                    assertNotEq_27,
                    assertNotEq_3,
                    assertNotEq_4,
                    assertNotEq_5,
                    assertNotEq_6,
                    assertNotEq_7,
                    assertNotEq_8,
                    assertNotEq_9,
                    assertTrue_0,
                    assertTrue_1,
                    clearMockedCalls,
                    expectCallMinGas_0,
                    expectCallMinGas_1,
                    expectCall_0,
                    expectCall_1,
                    expectCall_2,
                    expectCall_3,
                    expectCall_4,
                    expectCall_5,
                    expectDelegateCall,
                    expectEmitAnonymous_0,
                    expectEmitAnonymous_1,
                    expectEmitAnonymous_2,
                    expectEmitAnonymous_3,
                    expectEmit_0,
                    expectEmit_1,
                    expectEmit_2,
                    expectEmit_3,
                    expectEmit_4,
                    expectEmit_5,
                    expectEmit_6,
                    expectEmit_7,
                    expectPartialRevert_0,
                    expectPartialRevert_1,
                    expectRevert_0,
                    expectRevert_1,
                    expectRevert_10,
                    expectRevert_11,
                    expectRevert_2,
                    expectRevert_3,
                    expectRevert_4,
                    expectRevert_5,
                    expectRevert_6,
                    expectRevert_7,
                    expectRevert_8,
                    expectRevert_9,
                    mockCallRevert_0,
                    mockCallRevert_1,
                    mockCallRevert_2,
                    mockCallRevert_3,
                    mockCall_0,
                    mockCall_1,
                    mockCall_2,
                    mockCall_3,
                    mockCall_4,
                    mockCalls_0,
                    mockCalls_1,
                    mockFunction
                );
            }
        }
        Ok(Bytes::new())
    }

    pub(crate) fn assertion_failure(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &str,
    ) -> Result<Bytes, ApplyError> {
        if self.config.assertions_revert {
            return Err(Error::from(message.to_owned()).into());
        }
        self.console_log(message);
        interp
            .host()
            .state_mut()
            .storage_slot(&CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)?
            .set(U256::ONE);
        Ok(Bytes::new())
    }

    fn console_log(&mut self, message: &str) {
        self.logs.push(Log::new_unchecked(
            HARDHAT_CONSOLE_ADDRESS,
            vec![console::ds::log::SIGNATURE_HASH],
            message.abi_encode().into(),
        ));
    }

    fn prevrandao(
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        value: U256,
    ) -> Result<(), Error> {
        if interp.spec() < SpecId::MERGE {
            return Err(Error::from(
                "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead",
            ));
        }
        let host = interp.host();
        let mut block = *host.block();
        block.prevrandao = value;
        host.set_block(block);
        Ok(())
    }

    fn prank(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        caller: Address,
        origin: Option<Address>,
        single: bool,
        delegate: bool,
    ) -> Result<(), Error> {
        let depth = usize::from(message.depth.saturating_sub(1));
        if self.broadcast.is_some() {
            return Err(Error::from(
                "you have an active broadcast; broadcasting and pranks are not compatible",
            ));
        }
        if self.pranks.get(&depth).is_some_and(|prank| !prank.used) {
            return Err(Error::from("cannot overwrite a prank until it is applied at least once"));
        }
        let host = interp.host();
        let original =
            host.ext().origin_override.or(host.ext().transaction_origin).unwrap_or(message.caller);
        self.pranks.insert(
            depth,
            Prank::new(message.caller, original, caller, origin, depth, single, delegate),
        );
        Ok(())
    }

    fn apply_error(
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        error: ApplyError,
    ) -> MessageResult<FoundryEvmTypes> {
        match error {
            ApplyError::Cheat(error) => {
                Self::result(message, InstrStop::Revert, error.abi_encode().into())
            }
            ApplyError::Database(error) => {
                let _ = interp.fail(error);
                Self::result(message, InstrStop::FatalExternalError, Bytes::new())
            }
            ApplyError::Execution(error) => {
                let _ = interp.fail(error);
                Self::result(message, InstrStop::FatalExternalError, Bytes::new())
            }
        }
    }

    fn finish_expected_revert(
        &mut self,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        let Some(expected) = &mut self.expected_revert else { return };
        // Database failures must remain fatal, even when a revert is expected.
        if result.stop.is_fatal() {
            return;
        }
        let is_create = message.kind.is_create();
        let is_cheatcode = !is_create && message.call_target == CHEATCODE_ADDRESS;
        let failed = !result.stop.is_success();
        if expected.reverter.is_some() && failed {
            if is_create {
                if result.stop == InstrStop::Revert && expected.reverted_by.is_none() {
                    expected.reverted_by = Some(message.destination);
                }
            } else if expected.reverted_by.is_none() || expected.count > 1 {
                expected.reverted_by = Some(message.destination);
            }
        }
        // Foundry counts the executing root frame as depth one, but its end hook as zero.
        let depth = usize::from(message.depth)
            - usize::from(
                is_create
                    && self
                        .deploy_code_depth
                        .is_some_and(|depth| depth.checked_add(1) == Some(message.depth)),
            );
        if depth > expected.depth {
            return;
        }
        let process = if is_create {
            matches!(expected.kind, ExpectedRevertKind::Default)
        } else {
            expected.needs_processing(
                is_cheatcode,
                failed,
                depth,
                self.config.internal_expect_revert,
            )
        };
        if !process {
            if let ExpectedRevertKind::Cheatcode { pending_processing } = &mut expected.kind {
                *pending_processing = false;
            }
            return;
        }
        let mut expected = self.expected_revert.take().expect("active expectation");
        let mut decoder = foundry_evm_core::decode::RevertDecoder::new();
        if let Some(contracts) = &self.config.available_artifacts {
            decoder = decoder.with_abis(contracts.values().map(|contract| &contract.abi));
        }
        let stop = result.stop;
        match revert_handlers::check_expect_revert(
            is_cheatcode,
            is_create,
            self.config.internal_expect_revert,
            &expected,
            stop.is_success(),
            stop == InstrStop::Revert,
            result.output.clone(),
            &self.config.available_artifacts,
            |data| decoder.maybe_decode_data(data).unwrap_or_else(|| format!("EvmError: {stop:?}")),
        ) {
            Ok((address, output)) => {
                expected.actual_count += 1;
                if expected.actual_count < expected.count {
                    if is_create {
                        expected.reverted_by = None;
                    }
                    self.expected_revert = Some(expected);
                }
                // EVM2 has already resolved the message checkpoint before this hook.
                result.stop = InstrStop::Return;
                result.output = output;
                result.created_address = address;
            }
            Err(error) => {
                result.stop = InstrStop::Revert;
                result.output = error.abi_encode().into();
                result.created_address = None;
            }
        }
    }

    const fn result(
        message: &Message<FoundryEvmTypes>,
        stop: InstrStop,
        output: Bytes,
    ) -> MessageResult<FoundryEvmTypes> {
        let mut gas =
            GasTracker::new_with_execution_gas_and_reservoir(message.gas_limit, message.reservoir);
        gas.settle_gas(stop);
        MessageResult::<FoundryEvmTypes> {
            stop,
            output,
            gas,
            created_address: None,
            ext: (),
            _non_exhaustive: (),
        }
    }
}

impl Cheatcodes {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        self.initialize_snapshot_frame(interp);
    }

    const fn step_end(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        if let Some(error) = self.pending_emit_failure.take() {
            if let Err(stop) = interp.result()
                && stop.is_fatal()
            {
                return;
            }
            self.emit_failure = Some((interp.message().depth, error));
            interp.set_stop(InstrStop::Revert);
        }
    }

    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        if message.depth == 0
            && let Some(depth) = self.replay_depth
        {
            message.depth = depth;
        }
        if message.depth == 0
            && let Some(root) = self.isolated_root.take()
        {
            let gas_limit = message.gas_limit;
            let reservoir = message.reservoir;
            *message = root;
            message.gas_limit = gas_limit;
            message.reservoir = reservoir;
            self.start_snapshot_frame(interp.host(), message);
            return None;
        }
        if message.depth == 0 {
            self.transaction_epoch = self.transaction_epoch.wrapping_add(1);
            self.forks.persistent.insert(message.destination);
            self.reverter = None;
            // The script/test wrapper call is synthetic; only its collected child transactions
            // consume the sender's on-chain nonce. Isolated roots bypass this correction above.
            let result = interp.host().state_mut().account(&message.caller).map(|mut account| {
                let nonce = account.get().map_or(0, |info| info.nonce);
                account.set_nonce(nonce.saturating_sub(1));
            });
            if let Err(error) = result {
                let _ = interp.fail(error);
                return Some(Self::result(message, InstrStop::FatalExternalError, Bytes::new()));
            }
        }
        self.start_snapshot_frame(interp.host(), message);
        if self.storage_hooks.active && message.caller == CHEATCODE_ADDRESS {
            return None;
        }
        if message.call_target == HARDHAT_CONSOLE_ADDRESS {
            return Some(match console::hh::ConsoleCalls::abi_decode(&message.input) {
                Ok(call) => {
                    for line in call.fmt(Default::default()).lines() {
                        self.console_log(line);
                    }
                    Self::result(message, InstrStop::Return, Bytes::new())
                }
                Err(error) => Self::result(message, InstrStop::Revert, Error::encode(error)),
            });
        }
        if message.call_target != CHEATCODE_ADDRESS {
            let mut observed_address = message.call_target;
            if let Some(mocks) = self.mocked_functions.get(&message.call_target)
                && let Some(target) = mocks
                    .get(&message.input)
                    .or_else(|| message.input.get(..4).and_then(|selector| mocks.get(selector)))
            {
                match interp.host().state_mut().account(target).and_then(|mut account| {
                    account.warm();
                    account.load_code()
                }) {
                    Ok(code) => {
                        message.code = code;
                        message.code_address = *target;
                        message.disable_precompiles = false;
                        observed_address = *target;
                    }
                    Err(error) => {
                        let _ = interp.fail(error);
                        return Some(Self::result(
                            message,
                            InstrStop::FatalExternalError,
                            Bytes::new(),
                        ));
                    }
                }
            }
            if let Some(expected) = self.expected_calls.get_mut(&observed_address) {
                let scheme = match message.kind {
                    MessageKind::Call => ExpectedCallScheme::Call,
                    MessageKind::CallCode => ExpectedCallScheme::CallCode,
                    MessageKind::DelegateCall => ExpectedCallScheme::DelegateCall,
                    MessageKind::StaticCall => ExpectedCallScheme::StaticCall,
                    _ => unreachable!("call hook received a non-call message"),
                };
                expect::observe_call(
                    expected,
                    &message.input,
                    (message.kind != MessageKind::DelegateCall).then_some(message.value),
                    message.gas_limit,
                    scheme,
                );
            }
            if let Some(expected) = &mut self.expected_revert {
                expected.max_depth = expected.max_depth.max(usize::from(message.depth) + 1);
            }
            if message.depth > 0 {
                if let Err(error) = self.broadcast_message(interp, message) {
                    return Some(Self::apply_error(interp, message, error));
                }
                let depth = usize::from(message.depth - 1);
                if let Some(prank) =
                    self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank)
                {
                    if prank.delegate_call
                        && depth == prank.depth
                        && message.kind == MessageKind::DelegateCall
                    {
                        message.destination = prank.new_caller;
                        message.caller = prank.new_caller;
                        if let Some(origin) = prank.new_origin {
                            interp.host().ext_mut().origin_override = Some(origin);
                        }
                        if let Some(used) = prank.first_time_applied() {
                            self.pranks.insert(prank.depth, used);
                        }
                    } else if let Some(changes) = prank.changes_for(depth, message.caller) {
                        if let Some(caller) = changes.caller {
                            message.caller = caller;
                        }
                        if let Some(origin) = changes.origin {
                            interp.host().ext_mut().origin_override = Some(origin);
                        }
                        if let Some(used) = changes.used {
                            self.pranks.insert(prank.depth, used);
                        }
                    }
                }
            }
            if let Some(mocks) = self.mocked_calls.get_mut(&observed_address)
                && let Some(queue) = mock::find_mock_returns(
                    mocks,
                    &message.input,
                    (message.kind != MessageKind::DelegateCall).then_some(message.value),
                )
                && let Some(return_data) = queue.front().cloned()
            {
                let host = interp.host();
                let checkpoint = host.state().checkpoint();
                if message.kind != MessageKind::DelegateCall {
                    match host.state_mut().transfer(
                        &message.caller,
                        &message.destination,
                        &message.value,
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
                            let features = host.version().features;
                            host.state_mut().rollback(checkpoint, features);
                            return Some(Self::result(
                                message,
                                InstrStop::OutOfFunds,
                                Bytes::new(),
                            ));
                        }
                        Err(error) => {
                            let features = host.version().features;
                            host.state_mut().rollback(checkpoint, features);
                            let _ = interp.fail(error);
                            return Some(Self::result(
                                message,
                                InstrStop::FatalExternalError,
                                Bytes::new(),
                            ));
                        }
                    }
                }
                if return_data.reverts {
                    let features = host.version().features;
                    host.state_mut().rollback(checkpoint, features);
                }
                mock::advance_mock_returns(queue);
                return Some(Self::result(
                    message,
                    if return_data.reverts { InstrStop::Revert } else { InstrStop::Return },
                    return_data.data,
                ));
            }
            if message.kind == MessageKind::StaticCall && self.should_isolate(message) {
                interp.host().state_mut().cool_loaded_state();
            }
            if message.kind == MessageKind::Call && self.should_isolate(message) {
                return Some(match self.isolate(interp, message) {
                    Ok(result) => result,
                    Err(ApplyError::Cheat(error)) => {
                        Self::result(message, InstrStop::Revert, error.abi_encode().into())
                    }
                    Err(ApplyError::Database(error)) => {
                        let _ = interp.fail(error);
                        Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                    }
                    Err(ApplyError::Execution(error)) => {
                        let _ = interp.fail(error);
                        Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                    }
                });
            }
            return None;
        }
        if self.forks.active.is_some() && !self.forks.cheatcode_access.contains(&message.caller) {
            return Some(Self::result(
                message,
                InstrStop::Revert,
                Error::encode(format!(
                    "cheatcodes are not enabled for {}; see `vm.allowCheatcodes(address)`",
                    message.caller
                )),
            ));
        }
        let decoded = match Vm::VmCalls::abi_decode(&message.input) {
            Ok(call) => call,
            Err(error) => {
                return Some(Self::result(message, InstrStop::Revert, Error::encode(error)));
            }
        };
        if self
            .config
            .blocked_cheatcodes
            .contains(&message.input[..4].try_into().expect("decoded selector"))
        {
            return Some(Self::result(
                message,
                InstrStop::Revert,
                Error::encode("disabled during restricted execution"),
            ));
        }
        if message.depth > 1 && matches!(&decoded, Vm::VmCalls::skip_0(call) if call.skipTest) {
            return Some(Self::result(
                message,
                InstrStop::Revert,
                Error::encode("`skip` can only be used at test level"),
            ));
        }
        if message.depth > 1 && matches!(&decoded, Vm::VmCalls::skip_1(call) if call.skipTest) {
            return Some(Self::result(
                message,
                InstrStop::Revert,
                Error::encode("`skip` can only be used at test level"),
            ));
        }
        match decoded {
            Vm::VmCalls::assume(call) => Some(Self::result(
                message,
                if call.condition { InstrStop::Return } else { InstrStop::Revert },
                if call.condition { Bytes::new() } else { Bytes::from_static(MAGIC_ASSUME) },
            )),
            Vm::VmCalls::skip_0(call) if call.skipTest => {
                let output = Bytes::from_static(MAGIC_SKIP);
                self.skip_payloads.push(output.clone());
                Some(Self::result(message, InstrStop::Revert, output))
            }
            Vm::VmCalls::skip_1(call) if call.skipTest => {
                let mut output = MAGIC_SKIP.to_vec();
                output.extend_from_slice(call.reason.as_bytes());
                let output = Bytes::from(output);
                self.skip_payloads.push(output.clone());
                Some(Self::result(message, InstrStop::Revert, output))
            }
            Vm::VmCalls::skip_0(_) | Vm::VmCalls::skip_1(_) => {
                Some(Self::result(message, InstrStop::Return, Bytes::new()))
            }
            call => Some(match self.apply(interp, message, call) {
                Ok(output) => Self::result(message, InstrStop::Return, output),
                Err(ApplyError::Cheat(error)) => {
                    Self::result(message, InstrStop::Revert, error.abi_encode().into())
                }
                Err(ApplyError::Database(error)) => {
                    let _ = interp.fail(error);
                    Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                }
                Err(ApplyError::Execution(error)) => {
                    let _ = interp.fail(error);
                    Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                }
            }),
        }
    }

    fn log(&mut self, log: &Log, _: &mut evm2::Evm<'_, FoundryEvmTypes>) {
        self.logs.push(log.clone());
        if let Some(error) = expected_emit::observe_log(&mut self.expected_emits, log) {
            self.pending_emit_failure = Some(error);
        }
    }

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        if self.storage_hooks.failure.as_ref().is_some_and(|(depth, _)| *depth == message.depth) {
            result.output = self.storage_hooks.failure.take().expect("storage callback failure").1;
        }
        if self.isolation_depth == Some(message.depth) {
            self.finish_snapshot_frame(interp.host(), !result.stop.is_success(), false);
            return;
        }
        let execution_failed = !result.stop.is_success();
        if self.script_failure.as_ref().is_some_and(|(depth, _)| *depth == message.depth) {
            result.output = self.script_failure.take().expect("script frame failure").1;
        }
        if result.stop.is_revert() && self.reverter.is_none() {
            self.reverter = Some(message.destination);
        }
        if self.emit_failure.is_some_and(|(depth, _)| depth == message.depth) {
            let (_, error) = self.emit_failure.take().expect("matching emit failure");
            result.output = Error::encode(error);
        }
        if message.call_target == HARDHAT_CONSOLE_ADDRESS {
            self.finish_snapshot_frame(interp.host(), execution_failed, false);
            return;
        }
        if message.call_target != CHEATCODE_ADDRESS && message.depth > 0 {
            self.finish_broadcast(interp, message);
            let depth = usize::from(message.depth - 1);
            if let Some(prank) = self.pranks.get(&depth).copied() {
                interp.host().ext_mut().origin_override = Some(prank.prank_origin);
                if prank.single_call && prank.used {
                    self.pranks.remove(&depth);
                }
            }
        }
        self.finish_expected_revert(message, result);
        if message.call_target != CHEATCODE_ADDRESS
            && result.stop != InstrStop::Revert
            && !result.stop.is_fatal()
            && let Some(unmet) = expected_emit::check_call_emits(
                &mut self.expected_emits,
                usize::from(message.depth),
                message.kind == MessageKind::StaticCall || message.caller_is_static,
                result.stop.is_success(),
            )
        {
            result.stop = InstrStop::Revert;
            result.output = unmet.encode(|| None);
        }
        if message.depth == 0
            && result.stop != InstrStop::Revert
            && !result.stop.is_fatal()
            && let Some(error) =
                expect::first_unmet_call(&self.expected_calls, result.stop.is_success())
        {
            result.stop = InstrStop::Revert;
            result.output = Error::encode(error);
        }
        if message.depth == 0
            && result.stop != InstrStop::Revert
            && !result.stop.is_fatal()
            && let Some(error) = expected_emit::first_unmet_root_emit(
                &mut self.expected_emits,
                result.stop.is_success(),
            )
        {
            result.stop = InstrStop::Revert;
            result.output = Error::encode(error);
        }
        self.finish_snapshot_frame(
            interp.host(),
            execution_failed,
            !execution_failed && !result.stop.is_success(),
        );
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        if message.depth == 0
            && let Some(depth) = self.replay_depth
        {
            message.depth = depth;
            // Rebasing the signed root makes EVM2 apply the nested CREATE nonce bump too.
            // Remove the handler's bump so creation consumes exactly one nonce, even on revert.
            let result = interp.host().state_mut().account(&message.caller).map(|mut account| {
                let nonce = account.get().map_or(0, |info| info.nonce);
                account.set_nonce(nonce.saturating_sub(1));
            });
            if let Err(error) = result {
                let _ = interp.fail(error);
                return Some(Self::result(message, InstrStop::FatalExternalError, Bytes::new()));
            }
        }
        if message.depth == 0
            && let Some(root) = self.isolated_root.take()
        {
            let gas_limit = message.gas_limit;
            let reservoir = message.reservoir;
            *message = root;
            message.gas_limit = gas_limit;
            message.reservoir = reservoir;
            let caller = message.caller;
            let result = interp.host().state_mut().account(&caller).map(|mut account| {
                let nonce = account.get().map_or(0, |info| info.nonce);
                account.set_nonce(nonce.saturating_sub(1));
            });
            if let Err(error) = result {
                let _ = interp.fail(error);
                return Some(Self::result(message, InstrStop::FatalExternalError, Bytes::new()));
            }
            if self.forks.cheatcode_access.contains(&message.caller) {
                self.forks.cheatcode_access.insert(message.destination);
            }
            self.start_snapshot_frame(interp.host(), message);
            return None;
        }
        if message.depth == 0 {
            self.transaction_epoch = self.transaction_epoch.wrapping_add(1);
            interp.host().state_mut().prewarm(&message.destination);
        }
        self.start_snapshot_frame(interp.host(), message);
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = expected.max_depth.max(usize::from(message.depth) + 1);
        }
        if let Err(error) = self.broadcast_message(interp, message) {
            return Some(Self::apply_error(interp, message, error));
        }
        if message.depth > 0
            && !self
                .deploy_code_depth
                .is_some_and(|depth| depth.checked_add(1) == Some(message.depth))
        {
            let depth = usize::from(message.depth - 1);
            if let Some(prank) = self.pranks.range(..=depth).next_back().map(|(_, prank)| *prank)
                && let Some(changes) = prank.changes_for(depth, message.caller)
            {
                if let Some(caller) = changes.caller {
                    message.caller = caller;
                    let nonce = match interp.host().state_mut().account_info_untracked(&caller) {
                        Ok(info) => info.map_or(0, |info| info.nonce),
                        Err(error) => {
                            let _ = interp.fail(error);
                            return Some(Self::result(
                                message,
                                InstrStop::FatalExternalError,
                                Bytes::new(),
                            ));
                        }
                    };
                    message.destination = evm2::interpreter::derive_create_destination(
                        message.kind,
                        &caller,
                        &message.salt,
                        &message.input,
                        nonce,
                    );
                    message.call_target = message.destination;
                    message.code_address = message.destination;
                }
                if let Some(origin) = changes.origin {
                    interp.host().ext_mut().origin_override = Some(origin);
                }
                if let Some(used) = changes.used {
                    self.pranks.insert(prank.depth, used);
                }
            }
        }
        // Constructors inherit access from their deployer. Top-level deployment belongs to
        // Foundry's test/script owner; remote contracts remain untrusted in forking mode.
        if (message.depth <= 1 && self.replay_depth.is_none())
            || self.forks.cheatcode_access.contains(&message.caller)
        {
            self.forks.cheatcode_access.insert(message.destination);
        }
        if message.depth == 0 {
            self.forks.cheatcode_access.insert(message.caller);
        }
        if message.kind == MessageKind::Create2 && self.config.always_use_create_2_factory {
            let depth = usize::from(message.depth.saturating_sub(1))
                - usize::from(
                    self.deploy_code_depth
                        .is_some_and(|depth| depth.checked_add(1) == Some(message.depth)),
                );
            let target_depth = self
                .pranks
                .range(..=depth)
                .next_back()
                .map(|(_, prank)| prank.depth)
                .or_else(|| self.broadcast.as_ref().map(|broadcast| broadcast.depth))
                .unwrap_or(0);
            if depth == target_depth {
                return Some(match self.create2_factory(interp, message) {
                    Ok(result) => result,
                    Err(ApplyError::Cheat(error)) => {
                        Self::result(message, InstrStop::Revert, error.abi_encode().into())
                    }
                    Err(ApplyError::Database(error)) => {
                        let _ = interp.fail(error);
                        Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                    }
                    Err(ApplyError::Execution(error)) => {
                        let _ = interp.fail(error);
                        Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                    }
                });
            }
        }
        if message.kind == MessageKind::Create && self.should_isolate(message) {
            return Some(match self.isolate(interp, message) {
                Ok(result) => result,
                Err(ApplyError::Cheat(error)) => {
                    Self::result(message, InstrStop::Revert, error.abi_encode().into())
                }
                Err(ApplyError::Database(error)) => {
                    let _ = interp.fail(error);
                    Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                }
                Err(ApplyError::Execution(error)) => {
                    let _ = interp.fail(error);
                    Self::result(message, InstrStop::FatalExternalError, Bytes::new())
                }
            });
        }
        None
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        if self.storage_hooks.failure.as_ref().is_some_and(|(depth, _)| *depth == message.depth) {
            result.output = self.storage_hooks.failure.take().expect("storage callback failure").1;
        }
        if self.isolation_depth == Some(message.depth) {
            self.finish_snapshot_frame(interp.host(), !result.stop.is_success(), false);
            return;
        }
        let execution_failed = !result.stop.is_success();
        if self.emit_failure.is_some_and(|(depth, _)| depth == message.depth) {
            let (_, error) = self.emit_failure.take().expect("matching emit failure");
            result.output = Error::encode(error);
        }
        self.finish_broadcast(interp, message);
        if message.depth > 0
            && !self
                .deploy_code_depth
                .is_some_and(|depth| depth.checked_add(1) == Some(message.depth))
        {
            let depth = usize::from(message.depth - 1);
            if let Some(prank) = self.pranks.get(&depth).copied() {
                interp.host().ext_mut().origin_override = Some(prank.prank_origin);
                if prank.single_call && prank.used {
                    self.pranks.remove(&depth);
                }
            }
        }
        self.finish_expected_revert(message, result);
        self.finish_snapshot_frame(
            interp.host(),
            execution_failed,
            !execution_failed && !result.stop.is_success(),
        );
    }
}

impl Inspector<FoundryEvmTypes> for Cheatcodes {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        Self::initialize_interp(self, interp);
        if let Some(coverage) = &mut self.line_coverage {
            coverage.initialize_interp(interp);
        }
        if let Some(tracing) = &mut self.tracing {
            tracing.initialize_interp(interp);
        }
    }
    fn step(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        if let Some(coverage) = &mut self.line_coverage {
            coverage.step(interp);
        }
        if let Some(tracing) = &mut self.tracing {
            tracing.step(interp);
        }
        self.capture_storage_hook(interp);
        self.dynamic_gas_limit = match interp.opcode() {
            evm2::interpreter::op::CREATE2 => true,
            evm2::interpreter::op::CALL => interp
                .stack()
                .peek(0)
                .is_some_and(|gas| gas >= U256::from(interp.gas().remaining().saturating_sub(100))),
            _ => false,
        };
        let message = interp.message();
        if self.script_address == Some(message.destination)
            && self.script_address == Some(message.code_address)
        {
            let error = if interp.opcode() == evm2::interpreter::op::ADDRESS {
                Some(Bytes::from_static(b"Usage of `address(this)` detected in script contract. Script contracts are ephemeral and their addresses should not be relied upon."))
            } else if interp.opcode() == evm2::interpreter::op::CALLER
                && let Some(broadcast) = &self.broadcast
                && usize::from(message.depth) == broadcast.depth
                && message.caller != broadcast.new_origin
            {
                Some(Bytes::from(format!(
                    "Usage of `msg.sender` inside a `broadcast` in script contract detected. `msg.sender` is `{:#x}`, not the broadcast sender `{:#x}`. Use the `--sender` flag or pass the deployer address directly instead.",
                    message.caller, broadcast.new_origin
                ).into_bytes()))
            } else {
                None
            };
            if let Some(error) = error {
                self.script_failure = Some((message.depth, error));
                interp.set_stop(InstrStop::Revert);
            }
        }
    }
    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        Self::step_end(self, interp);
        self.invoke_storage_hook(interp);
        if let Some(tracing) = &mut self.tracing {
            tracing.step_end(interp);
        }
    }
    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        let isolated_root = message.depth == 0 && self.isolated_root.is_some();
        let index = if !isolated_root && let Some(tracing) = &mut self.tracing {
            tracing.call(interp, message);
            let index = tracing.traces().nodes().len() - 1;
            Some(index)
        } else {
            None
        };
        let result = Self::call(self, interp, message);
        if let Some(index) = index
            && let Some(tracing) = &mut self.tracing
        {
            let trace = &mut tracing.traces_mut().nodes_mut()[index].trace;
            trace.caller = message.caller;
            trace.address = message.destination;
        }
        result
    }
    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        let isolated_root = self.isolation_depth == Some(message.depth);
        Self::call_end(self, interp, message, result);
        if message.depth == 0 && !result.stop.is_fatal() {
            let host = interp.host();
            if let Err(error) = self.forks.finalize_inactive(host.spec_id(), host.version()) {
                let _ = interp.fail(error);
                result.stop = InstrStop::FatalExternalError;
            }
        }
        if !isolated_root && let Some(tracing) = &mut self.tracing {
            tracing.call_end(interp, message, result);
        }
    }
    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        let isolated_root = message.depth == 0 && self.isolated_root.is_some();
        let index = if !isolated_root && let Some(tracing) = &mut self.tracing {
            tracing.create(interp, message);
            let index = tracing.traces().nodes().len() - 1;
            Some(index)
        } else {
            None
        };
        let result = Self::create(self, interp, message);
        if let Some(index) = index
            && let Some(tracing) = &mut self.tracing
        {
            let trace = &mut tracing.traces_mut().nodes_mut()[index].trace;
            trace.caller = message.caller;
            trace.address = message.destination;
        }
        result
    }
    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        let isolated_root = self.isolation_depth == Some(message.depth);
        // The parent boundary observes an isolated creation once, before expectation rewriting.
        if !isolated_root && let Some(coverage) = &mut self.line_coverage {
            coverage.create_end(interp, message, result);
        }
        Self::create_end(self, interp, message, result);
        if message.depth == 0 && !result.stop.is_fatal() {
            let host = interp.host();
            if let Err(error) = self.forks.finalize_inactive(host.spec_id(), host.version()) {
                let _ = interp.fail(error);
                result.stop = InstrStop::FatalExternalError;
            }
        }
        if !isolated_root && let Some(tracing) = &mut self.tracing {
            tracing.create_end(interp, message, result);
        }
    }
    fn log(&mut self, log: &Log, host: &mut evm2::Evm<'_, FoundryEvmTypes>) {
        Self::log(self, log, host);
        if let Some(tracing) = &mut self.tracing {
            <TracingInspector as Inspector<FoundryEvmTypes>>::log(tracing, log, host);
        }
    }
}

fn unsupported_call(call: Vm::VmCalls) -> Result<Bytes, ApplyError> {
    const fn signature<C: crate::CheatcodeDef>(_: &C) -> &'static str {
        C::CHEATCODE.func.signature
    }
    macro_rules! unsupported {
        ($($variant:ident),*) => {
            match call {
                $(Vm::VmCalls::$variant(cheat) => Err(Error::from(format!(
                    "{} has not been migrated to EVM2", signature(&cheat)
                )).into()),)*
            }
        };
    }
    vm_calls!(unsupported)
}
