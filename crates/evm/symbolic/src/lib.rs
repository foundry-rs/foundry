//! Foundry's symbolic EVM executor.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

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
    constants::{CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS},
    core::{backend::DatabaseExt, evm::FoundryEvmNetwork},
    executors::Executor,
    revm::{
        bytecode::{Bytecode, JumpTable, opcode},
        context::{Block, Transaction},
        database::DatabaseRef,
        precompile::{blake2, bn254, hash, identity, kzg_point_evaluation, modexp, secp256k1},
        primitives::hardfork::SpecId,
    },
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::BTreeMap;
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

mod consts;
pub use consts::BUILTIN_SYMBOLIC_SOLVERS;
pub(crate) use consts::*;

mod abi;
mod executor;
mod runtime;

pub use runtime::{PortfolioDiagnostics, SymbolicBranchTarget, SymbolicError, SymbolicRunInput};

/// Returns whether `solver` is one of Foundry's semantic symbolic solver names.
pub fn symbolic_solver_is_builtin(solver: &str) -> bool {
    BUILTIN_SYMBOLIC_SOLVERS.contains(&solver)
}

/// Returns a warning when a configured symbolic solver portfolio has unavailable entries.
pub fn symbolic_solver_portfolio_availability_warning(config: &SymbolicConfig) -> Option<String> {
    runtime::solver_portfolio_availability_warning(config)
}

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
                for &(bits, candidate) in symbolic_create_uint_selectors() {
                    if selector == candidate {
                        return Some(Self::CreateUintBits(bits));
                    }
                }
                for &(bits, candidate) in symbolic_create_int_selectors() {
                    if selector == candidate {
                        return Some(Self::CreateIntBits(bits));
                    }
                }
                for &(bytes, candidate) in symbolic_create_bytes_selectors() {
                    if selector == candidate {
                        return Some(Self::CreateBytesFixed(bytes));
                    }
                }
                None
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

fn symbolic_create_uint_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: [(usize, [u8; 4]); 32] = [
        (8, SymbolicVm::createUint8Call::SELECTOR),
        (16, SymbolicVm::createUint16Call::SELECTOR),
        (24, SymbolicVm::createUint24Call::SELECTOR),
        (32, SymbolicVm::createUint32Call::SELECTOR),
        (40, SymbolicVm::createUint40Call::SELECTOR),
        (48, SymbolicVm::createUint48Call::SELECTOR),
        (56, SymbolicVm::createUint56Call::SELECTOR),
        (64, SymbolicVm::createUint64Call::SELECTOR),
        (72, SymbolicVm::createUint72Call::SELECTOR),
        (80, SymbolicVm::createUint80Call::SELECTOR),
        (88, SymbolicVm::createUint88Call::SELECTOR),
        (96, SymbolicVm::createUint96Call::SELECTOR),
        (104, SymbolicVm::createUint104Call::SELECTOR),
        (112, SymbolicVm::createUint112Call::SELECTOR),
        (120, SymbolicVm::createUint120Call::SELECTOR),
        (128, SymbolicVm::createUint128Call::SELECTOR),
        (136, SymbolicVm::createUint136Call::SELECTOR),
        (144, SymbolicVm::createUint144Call::SELECTOR),
        (152, SymbolicVm::createUint152Call::SELECTOR),
        (160, SymbolicVm::createUint160Call::SELECTOR),
        (168, SymbolicVm::createUint168Call::SELECTOR),
        (176, SymbolicVm::createUint176Call::SELECTOR),
        (184, SymbolicVm::createUint184Call::SELECTOR),
        (192, SymbolicVm::createUint192Call::SELECTOR),
        (200, SymbolicVm::createUint200Call::SELECTOR),
        (208, SymbolicVm::createUint208Call::SELECTOR),
        (216, SymbolicVm::createUint216Call::SELECTOR),
        (224, SymbolicVm::createUint224Call::SELECTOR),
        (232, SymbolicVm::createUint232Call::SELECTOR),
        (240, SymbolicVm::createUint240Call::SELECTOR),
        (248, SymbolicVm::createUint248Call::SELECTOR),
        (256, SymbolicVm::createUint256Call::SELECTOR),
    ];
    &SELECTORS
}

