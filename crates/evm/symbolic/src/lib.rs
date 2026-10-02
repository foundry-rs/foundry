//! Foundry's symbolic EVM executor.

#![warn(unused_crate_dependencies)]

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, B256, Bytes, I256, U256, hex, keccak256,
    map::{HashMap, HashSet, IndexSet},
};
use alloy_signer::SignerSync;
use alloy_signer_local::{
    PrivateKeySigner,
    coins_bip39::{English, Wordlist},
};
use alloy_sol_types::SolCall;
use base64::prelude::*;
use foundry_cheatcodes_spec::{SymbolicVm, Vm};
use foundry_config::{SymbolicConfig, SymbolicExplorationOrder, SymbolicStorageLayout};
use foundry_evm::{
    constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS},
    core::{backend::DatabaseExt, evm::FoundryEvmNetwork},
    executors::Executor,
    revm::{
        bytecode::{Bytecode, JumpTable, opcode},
        context::{Block, Cfg, Transaction},
        database::DatabaseRef,
        precompile::{blake2, bn254, hash, identity, kzg_point_evaluation, modexp, secp256k1},
        primitives::hardfork::SpecId,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fmt::{self, Write as _},
    io::Write,
    ops::{ControlFlow, Deref, DerefMut},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tracing::{debug, trace, trace_span, warn};

mod abi;
mod consts;
mod executor;
mod runtime;

pub(crate) use consts::*;
pub use runtime::{SymbolicBranchTarget, SymbolicError, SymbolicRunInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymbolicVmCheatcode {
    CreateAddress,
    CreateBool,
    CreateBytes,
    CreateBytesSized,
    CreateBytesFixed(usize),
    CreateCalldata,
    CreateInt,
    CreateIntBits(usize),
    CreateString,
    CreateStringSized,
    CreateUint,
    CreateUintBits(usize),
    EnableSymbolicStorage,
    SnapshotStorage,
    SnapshotState,
}

impl SymbolicVmCheatcode {
    fn from_selector(selector: [u8; 4]) -> Option<Self> {
        match selector {
            SymbolicVm::createAddressCall::SELECTOR => Some(Self::CreateAddress),
            SymbolicVm::createBoolCall::SELECTOR => Some(Self::CreateBool),
            SymbolicVm::createBytes_0Call::SELECTOR => Some(Self::CreateBytes),
            SymbolicVm::createBytes_1Call::SELECTOR => Some(Self::CreateBytesSized),
            SymbolicVm::createCalldataCall::SELECTOR => Some(Self::CreateCalldata),
            SymbolicVm::createIntCall::SELECTOR => Some(Self::CreateInt),
            SymbolicVm::createString_0Call::SELECTOR => Some(Self::CreateString),
            SymbolicVm::createString_1Call::SELECTOR => Some(Self::CreateStringSized),
            SymbolicVm::createUintCall::SELECTOR => Some(Self::CreateUint),
            SymbolicVm::enableSymbolicStorageCall::SELECTOR
            | Vm::setArbitraryStorage_0Call::SELECTOR => Some(Self::EnableSymbolicStorage),
            SymbolicVm::snapshotStorageCall::SELECTOR => Some(Self::SnapshotStorage),
            Vm::snapshotStateCall::SELECTOR => Some(Self::SnapshotState),
            _ => {
                let name = SymbolicVm::SymbolicVmCalls::name_by_selector(selector)?;
                if let Some(bits) = name.strip_prefix("createUint") {
                    bits.parse().ok().map(Self::CreateUintBits)
                } else if let Some(bits) = name.strip_prefix("createInt") {
                    bits.parse().ok().map(Self::CreateIntBits)
                } else if let Some(bytes) = name.strip_prefix("createBytes") {
                    bytes.parse().ok().map(Self::CreateBytesFixed)
                } else {
                    None
                }
            }
        }
    }

    const fn min_input_words(self) -> usize {
        match self {
            Self::CreateUint
            | Self::CreateInt
            | Self::CreateBytesSized
            | Self::CreateStringSized
            | Self::EnableSymbolicStorage
            | Self::SnapshotStorage => 1,
            Self::CreateAddress
            | Self::CreateBool
            | Self::CreateBytes
            | Self::CreateBytesFixed(_)
            | Self::CreateCalldata
            | Self::CreateIntBits(_)
            | Self::CreateString
            | Self::CreateUintBits(_)
            | Self::SnapshotState => 0,
        }
    }
}

/// Outcome of a symbolic test execution.
///
/// The forge runner treats `Safe` as a passing symbolic test, `Counterexample` as a
/// candidate failure that must be replayed concretely, and `Incomplete` as a failing
/// test because the symbolic engine could not prove the property with the supported
/// semantics and configured resource limits.
#[derive(Clone, Debug)]
pub enum SymbolicRunResult {
    /// All explored paths completed without a feasible failure.
    Safe {
        /// Execution counters collected during the run.
        stats: SymbolicStats,
        /// One concrete successful input, when requested by the caller.
        success_input: Option<SymbolicConcreteInput>,
    },
    /// A feasible failure was found.
    Counterexample {
        /// ABI-typed argument values extracted from the solver model.
        args: Vec<DynSolValue>,
        /// ABI-encoded calldata for the failing invocation.
        calldata: Bytes,
        /// Execution counters collected before the counterexample was returned.
        stats: SymbolicStats,
    },
    /// Execution was intentionally stopped because V1 semantics were insufficient.
    Incomplete {
        /// Category describing why symbolic execution stopped before proving the test.
        kind: SymbolicStopReason,
        /// Human-readable explanation of the unsupported construct or exhausted limit.
        reason: String,
        /// Execution counters collected before execution stopped.
        stats: SymbolicStats,
    },
}

/// One concrete symbolic input materialized from a solver model.
#[derive(Clone, Debug)]
pub struct SymbolicConcreteInput {
    /// ABI-typed argument values extracted from the solver model.
    pub args: Vec<DynSolValue>,
    /// ABI-encoded calldata for replay.
    pub calldata: Bytes,
}

/// Result of best-effort symbolic exploration toward one branch target.
#[derive(Debug)]
pub struct SymbolicBranchTargetSearchResult {
    /// Concrete inputs whose completed root path reached the requested branch outcome.
    pub candidates: Vec<SymbolicConcreteInput>,
    /// Underlying execution result, retained so callers can report incomplete exploration.
    pub execution: SymbolicRunResult,
}

/// A concrete invariant target selected from Foundry's invariant discovery.
#[derive(Clone, Debug)]
pub struct SymbolicInvariantTarget {
    /// Address that receives the sequence call.
    pub address: Address,
    /// Human-readable contract identifier used in counterexample rendering.
    pub contract_name: Option<String>,
    /// ABI function invoked with symbolic arguments.
    pub function: Function,
}

/// Input for best-effort invariant candidate search after one symbolic handler call.
pub struct SymbolicInvariantCandidateInput<'a, FEN: FoundryEvmNetwork> {
    /// Concrete Foundry executor containing the replayed invariant frontier prefix.
    pub executor: &'a Executor<FEN>,
    /// Address of the deployed invariant test contract.
    pub invariant_address: Address,
    /// Invariant functions checked independently after the handler call.
    pub invariants: &'a [&'a Function],
    /// Optional campaign hook checked from the unchanged post-handler state.
    pub after_invariant: Option<&'a Function>,
    /// Concrete handler target selected from the captured frontier.
    pub target: &'a SymbolicInvariantTarget,
    /// Sender of the captured handler call.
    pub handler_sender: Address,
    /// Whether symbolic `vm.ffi` calls are allowed to execute subprocesses.
    pub ffi_enabled: bool,
}

