//! Monad staking cheatcodes.

use crate::{CheatsCtxt, Result};
use alloy_primitives::{Address, U256};
use alloy_sol_types::SolInterface;
use foundry_evm_core::{
    constants::MONAD_CHEATCODE_ADDRESS,
    evm::{FoundryEvmNetwork, MonadEvmNetwork},
};
use monad_revm::{
    api::block::{
        syscall_on_epoch_change_calldata, syscall_reward_calldata, syscall_snapshot_calldata,
    },
    staking::{
        StorageReader,
        constants::SYSTEM_ADDRESS,
        storage::{STAKING_ADDRESS, global_slots, validator_key, validator_offsets},
        write::{
            StakingStorage, handle_syscall_on_epoch_change, handle_syscall_reward,
            handle_syscall_snapshot,
        },
    },
};
use revm::{
    context::{ContextTr, JournalTr},
    precompile::PrecompileHalt,
    primitives::Log,
};
use std::any::TypeId;

alloy_sol_types::sol! {
    /// Monad-specific cheatcodes. Accessible via `MonadVm(MONAD_CHEATCODE_ADDRESS)`.
    interface MonadVm {
        /// Sets the current epoch and delay period for the staking precompile.
        function setEpoch(uint64 epoch, bool inDelayPeriod) external;

        /// Sets the current block proposer validator ID.
        function setProposer(uint64 valId) external;

        /// Directly sets a validator's accumulated reward per token.
        function setAccumulator(uint64 valId, uint256 value) external;

        /// Distribute block reward via the real syscallReward handler.
        function blockReward(address author, uint256 reward) external;

        /// Execute syscallSnapshot.
        function epochSnapshot() external;

        /// Execute syscallOnEpochChange.
        function epochChange(uint64 newEpoch) external;

        /// Convenience: epochSnapshot() then epochChange(newEpoch).
        function epochBoundary(uint64 newEpoch) external;
    }
}

pub(crate) fn is_monad_cheatcode_call<FEN: FoundryEvmNetwork>(target: Address) -> bool {
    target == MONAD_CHEATCODE_ADDRESS && TypeId::of::<FEN>() == TypeId::of::<MonadEvmNetwork>()
}

pub(crate) fn apply_monad_cheatcode<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    input: &[u8],
) -> Result {
    let decoded = MonadVm::MonadVmCalls::abi_decode(input).map_err(|e| {
        if let alloy_sol_types::Error::UnknownSelector { selector, .. } = e {
            let msg = format!(
                "unknown monad cheatcode with selector {selector}; \
                 check that your MonadVm interface matches this forge version"
            );
            return alloy_sol_types::Error::Other(std::borrow::Cow::Owned(msg));
        }
        e
    })?;

    match decoded {
        MonadVm::MonadVmCalls::setEpoch(call) => apply_set_epoch(ccx, call),
        MonadVm::MonadVmCalls::setProposer(call) => apply_set_proposer(ccx, call),
        MonadVm::MonadVmCalls::setAccumulator(call) => apply_set_accumulator(ccx, call),
        MonadVm::MonadVmCalls::blockReward(call) => apply_block_reward(ccx, call),
        MonadVm::MonadVmCalls::epochSnapshot(call) => apply_epoch_snapshot(ccx, call),
        MonadVm::MonadVmCalls::epochChange(call) => apply_epoch_change(ccx, call),
        MonadVm::MonadVmCalls::epochBoundary(call) => apply_epoch_boundary(ccx, call),
    }
}

fn u64_left_aligned(v: u64) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&v.to_be_bytes());
    U256::from_be_bytes(bytes)
}

fn sstore_staking<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    key: U256,
    value: U256,
) -> Result<()> {
    ccx.ecx.journal_mut().load_account(STAKING_ADDRESS)?;
    ccx.ecx
        .journal_mut()
        .sstore(STAKING_ADDRESS, key, value)
        .map_err(|e| fmt_err!("staking sstore failed: {:?}", e))?;
    Ok(())
}

struct CheatsCtxtStorage<'a, 'b, 'db, FEN: FoundryEvmNetwork> {
    ccx: &'a mut CheatsCtxt<'b, 'db, FEN>,
}

impl<FEN: FoundryEvmNetwork> StorageReader for CheatsCtxtStorage<'_, '_, '_, FEN> {
    fn sload(&mut self, key: U256) -> core::result::Result<U256, PrecompileHalt> {
        self.ccx
            .ecx
            .journal_mut()
            .sload(STAKING_ADDRESS, key)
            .map(|r| r.data)
            .map_err(|e| PrecompileHalt::Other(format!("sload failed: {e:?}").into()))
    }
}