fn symbolic_create_int_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: [(usize, [u8; 4]); 32] = [
        (8, SymbolicVm::createInt8Call::SELECTOR),
        (16, SymbolicVm::createInt16Call::SELECTOR),
        (24, SymbolicVm::createInt24Call::SELECTOR),
        (32, SymbolicVm::createInt32Call::SELECTOR),
        (40, SymbolicVm::createInt40Call::SELECTOR),
        (48, SymbolicVm::createInt48Call::SELECTOR),
        (56, SymbolicVm::createInt56Call::SELECTOR),
        (64, SymbolicVm::createInt64Call::SELECTOR),
        (72, SymbolicVm::createInt72Call::SELECTOR),
        (80, SymbolicVm::createInt80Call::SELECTOR),
        (88, SymbolicVm::createInt88Call::SELECTOR),
        (96, SymbolicVm::createInt96Call::SELECTOR),
        (104, SymbolicVm::createInt104Call::SELECTOR),
        (112, SymbolicVm::createInt112Call::SELECTOR),
        (120, SymbolicVm::createInt120Call::SELECTOR),
        (128, SymbolicVm::createInt128Call::SELECTOR),
        (136, SymbolicVm::createInt136Call::SELECTOR),
        (144, SymbolicVm::createInt144Call::SELECTOR),
        (152, SymbolicVm::createInt152Call::SELECTOR),
        (160, SymbolicVm::createInt160Call::SELECTOR),
        (168, SymbolicVm::createInt168Call::SELECTOR),
        (176, SymbolicVm::createInt176Call::SELECTOR),
        (184, SymbolicVm::createInt184Call::SELECTOR),
        (192, SymbolicVm::createInt192Call::SELECTOR),
        (200, SymbolicVm::createInt200Call::SELECTOR),
        (208, SymbolicVm::createInt208Call::SELECTOR),
        (216, SymbolicVm::createInt216Call::SELECTOR),
        (224, SymbolicVm::createInt224Call::SELECTOR),
        (232, SymbolicVm::createInt232Call::SELECTOR),
        (240, SymbolicVm::createInt240Call::SELECTOR),
        (248, SymbolicVm::createInt248Call::SELECTOR),
        (256, SymbolicVm::createInt256Call::SELECTOR),
    ];
    &SELECTORS
}

fn symbolic_create_bytes_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: [(usize, [u8; 4]); 32] = [
        (1, SymbolicVm::createBytes1Call::SELECTOR),
        (2, SymbolicVm::createBytes2Call::SELECTOR),
        (3, SymbolicVm::createBytes3Call::SELECTOR),
        (4, SymbolicVm::createBytes4Call::SELECTOR),
        (5, SymbolicVm::createBytes5Call::SELECTOR),
        (6, SymbolicVm::createBytes6Call::SELECTOR),
        (7, SymbolicVm::createBytes7Call::SELECTOR),
        (8, SymbolicVm::createBytes8Call::SELECTOR),
        (9, SymbolicVm::createBytes9Call::SELECTOR),
        (10, SymbolicVm::createBytes10Call::SELECTOR),
        (11, SymbolicVm::createBytes11Call::SELECTOR),
        (12, SymbolicVm::createBytes12Call::SELECTOR),
        (13, SymbolicVm::createBytes13Call::SELECTOR),
        (14, SymbolicVm::createBytes14Call::SELECTOR),
        (15, SymbolicVm::createBytes15Call::SELECTOR),
        (16, SymbolicVm::createBytes16Call::SELECTOR),
        (17, SymbolicVm::createBytes17Call::SELECTOR),
        (18, SymbolicVm::createBytes18Call::SELECTOR),
        (19, SymbolicVm::createBytes19Call::SELECTOR),
        (20, SymbolicVm::createBytes20Call::SELECTOR),
        (21, SymbolicVm::createBytes21Call::SELECTOR),
        (22, SymbolicVm::createBytes22Call::SELECTOR),
        (23, SymbolicVm::createBytes23Call::SELECTOR),
        (24, SymbolicVm::createBytes24Call::SELECTOR),
        (25, SymbolicVm::createBytes25Call::SELECTOR),
        (26, SymbolicVm::createBytes26Call::SELECTOR),
        (27, SymbolicVm::createBytes27Call::SELECTOR),
        (28, SymbolicVm::createBytes28Call::SELECTOR),
        (29, SymbolicVm::createBytes29Call::SELECTOR),
        (30, SymbolicVm::createBytes30Call::SELECTOR),
        (31, SymbolicVm::createBytes31Call::SELECTOR),
        (32, SymbolicVm::createBytes32Call::SELECTOR),
    ];
    &SELECTORS
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
    solver: Box<dyn runtime::SymbolicSolver>,
    deferred_incomplete: Option<DeferredIncomplete>,
    deadline: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
enum DeferredIncomplete {
    Unsupported(&'static str),
    SolverUnknown,
}

#[cfg(test)]
mod tests;