/// One unconfirmed symbolic input produced by invariant candidate search.
#[derive(Clone, Debug)]
pub struct SymbolicInvariantCandidate {
    /// Index within [`SymbolicInvariantCandidateInput::invariants`] predicted to fail.
    pub invariant_idx: usize,
    /// Concrete handler call extracted from the solver model.
    pub step: SymbolicInvariantStep,
    /// Concrete setup-storage values needed to replay the candidate.
    pub storage: Vec<SymbolicStorageAssignment>,
}

/// An execution or solver limitation encountered during best-effort candidate search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolicInvariantSearchLimitation {
    /// Category describing why part of the search could not complete.
    pub kind: SymbolicStopReason,
    /// Human-readable description of the limitation.
    pub reason: String,
}

impl From<SymbolicError> for SymbolicInvariantSearchLimitation {
    fn from(error: SymbolicError) -> Self {
        Self { kind: error.stop_reason(), reason: error.to_string() }
    }
}

/// Result of best-effort invariant candidate search after one symbolic handler call.
#[derive(Clone, Debug)]
pub struct SymbolicInvariantCandidateSearchResult {
    /// Unconfirmed candidates that must be replayed concretely by the caller.
    pub candidates: Vec<SymbolicInvariantCandidate>,
    /// First encountered search limitation, unless a later error exhausts the search.
    /// `None` is not a proof of safety.
    pub limitation: Option<SymbolicInvariantSearchLimitation>,
}

