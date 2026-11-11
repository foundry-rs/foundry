//! Foundry's symbolic EVM executor.

#![warn(unused_crate_dependencies)]

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi, StateMutability};
use alloy_network::TransactionBuilder;
use alloy_primitives::{
    Address, B256, Bytes, I256, Keccak256, U256, hex, keccak256,
    map::{HashMap, HashSet, IndexSet},
};
use alloy_signer::SignerSync;
use alloy_signer_local::{
    PrivateKeySigner,
    coins_bip39::{English, Wordlist},
};
use alloy_sol_types::{SolCall, SolInterface};
use base64::prelude::*;
use evm2::{
    EvmFeatures, SpecId,
    bytecode::{Bytecode, JumpTable},
    constants::{CALL_DEPTH_LIMIT, EIP7702_BYTECODE_LEN},
    interpreter::{GasTracker, op as opcode},
    precompiles::{
        Precompiles, blake2, bls12_381, bn254, hash, identity, kzg_point_evaluation, modexp,
        secp256k1, secp256r1,
    },
};
use foundry_cheatcodes_spec::{SymbolicVm, Vm};
use foundry_common::FoundryTransactionBuilder;
use foundry_config::{SymbolicConfig, SymbolicExplorationOrder, SymbolicStorageLayout};
use foundry_evm::{
    constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS},
    core::{abi::console, evm::FoundryEvmNetwork},
    executors::Executor,
    inspectors::cheatcodes::{decode_cheatcode_call, parse_value},
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

pub use consts::SYMBOLIC_VM_COMPAT_ADDRESS;
pub(crate) use consts::*;
pub use runtime::{ExecutorView, SymbolicBranchTarget, SymbolicError, SymbolicRunInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymbolicVmCheatcode {
    CreateAddress,
    CreateBool,
    CreateBytes,
    CreateBytesSized,
    CreateBytesFixed(usize),
    /// `createCalldata` for a contract named by `target`, with a trailing view and pure function
    /// flag when `view_flag` is set.
    CreateCalldata {
        target: CreateCalldataTarget,
        view_flag: bool,
    },
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
            SymbolicVm::createCalldata_0Call::SELECTOR => {
                Some(Self::CreateCalldata { target: CreateCalldataTarget::Name, view_flag: false })
            }
            SymbolicVm::createCalldata_1Call::SELECTOR => {
                Some(Self::CreateCalldata { target: CreateCalldataTarget::Name, view_flag: true })
            }
            SymbolicVm::createCalldata_2Call::SELECTOR => Some(Self::CreateCalldata {
                target: CreateCalldataTarget::FileAndName,
                view_flag: false,
            }),
            SymbolicVm::createCalldata_3Call::SELECTOR => Some(Self::CreateCalldata {
                target: CreateCalldataTarget::FileAndName,
                view_flag: true,
            }),
            SymbolicVm::createCalldata_4Call::SELECTOR => Some(Self::CreateCalldata {
                target: CreateCalldataTarget::Address,
                view_flag: false,
            }),
            SymbolicVm::createCalldata_5Call::SELECTOR => Some(Self::CreateCalldata {
                target: CreateCalldataTarget::Address,
                view_flag: true,
            }),
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
            Self::CreateCalldata { target, view_flag } => target.words() + view_flag as usize,
            Self::CreateAddress
            | Self::CreateBool
            | Self::CreateBytes
            | Self::CreateBytesFixed(_)
            | Self::CreateIntBits(_)
            | Self::CreateString
            | Self::CreateUintBits(_)
            | Self::SnapshotState => 0,
        }
    }
}

/// Returns the selectors that the symbolic engine answers as `svm.*` helpers at
/// [`SYMBOLIC_VM_COMPAT_ADDRESS`].
pub fn symbolic_vm_selectors() -> impl Iterator<Item = [u8; 4]> {
    SymbolicVm::SymbolicVmCalls::SELECTORS
        .iter()
        .copied()
        .chain([Vm::setArbitraryStorage_0Call::SELECTOR, Vm::snapshotStateCall::SELECTOR])
        .filter(|selector| SymbolicVmCheatcode::from_selector(*selector).is_some())
}

/// How a `createCalldata` call names its contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreateCalldataTarget {
    /// A deployed contract address.
    Address,
    /// A contract or interface name.
    Name,
    /// A source file name and a contract or interface name.
    FileAndName,
}

