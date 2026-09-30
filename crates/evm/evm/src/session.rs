//! Foundry's between-transaction execution boundary, initially backed by the Ethereum executor.
//!
//! A session owns accepted state, environment, and retained inspector/cheatcode state. Execution
//! stages a successor; its exclusive pending handle publishes that successor only on acceptance.
//! Reports and borrowed feedback cannot mutate engine state or commit to another session.
//!
//! Checkpoints capture an idle session, including its backing database. They are not the live
//! `vm.snapshotState` operation. Canonical replay, active fork switching, and reentrant hook
//! capabilities remain separate extensions; this module does not emulate them with idle resets.

use crate::{
    ethereum::{EthereumExecutor, EthereumInspectorStack},
    executors::ITest,
};
use alloy_primitives::{Address, B256, Bytes, Log, TxKind, U256};
use alloy_sol_types::SolCall;
use evm2::{
    TxResult,
    evm::{
        AccountChangeRef, AccountInfo, Database, EmptyDB, PendingState, StateChangeSink,
        StateChangeSource, StorageChange,
    },
};
use foundry_evm_core::{
    constants::{CALLER, CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT, MAGIC_ASSUME},
    decode::SkipReason,
};
use foundry_evm_coverage::HitMaps;
use std::convert::Infallible;

/// An inspected Foundry synthetic call or deployment, not a canonical network transaction.
#[derive(Clone, Debug)]
pub struct CallRequest {
    pub caller: Address,
    pub target: TxKind,
    pub input: Bytes,
    pub value: U256,
    /// Optional transaction gas limit; otherwise use the session's configured limit.
    pub gas_limit: Option<u64>,
    /// Block-number increment staged with this operation, discarded if the operation is discarded.
    pub block_delay: U256,
    /// Timestamp increment staged with this operation, discarded if the operation is discarded.
    pub time_delay: U256,
}

impl CallRequest {
    /// Uses the session's gas policy without delaying the environment.
    pub const fn new(caller: Address, target: TxKind, input: Bytes, value: U256) -> Self {
        Self {
            caller,
            target,
            input,
            value,
            gas_limit: None,
            block_delay: U256::ZERO,
            time_delay: U256::ZERO,
        }
    }
}

/// EVM completion, independent of Forge assertion or campaign acceptance policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStatus {
    Success,
    Revert,
    Halt,
}

/// Owned observations that remain available after accepting or discarding execution.
#[derive(Debug)]
pub struct CallReport {
    request: CallRequest,
    result: TxResult,
    logs: Vec<Log>,
    coverage: Option<HitMaps>,
    skip_reason: Option<SkipReason>,
}

impl CallReport {
    /// Returns the exact synthetic request that ran, including staged delays.
    pub const fn request(&self) -> &CallRequest {
        &self.request
    }

    /// Returns engine completion; success does not imply that Forge assertions passed.
    pub const fn status(&self) -> CallStatus {
        if self.result.status {
            CallStatus::Success
        } else if self.result.stop.is_revert() {
            CallStatus::Revert
        } else {
            CallStatus::Halt
        }
    }

    /// Returns raw return or revert data.
    pub const fn output(&self) -> &Bytes {
        &self.result.output
    }

    /// Returns the engine's diagnostic stop name without exposing its result type.
    pub fn exit_reason(&self) -> String {
        format!("{:?}", self.result.stop)
    }

    /// Returns gas used after refunds and the transaction gas floor.
    pub const fn gas_used(&self) -> u64 {
        self.result.tx_gas_used()
    }

    /// Returns the address of a successful deployment.
    pub const fn created_address(&self) -> Option<Address> {
        self.result.created_address
    }

    /// Returns this operation's diagnostic logs, including logs from reverted frames.
    pub fn logs(&self) -> &[Log] {
        &self.logs
    }

    /// Takes this operation's diagnostic logs.
    pub fn take_logs(&mut self) -> Vec<Log> {
        std::mem::take(&mut self.logs)
    }

    /// Takes this operation's enabled line coverage.
    pub fn take_coverage(&mut self) -> Option<HitMaps> {
        self.coverage.take()
    }

    /// Returns an authenticated `vm.skip` reason, not a spoofed matching revert payload.
    pub const fn skip_reason(&self) -> Option<&SkipReason> {
        self.skip_reason.as_ref()
    }

    /// Recognizes Foundry's existing assumption-rejection marker.
    pub fn assumption_rejected(&self) -> bool {
        self.status() == CallStatus::Revert && self.output().as_ref() == MAGIC_ASSUME
    }
}

/// Assertion evidence; the workflow decides whether a pre-existing global failure is relevant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TestFacts {
    /// The prospective accepted state contains the global failure flag.
    pub global_failure: bool,
    /// This call loaded a nonzero failure slot, including a loaded-but-unchanged slot.
    pub call_global_failure: bool,
    /// A speculative legacy `failed()` query returned true. Unsupported probes count as false.
    pub legacy_failure: bool,
}

