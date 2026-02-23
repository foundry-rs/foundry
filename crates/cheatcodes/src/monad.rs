//! Monad staking cheatcodes — separate address, separate dispatch.
//!
//! These cheatcodes live at [`MONAD_CHEATCODE_ADDRESS`] (distinct from the standard Foundry
//! `CHEATCODE_ADDRESS`). They provide two categories of functionality:
//!
//! 1. **Direct storage manipulation**: `setEpoch`, `setProposer`, `setAccumulator` — write directly
//!    to the staking precompile's storage at `0x1000`.
//!
//! 2. **Syscall wrappers**: `blockReward`, `epochSnapshot`, `epochChange`, `epochBoundary` —
//!    delegate to the real monad-revm syscall handlers via the [`StakingStorage`] adapter, giving
//!    production-equivalent behavior with zero logic duplication.
//!
//! State-mutating staking functions (delegate, undelegate, addValidator, etc.) are
//! handled by the staking precompile directly.

use crate::{CheatsCtxt, Result};
use alloy_primitives::{Address, U256, address};
use alloy_sol_types::SolInterface;
use foundry_evm_core::ContextExt;
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
use revm::{precompile::PrecompileError, primitives::Log};

// ---------------------------------------------------------------------------
// Address & ABI
// ---------------------------------------------------------------------------

/// Monad cheatcode address: `keccak256("monad cheatcode")[12..]`.
pub const MONAD_CHEATCODE_ADDRESS: Address = address!("0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA");

alloy_sol_types::sol! {
    /// Monad-specific cheatcodes. Accessible via `MonadVm(MONAD_CHEATCODE_ADDRESS)`.
    ///
    /// State-mutating staking functions (delegate, undelegate, addValidator, etc.)
    /// are handled by the staking precompile directly. These cheatcodes cover
    /// syscall simulation and direct state control for testing.
    interface MonadVm {
        /// Sets the current epoch and delay period for the staking precompile.
        function setEpoch(uint64 epoch, bool inDelayPeriod) external;

        /// Sets the current block proposer validator ID.
        function setProposer(uint64 valId) external;

        /// Directly sets a validator's accumulated reward per token.
        function setAccumulator(uint64 valId, uint256 value) external;

        /// Distribute block reward via the real syscallReward handler.
        /// Mints `reward` to staking address and distributes via accumulator math
        /// using consensus/snapshot view stake (production-equivalent behavior).
        function blockReward(address author, uint256 reward) external;

        /// Execute syscallSnapshot: copies consensus→snapshot view, rebuilds
        /// consensus set from execution set sorted by stake. Sets in_boundary = true.
        function epochSnapshot() external;

        /// Execute syscallOnEpochChange: increments epoch, clears in_boundary.
        /// `newEpoch` must be strictly greater than the current epoch.
        function epochChange(uint64 newEpoch) external;

        /// Convenience: epochSnapshot() then epochChange(newEpoch).
        function epochBoundary(uint64 newEpoch) external;
    }
}

// ---------------------------------------------------------------------------
// Dispatch entry point (called from inspector.rs)
// ---------------------------------------------------------------------------