impl CreateCalldataTarget {
    /// Returns the number of ABI head words that name the contract.
    const fn words(self) -> usize {
        match self {
            Self::Address | Self::Name => 1,
            Self::FileAndName => 2,
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
        /// Return data of each `svm.*` helper call on the failing path, in call order.
        svm_returns: Vec<Bytes>,
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
pub struct SymbolicInvariantCandidateInput<'a> {
    /// Read-only view of the concrete Foundry executor containing the replayed invariant frontier
    /// prefix.
    pub executor: ExecutorView<'a>,
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
pub struct SymbolicInvariantRunInput<'a> {
    /// Read-only view of the concrete Foundry executor used as the source of deployed bytecode and
    /// backend state.
    pub executor: ExecutorView<'a>,
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

/// Input for a symbolic equivalence check of two runtime bytecodes.
pub struct SymbolicEquivalenceInput<'a> {
    /// Read-only view of the concrete Foundry executor that supplies the environment and the
    /// pre-state both codes start from.
    pub executor: ExecutorView<'a>,
    /// Address both codes run at, so they share one storage pre-state.
    pub address: Address,
    /// Runtime bytecode of the first contract.
    pub code_a: Bytes,
    /// Runtime bytecode of the second contract.
    pub code_b: Bytes,
    /// Account that stands for the sender of the top-level call, whose address is symbolic.
    pub sender: Address,
    /// Fixed call value of the top-level call; symbolic when `None`.
    pub value: Option<U256>,
    /// When set, calldata is limited to ABI-encoded calls of this function.
    pub function: Option<&'a Function>,
    /// Fixed lengths of raw calldata to explore when `function` is `None`. When empty, the
    /// calldata length is symbolic, up to `max_calldata` bytes.
    pub calldata_lengths: Vec<usize>,
    /// Largest symbolic calldata, in bytes.
    pub max_calldata: usize,
    /// Largest return data of a call into unknown code, in bytes.
    pub max_returndata: usize,
    /// Assume calls into unknown code never re-enter the programs, like hevm's
    /// `--promise-no-reent`. Without it a state-changing CALL into unknown code is incomplete.
    pub promise_no_reent: bool,
    /// Compare the programs modulo gas, like hevm: inputs on which either program runs out of
    /// memory gas are left out instead of counting as a difference.
    pub ignore_out_of_gas: bool,
}

/// Concrete transaction and block environment of an equivalence counterexample.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolicEquivalenceEnv {
    /// Address both programs run at.
    pub address: Address,
    /// Sender of the call.
    pub caller: Address,
    /// Origin of the transaction.
    pub origin: Address,
    /// Call value.
    pub value: U256,
    /// Block timestamp.
    pub timestamp: U256,
    /// Block number.
    pub number: U256,
    /// `PREVRANDAO` / `DIFFICULTY`.
    pub prevrandao: U256,
    /// Block gas limit.
    pub gas_limit: U256,
    /// Base fee.
    pub basefee: U256,
    /// Blob base fee.
    pub blob_basefee: U256,
    /// Chain ID.
    pub chain_id: U256,
    /// Block beneficiary.
    pub coinbase: Address,
    /// Gas price.
    pub gas_price: U256,
    /// Starting balances of the accounts the counterexample reads, keyed by the account that
    /// stands for them in the check.
    pub balances: Vec<(Address, U256)>,
}

/// Outcome of a symbolic equivalence check.
#[derive(Clone, Debug)]
pub enum SymbolicEquivalenceResult {
    /// Every explored pair of paths agreed on success, output and storage.
    Equivalent {
        /// Execution counters collected during the run.
        stats: SymbolicStats,
    },
    /// The solver found an input on which the two codes differ. Callers must replay it
    /// concretely before reporting it.
    Counterexample {
        /// Calldata that distinguishes the two codes.
        calldata: Bytes,
        /// ABI-typed arguments, when the check was limited to a function.
        args: Vec<DynSolValue>,
        /// Pre-state storage values the counterexample needs.
        storage: Vec<SymbolicStorageAssignment>,
        /// Transaction and block environment of the counterexample.
        env: Box<SymbolicEquivalenceEnv>,
        /// Calls into unknown code on the two paths, whose answers come from the model.
        unknown_calls: usize,
        /// Whether a path read `GAS`, which the model chooses freely.
        reads_gas: bool,
        /// Execution counters collected before the counterexample was found.
        stats: SymbolicStats,
    },
    /// The check stopped before it could establish equivalence.
    Incomplete {
        /// Category describing why the check stopped.
        kind: SymbolicStopReason,
        /// Human-readable explanation.
        reason: String,
        /// Execution counters collected before the check stopped.
        stats: SymbolicStats,
    },
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
/// This executor is intentionally separate from the concrete evm2 executor used by
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
    folded_slots: runtime::FoldedSlots,
    /// Set while an equivalence check runs.
    equivalence: Option<EquivalenceMode>,
}

/// Settings of a running equivalence check.
#[derive(Clone, Copy, Debug)]
struct EquivalenceMode {
    /// Assume unknown code never calls back into the programs, so CALLs into it are abstracted.
    promise_no_reent: bool,
    /// Largest return data of a call into unknown code, in bytes.
    max_returndata: usize,
    /// Whether a program read GAS, which counterexamples may then depend on.
    reads_gas: bool,
    /// Leave out inputs on which a program runs out of memory gas.
    ignore_out_of_gas: bool,
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