/// Account information observed by a call. Bytecode is resolved separately by its hash.
#[derive(Clone, Copy, Debug)]
pub struct AccountObservation {
    pub address: Address,
    /// Absent for a deleted or nonexistent account.
    pub code_hash: Option<B256>,
    pub created: bool,
    pub selfdestructed: bool,
}

/// A loaded storage slot, including reads that did not change its value.
#[derive(Clone, Copy, Debug)]
pub struct StorageObservation {
    pub address: Address,
    pub key: U256,
    pub original: U256,
    pub current: U256,
}

/// Read-only state feedback for dictionaries and contract discovery.
///
/// Account order is unspecified. This does not expose REVM's touched-account flag or promise
/// equivalent dynamic-target filtering; that policy also needs call/create observations.
#[derive(Clone, Copy, Debug)]
pub enum StateObservation {
    Account(AccountObservation),
    Storage(StorageObservation),
}

/// The single owner of a worker's accepted execution state.
#[derive(Debug)]
pub struct ExecutionSession<D: Database + Clone = EmptyDB> {
    executor: EthereumExecutor<D, EthereumInspectorStack>,
}

/// An opaque idle-state checkpoint with its matching backend, environment, and cheatcodes.
#[derive(Clone, Debug)]
pub struct SessionCheckpoint<D: Database + Clone = EmptyDB> {
    executor: EthereumExecutor<D, EthereumInspectorStack>,
}

impl<D: Database + Clone + 'static> ExecutionSession<D> {
    /// Transfers ownership from the resolved Ethereum construction path into a shared session.
    /// Collect setup observations before transfer; per-call buffers start empty in the session.
    pub fn from_ethereum(mut executor: EthereumExecutor<D, EthereumInspectorStack>) -> Self {
        let _ = executor.inspector_mut().take_logs();
        let _ = executor.inspector_mut().take_line_coverage();
        let _ = executor.inspector_mut().cheatcodes_mut().take_skip_payloads();
        Self { executor }
    }

    /// Captures an idle baseline; campaign dictionaries and generator RNG are caller-owned.
    pub fn checkpoint(&self) -> SessionCheckpoint<D> {
        SessionCheckpoint { executor: self.executor.clone() }
    }

    /// Restores all retained execution components together, including the backing database.
    pub fn restore(&mut self, checkpoint: &SessionCheckpoint<D>) {
        self.executor = checkpoint.executor.clone();
    }

    /// Reads an accepted balance, for example before campaign-specific value clamping.
    pub fn balance(&mut self, address: Address) -> eyre::Result<U256> {
        Ok(Database::get_account(self.executor.state_mut(), &address)?
            .map_or(U256::ZERO, |info| info.balance))
    }

    /// Reads accepted storage without lending the mutable engine state to the caller.
    pub fn storage(&mut self, address: Address, key: U256) -> eyre::Result<U256> {
        Ok(Database::get_storage(self.executor.state_mut(), &address, &key)?)
    }

    /// Reads an accepted account's nonce, treating an absent account as empty.
    pub fn nonce(&mut self, address: Address) -> eyre::Result<u64> {
        Ok(Database::get_account(self.executor.state_mut(), &address)?.map_or(0, |info| info.nonce))
    }

    /// Returns the accepted block number.
    pub fn block_number(&self) -> U256 {
        self.executor.env().block.number
    }

    /// Returns the accepted block timestamp.
    pub fn block_timestamp(&self) -> U256 {
        self.executor.env().block.timestamp
    }

    /// Stages execution without publishing its effects. Errors leave the accepted session intact.
    ///
    /// A pending operation excludes concurrent execution and checkpoint restoration:
    ///
    /// ```compile_fail
    /// use foundry_evm::session::{CallRequest, ExecutionSession};
    /// fn cannot_restore(session: &mut ExecutionSession, request: CallRequest) -> eyre::Result<()> {
    ///     let baseline = session.checkpoint();
    ///     let pending = session.execute(request)?;
    ///     session.restore(&baseline);
    ///     let _ = pending.accept();
    ///     Ok(())
    /// }
    /// ```
    pub fn execute(&mut self, request: CallRequest) -> eyre::Result<PendingCall<'_, D>> {
        let mut candidate = self.executor.clone();
        candidate.env_mut().block.number += request.block_delay;
        candidate.env_mut().block.timestamp += request.time_delay;
        let outcome = candidate.transact_raw_with_state(
            request.caller,
            request.target,
            request.input.clone(),
            request.value,
            request.gas_limit,
        )?;
        let inspector = candidate.inspector_mut();
        let logs = inspector.take_logs();
        let coverage = inspector.take_line_coverage();
        let skips = inspector.cheatcodes_mut().take_skip_payloads();
        let skip_reason = (!outcome.result.status && skips.contains(&outcome.result.output))
            .then(|| SkipReason::decode(&outcome.result.output))
            .flatten();
        let report = CallReport { request, result: outcome.result, logs, coverage, skip_reason };
        Ok(PendingCall { session: self, candidate, state: outcome.pending_state, report })
    }
}