/// Decode calldata and dispatch to the appropriate monad cheatcode handler.
pub fn apply_monad_cheatcode(ccx: &mut CheatsCtxt, input: &[u8]) -> Result {
    let decoded = MonadVm::MonadVmCalls::abi_decode(input).map_err(|e| {
        if let alloy_sol_types::Error::UnknownSelector { selector, .. } = e {
            let msg = format!(
                "unknown monad cheatcode with selector {selector}; \
                 check that your Monad.sol interface matches this forge version"
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

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

/// Encode a u64 left-aligned in a 32-byte slot (big-endian in first 8 bytes).
fn u64_left_aligned(v: u64) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&v.to_be_bytes());
    U256::from_be_bytes(bytes)
}

// ---------------------------------------------------------------------------
// Storage access helpers (bypass precompile check)
// ---------------------------------------------------------------------------

fn sstore_staking(ccx: &mut CheatsCtxt, key: U256, value: U256) -> Result<()> {
    let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
    journal.load_account(db, STAKING_ADDRESS)?;
    journal.touch(STAKING_ADDRESS);
    journal
        .sstore(db, STAKING_ADDRESS, key, value, false)
        .map_err(|e| fmt_err!("staking sstore failed: {:?}", e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// StakingStorage adapter for CheatsCtxt
// ---------------------------------------------------------------------------

/// Bridges Foundry's [`CheatsCtxt`] to monad-revm's [`StakingStorage`] trait,
/// enabling syscall handlers to read/write staking storage via the journal.
struct CheatsCtxtStorage<'a, 'cheats, 'evm, 'db, 'db2> {
    ccx: &'a mut CheatsCtxt<'cheats, 'evm, 'db, 'db2>,
}

impl StorageReader for CheatsCtxtStorage<'_, '_, '_, '_, '_> {
    fn sload(&mut self, key: U256) -> core::result::Result<U256, PrecompileError> {
        let (db, journal, _) = self.ccx.ecx.as_db_env_and_journal();
        journal
            .load_account(db, STAKING_ADDRESS)
            .map_err(|e| PrecompileError::Other(format!("load_account failed: {e:?}").into()))?;
        journal
            .sload(db, STAKING_ADDRESS, key, false)
            .map(|r| r.data)
            .map_err(|e| PrecompileError::Other(format!("sload failed: {e:?}").into()))
    }
}

impl StakingStorage for CheatsCtxtStorage<'_, '_, '_, '_, '_> {
    fn sstore(&mut self, key: U256, value: U256) -> core::result::Result<(), PrecompileError> {
        let (db, journal, _) = self.ccx.ecx.as_db_env_and_journal();
        journal
            .load_account(db, STAKING_ADDRESS)
            .map_err(|e| PrecompileError::Other(format!("load_account failed: {e:?}").into()))?;
        journal.touch(STAKING_ADDRESS);
        journal
            .sstore(db, STAKING_ADDRESS, key, value, false)
            .map(|_| ())
            .map_err(|e| PrecompileError::Other(format!("sstore failed: {e:?}").into()))
    }

    fn transfer(
        &mut self,
        from: Address,
        to: Address,
        amount: U256,
    ) -> core::result::Result<(), PrecompileError> {
        if amount.is_zero() {
            return Ok(());
        }
        let (db, journal, _) = self.ccx.ecx.as_db_env_and_journal();
        journal
            .transfer(db, from, to, amount)
            .map_err(|e| PrecompileError::Other(format!("transfer error: {e:?}").into()))?
            .map_or(Ok(()), |te| {
                Err(PrecompileError::Other(format!("transfer failed: {te:?}").into()))
            })
    }

    fn emit_log(&mut self, log: Log) -> core::result::Result<(), PrecompileError> {
        let (_, journal, _) = self.ccx.ecx.as_db_env_and_journal();
        journal.log(log);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Direct storage cheatcode handlers
// ---------------------------------------------------------------------------

fn apply_set_epoch(ccx: &mut CheatsCtxt, call: MonadVm::setEpochCall) -> Result {
    let MonadVm::setEpochCall { epoch, inDelayPeriod } = call;
    sstore_staking(ccx, global_slots::EPOCH, u64_left_aligned(epoch))?;
    // IN_BOUNDARY is a left-aligned bool (byte 0 = 1 for true, 0 for false)
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

fn apply_set_proposer(ccx: &mut CheatsCtxt, call: MonadVm::setProposerCall) -> Result {
    let MonadVm::setProposerCall { valId } = call;
    sstore_staking(ccx, global_slots::PROPOSER_VAL_ID, u64_left_aligned(valId))?;
    Ok(Default::default())
}

fn apply_set_accumulator(ccx: &mut CheatsCtxt, call: MonadVm::setAccumulatorCall) -> Result {
    let MonadVm::setAccumulatorCall { valId, value } = call;
    sstore_staking(
        ccx,
        validator_key(valId, validator_offsets::ACCUMULATED_REWARD_PER_TOKEN),
        value,
    )?;
    Ok(Default::default())
}

// ---------------------------------------------------------------------------
// Syscall cheatcode handlers
// ---------------------------------------------------------------------------

fn apply_block_reward(ccx: &mut CheatsCtxt, call: MonadVm::blockRewardCall) -> Result {
    let MonadVm::blockRewardCall { author, reward } = call;

    // Build extended calldata (68 bytes: selector + author + reward)
    let calldata = syscall_reward_calldata(author, reward);

    // Run syscall first — it will revert for unknown authors or zero-stake validators.
    // Mint only after success to avoid crediting tokens on revert.
    let mut storage = CheatsCtxtStorage { ccx };
    handle_syscall_reward(&mut storage, &calldata, u64::MAX, &SYSTEM_ADDRESS, U256::ZERO)
        .map_err(|e| fmt_err!("blockReward failed: {e}"))?;

    // Mint reward to STAKING_ADDRESS balance (token minting — not a transfer)
    {
        let (db, journal, _) = storage.ccx.ecx.as_db_env_and_journal();
        journal.load_account(db, STAKING_ADDRESS)?;
        journal.touch(STAKING_ADDRESS);
        let account = journal.state.get_mut(&STAKING_ADDRESS).expect("staking account loaded");
        account.info.balance = account.info.balance.saturating_add(reward);
    }

    Ok(Default::default())
}

fn apply_epoch_snapshot(ccx: &mut CheatsCtxt, _call: MonadVm::epochSnapshotCall) -> Result {
    let calldata = syscall_snapshot_calldata();

    let mut storage = CheatsCtxtStorage { ccx };
    handle_syscall_snapshot(&mut storage, &calldata, u64::MAX, &SYSTEM_ADDRESS)
        .map_err(|e| fmt_err!("epochSnapshot failed: {e}"))?;

    Ok(Default::default())
}

fn apply_epoch_change(ccx: &mut CheatsCtxt, call: MonadVm::epochChangeCall) -> Result {
    let calldata = syscall_on_epoch_change_calldata(call.newEpoch);

    let mut storage = CheatsCtxtStorage { ccx };
    handle_syscall_on_epoch_change(&mut storage, &calldata, u64::MAX, &SYSTEM_ADDRESS)
        .map_err(|e| fmt_err!("epochChange failed: {e}"))?;

    Ok(Default::default())
}

fn apply_epoch_boundary(ccx: &mut CheatsCtxt, call: MonadVm::epochBoundaryCall) -> Result {
    apply_epoch_snapshot(ccx, MonadVm::epochSnapshotCall {})?;
    apply_epoch_change(ccx, MonadVm::epochChangeCall { newEpoch: call.newEpoch })?;
    Ok(Default::default())
}