/// Input for bounded symbolic invariant execution.
pub struct SymbolicInvariantRunInput<'a, FEN: FoundryEvmNetwork> {
    /// Concrete Foundry executor used as the source of deployed bytecode and backend state.
    pub executor: &'a Executor<FEN>,
    /// Address of the deployed invariant test contract.
    pub invariant_address: Address,
    /// Default sender used when invariant targeting does not configure senders.
    pub sender: Address,
    /// Invariant function checked after each symbolic sequence step.
    pub invariant: &'a Function,
    /// Optional `afterInvariant` hook to execute after a passing invariant check.
    pub after_invariant: Option<&'a Function>,
    /// Concrete target/selector set discovered by Foundry invariant targeting.
    pub targets: Vec<SymbolicInvariantTarget>,
    /// Concrete sender set discovered by Foundry invariant targeting.
    pub senders: Vec<Address>,
    /// Sender addresses excluded by Foundry invariant targeting.
    pub excluded_senders: Vec<Address>,
    /// Maximum number of sequence calls to execute.
    pub depth: usize,
    /// Concrete invariant check interval. `0` means only check at sequence end.
    pub check_interval: u32,
    /// Whether ordinary target-call reverts should be reported as failures.
    pub fail_on_revert: bool,
    /// Whether symbolic `vm.ffi` calls are allowed to execute subprocesses.
    pub ffi_enabled: bool,
}

/// One concrete storage value required to replay a symbolic invariant candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicStorageAssignment {
    /// Account whose storage slot should be initialized.
    pub address: Address,
    /// Concrete storage slot.
    pub slot: U256,
    /// Concrete value extracted from the solver model.
    pub value: U256,
}

/// Outcome of bounded symbolic invariant execution.
#[derive(Clone, Debug)]
pub enum SymbolicInvariantRunResult {
    /// No feasible invariant failure was found within the configured sequence depth.
    Safe(SymbolicStats),
    /// A feasible invariant or handler failure was found.
    Counterexample {
        /// Which part of the invariant run produced the failure.
        kind: SymbolicInvariantCounterexampleKind,
        /// Concrete sequence extracted from the solver model.
        sequence: Vec<SymbolicInvariantStep>,
        /// Concrete setup-storage values needed for replay.
        storage: Vec<SymbolicStorageAssignment>,
        /// Execution counters collected before the counterexample was returned.
        stats: SymbolicStats,
    },
    /// Execution stopped before proving the invariant.
    Incomplete {
        /// Category describing why symbolic execution stopped.
        kind: SymbolicStopReason,
        /// Human-readable explanation of the unsupported construct or exhausted limit.
        reason: String,
        /// Execution counters collected before execution stopped.
        stats: SymbolicStats,
    },
}