impl<D: Database + Clone + 'static> SessionCheckpoint<D> {
    /// Creates an independent worker session over this baseline.
    pub fn spawn(&self) -> ExecutionSession<D> {
        ExecutionSession { executor: self.executor.clone() }
    }
}

/// A speculative successor exclusively borrowing its originating session.
///
/// Dropping the handle discards it. Observations cannot be used as a later commit token.
/// Filesystem and other external cheatcode side effects are outside session rollback.
#[must_use = "accept or discard the pending call; dropping it discards its effects"]
pub struct PendingCall<'session, D: Database + Clone = EmptyDB> {
    session: &'session mut ExecutionSession<D>,
    candidate: EthereumExecutor<D, EthereumInspectorStack>,
    state: PendingState,
    report: CallReport,
}

impl<D: Database + Clone + 'static> PendingCall<'_, D> {
    /// Returns immutable observations before deciding whether to accept this call.
    pub const fn report(&self) -> &CallReport {
        &self.report
    }

    /// Visits loaded state without exposing mutation or engine-state ownership.
    pub fn visit_state(&self, visitor: impl FnMut(StateObservation)) {
        let Ok(()) = self.state.visit(&mut Feedback(visitor));
    }

    /// Resolves observed code against the prospective state and its matching backing database.
    pub fn bytecode(&mut self, code_hash: B256) -> eyre::Result<Bytes> {
        Ok(Database::get_code_by_hash(self.candidate.state_mut(), &code_hash)?.original_bytes())
    }

    /// Inspects assertion facts without accepting execution or contaminating its observations.
    /// Unsupported live snapshot failure bookkeeping is not represented by these facts.
    pub fn test_facts(
        &mut self,
        target: Address,
        legacy_assertions: bool,
    ) -> eyre::Result<TestFacts> {
        let global_failure = !Database::get_storage(
            self.candidate.state_mut(),
            &CHEATCODE_ADDRESS,
            &GLOBAL_FAIL_SLOT,
        )?
        .is_zero();
        let mut call_global_failure = false;
        self.visit_state(|observation| {
            if let StateObservation::Storage(slot) = observation
                && slot.address == CHEATCODE_ADDRESS
                && slot.key == GLOBAL_FAIL_SLOT
            {
                call_global_failure = !slot.current.is_zero();
            }
        });
        let legacy_failure = if legacy_assertions {
            self.candidate
                .call_raw(
                    CALLER,
                    target,
                    Bytes::from_static(&ITest::failedCall::SELECTOR),
                    U256::ZERO,
                )
                .ok()
                .filter(|probe| probe.status && probe.output.len() == 32)
                .is_some_and(|probe| !U256::from_be_slice(&probe.output).is_zero())
        } else {
            false
        };
        Ok(TestFacts { global_failure, call_global_failure, legacy_failure })
    }

    /// Publishes state, environment, and retained cheatcodes together, even after an EVM revert.
    /// The caller owns assumption, assertion, and failure policy.
    pub fn accept(self) -> CallReport {
        self.session.executor = self.candidate;
        self.report
    }

    /// Discards session-local effects while retaining owned observations for reporting.
    pub fn discard(self) -> CallReport {
        self.report
    }
}

struct Feedback<F>(F);

impl<F: FnMut(StateObservation)> StateChangeSink for Feedback<F> {
    type Error = Infallible;

    fn account(&mut self, change: AccountChangeRef<'_>) -> Result<(), Self::Error> {
        (self.0)(StateObservation::Account(AccountObservation {
            address: change.address,
            code_hash: change.current.map(|info| info.code_hash),
            created: change.created,
            selfdestructed: change.selfdestructed,
        }));
        Ok(())
    }

    fn account_read(
        &mut self,
        address: Address,
        info: Option<&AccountInfo>,
    ) -> Result<(), Self::Error> {
        (self.0)(StateObservation::Account(AccountObservation {
            address,
            code_hash: info.map(|info| info.code_hash),
            created: false,
            selfdestructed: false,
        }));
        Ok(())
    }

    fn storage(&mut self, change: StorageChange) -> Result<(), Self::Error> {
        (self.0)(StateObservation::Storage(StorageObservation {
            address: change.address,
            key: change.key,
            original: change.original,
            current: change.current,
        }));
        Ok(())
    }

    fn storage_read(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), Self::Error> {
        (self.0)(StateObservation::Storage(StorageObservation {
            address,
            key,
            original: value,
            current: value,
        }));
        Ok(())
    }
}