impl<FEN: FoundryEvmNetwork> StakingStorage for CheatsCtxtStorage<'_, '_, '_, FEN> {
    fn sstore(&mut self, key: U256, value: U256) -> core::result::Result<(), PrecompileHalt> {
        self.ccx
            .ecx
            .journal_mut()
            .sstore(STAKING_ADDRESS, key, value)
            .map(|_| ())
            .map_err(|e| PrecompileHalt::Other(format!("sstore failed: {e:?}").into()))
    }

    fn transfer(
        &mut self,
        from: Address,
        to: Address,
        amount: U256,
    ) -> core::result::Result<(), PrecompileHalt> {
        if amount.is_zero() {
            return Ok(());
        }

        match self.ccx.ecx.journal_mut().transfer(from, to, amount) {
            Ok(None) => Ok(()),
            Ok(Some(e)) => Err(PrecompileHalt::Other(format!("transfer failed: {e:?}").into())),
            Err(e) => Err(PrecompileHalt::Other(format!("transfer error: {e:?}").into())),
        }
    }

    fn emit_log(&mut self, log: Log) -> core::result::Result<(), PrecompileHalt> {
        self.ccx.ecx.journal_mut().log(log);
        Ok(())
    }
}

fn apply_set_epoch<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    call: MonadVm::setEpochCall,
) -> Result {
    let MonadVm::setEpochCall { epoch, inDelayPeriod } = call;
    sstore_staking(ccx, global_slots::EPOCH, u64_left_aligned(epoch))?;

    let boundary_val = if inDelayPeriod {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        U256::from_be_bytes(bytes)
    } else {
        U256::ZERO
    };
    sstore_staking(ccx, global_slots::IN_BOUNDARY, boundary_val)?;
    Ok(Default::default())
}

fn apply_set_proposer<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    call: MonadVm::setProposerCall,
) -> Result {
    sstore_staking(ccx, global_slots::PROPOSER_VAL_ID, u64_left_aligned(call.valId))?;
    Ok(Default::default())
}

fn apply_set_accumulator<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    call: MonadVm::setAccumulatorCall,
) -> Result {
    sstore_staking(
        ccx,
        validator_key(call.valId, validator_offsets::ACCUMULATED_REWARD_PER_TOKEN),
        call.value,
    )?;
    Ok(Default::default())
}

fn apply_block_reward<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    call: MonadVm::blockRewardCall,
) -> Result {
    let MonadVm::blockRewardCall { author, reward } = call;
    let calldata = syscall_reward_calldata(author, reward);

    let mut storage = CheatsCtxtStorage { ccx };
    handle_syscall_reward(&mut storage, &calldata, u64::MAX, &SYSTEM_ADDRESS, U256::ZERO)
        .map_err(|e| fmt_err!("blockReward failed: {e}"))?;

    storage.ccx.ecx.journal_mut().load_account(STAKING_ADDRESS)?;
    let account =
        storage.ccx.ecx.journal_mut().evm_state_mut().get_mut(&STAKING_ADDRESS).expect("loaded");
    account.info.balance = account.info.balance.saturating_add(reward);

    Ok(Default::default())
}

fn apply_epoch_snapshot<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    _call: MonadVm::epochSnapshotCall,
) -> Result {
    let calldata = syscall_snapshot_calldata();
    let mut storage = CheatsCtxtStorage { ccx };
    handle_syscall_snapshot(&mut storage, &calldata, u64::MAX, &SYSTEM_ADDRESS)
        .map_err(|e| fmt_err!("epochSnapshot failed: {e}"))?;
    Ok(Default::default())
}

fn apply_epoch_change<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    call: MonadVm::epochChangeCall,
) -> Result {
    let calldata = syscall_on_epoch_change_calldata(call.newEpoch);
    let mut storage = CheatsCtxtStorage { ccx };
    handle_syscall_on_epoch_change(&mut storage, &calldata, u64::MAX, &SYSTEM_ADDRESS)
        .map_err(|e| fmt_err!("epochChange failed: {e}"))?;
    Ok(Default::default())
}

fn apply_epoch_boundary<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    call: MonadVm::epochBoundaryCall,
) -> Result {
    apply_epoch_snapshot(ccx, MonadVm::epochSnapshotCall {})?;
    apply_epoch_change(ccx, MonadVm::epochChangeCall { newEpoch: call.newEpoch })?;
    Ok(Default::default())
}