/// Part of a symbolic invariant run that produced a replayable counterexample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolicInvariantCounterexampleKind {
    /// An `invariant_*` or `afterInvariant` check failed.
    Predicate,
    /// A fuzzed target/handler call failed with an assertion.
    Handler,
}

/// One concrete step in a symbolic invariant counterexample sequence.
#[derive(Clone, Debug)]
pub struct SymbolicInvariantStep {
    /// Sender used for the call.
    pub sender: Address,
    /// Target address called by the sequence step.
    pub address: Address,
    /// Human-readable contract identifier, when known.
    pub contract_name: Option<String>,
    /// ABI function name.
    pub function_name: String,
    /// ABI function signature.
    pub signature: String,
    /// ABI-typed arguments extracted from the solver model.
    pub args: Vec<DynSolValue>,
    /// ABI-encoded calldata for replay.
    pub calldata: Bytes,
}

/// High-level reason a symbolic run stopped without a proof or replayed counterexample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolicStopReason {
    /// The executor reached a supported-but-incomplete semantic boundary.
    Stuck,
    /// Every explored execution path ended in an ordinary revert.
    RevertAll,
    /// The solver timed out or returned `unknown`.
    Timeout,
    /// An internal engine, backend, or solver process error occurred.
    Error,
}

/// Symbolic execution counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolicStats {
    /// Number of explored symbolic paths.
    pub paths: usize,
    /// Number of normalized solver queries issued during the run.
    pub solver_queries: usize,
    /// Number of queries sent to the SMT backend after local fast paths.
    #[serde(default)]
    pub smt_queries: usize,
    /// Number of satisfiability checks requested by the executor.
    #[serde(default)]
    pub sat_queries: usize,
    /// Number of concrete model requests requested by the executor.
    #[serde(default)]
    pub model_queries: usize,
    /// Number of satisfiability checks served from the normalized cache.
    #[serde(default)]
    pub sat_cache_hits: usize,
    /// Number of model requests served from the normalized model cache.
    #[serde(default)]
    pub model_cache_hits: usize,
    /// Number of satisfiable witnesses produced by local hard-arithmetic search.
    #[serde(default)]
    pub heuristic_witnesses: usize,
    /// Wall-clock time spent waiting on backend solver subprocesses, in milliseconds.
    #[serde(default)]
    pub solver_time_ms: u64,
    /// Total SMT-LIB input bytes sent to backend solver subprocesses.
    #[serde(default)]
    pub smt_input_bytes: u64,
    /// Largest single SMT-LIB query input sent to a backend solver subprocess, in bytes.
    #[serde(default)]
    pub smt_max_query_bytes: u64,
    /// Wall-clock time spent building SMT-LIB query strings, in milliseconds.
    #[serde(default)]
    pub smt_build_time_ms: u64,
    /// Longest single backend solver subprocess query, in milliseconds.
    #[serde(default)]
    pub smt_max_query_time_ms: u64,
}

/// SMT-LIB-backed symbolic executor.
///
/// This executor is intentionally separate from the concrete revm executor used by
/// Foundry. It consumes bytecode and state from an existing [`Executor`], explores
/// symbolic branches, and returns either a proof result, a counterexample candidate,
/// or an incomplete result.
pub struct SymbolicExecutor {
    config: SymbolicConfig,
    cx: runtime::SymCx,
    solver: runtime::SmtLibSubprocessSolver,
    deferred_incomplete: Option<DeferredIncomplete>,
    deadline: Option<Instant>,
    nested_deferred_mode: DeferredPathMode,
    stateless_retry_safe: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeferredPathMode {
    Skip,
    Yield,
    Drain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeferredIncomplete {
    Unsupported(&'static str),
    SolverUnknown,
    HardArithmetic,
}
