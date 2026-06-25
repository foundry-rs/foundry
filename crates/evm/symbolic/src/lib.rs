//! Foundry's symbolic EVM executor.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, B256, Bytes, I256, U256, U512, hex, keccak256,
    map::{HashMap, HashSet, IndexSet},
};
use alloy_signer::SignerSync;
use alloy_signer_local::{
    PrivateKeySigner,
    coins_bip39::{English, Wordlist},
};
use base64::prelude::*;
use foundry_config::{
    SymbolicConfig, SymbolicExplorationOrder, SymbolicStorageLayout, split_quoted_args,
};
use foundry_evm::{
    constants::{CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS},
    core::{backend::DatabaseExt, evm::FoundryEvmNetwork},
    executors::Executor,
    revm::{
        bytecode::opcode,
        context::{Block, Transaction},
        database::DatabaseRef,
        precompile::{blake2, bn254, hash, identity, kzg_point_evaluation, modexp, secp256k1},
        primitives::hardfork::SpecId,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::{self, Write as _},
    io::{Read, Write},
    ops::{Deref, DerefMut},
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

pub use runtime::{PortfolioDiagnostics, SymbolicError, SymbolicRunInput};

/// Returns whether `solver` is one of Foundry's semantic symbolic solver names.
pub fn symbolic_solver_is_builtin(solver: &str) -> bool {
    BUILTIN_SYMBOLIC_SOLVERS.contains(&solver)
}

/// Returns a warning when a configured symbolic solver portfolio has unavailable entries.
pub fn symbolic_solver_portfolio_availability_warning(config: &SymbolicConfig) -> Option<String> {
    runtime::solver_portfolio_availability_warning(config)
}

/// Returns the `selector_for` symbolic public API helper result.
fn selector_for(signature: &str) -> [u8; 4] {
    let hash = keccak256(signature);
    [hash[0], hash[1], hash[2], hash[3]]
}

struct SymbolicVmSelectors {
    create_address: [u8; 4],
    create_bool: [u8; 4],
    create_bytes: [u8; 4],
    create_bytes_sized: [u8; 4],
    create_bytes32: [u8; 4],
    create_bytes4: [u8; 4],
    create_calldata: [u8; 4],
    create_int: [u8; 4],
    create_int256: [u8; 4],
    create_string: [u8; 4],
    create_string_sized: [u8; 4],
    create_uint: [u8; 4],
    create_uint256: [u8; 4],
    enable_symbolic_storage: [u8; 4],
    snapshot_storage: [u8; 4],
}

/// Returns cached selectors for static symbolic VM helper signatures.
fn symbolic_vm_selectors() -> &'static SymbolicVmSelectors {
    static SELECTORS: std::sync::LazyLock<SymbolicVmSelectors> =
        std::sync::LazyLock::new(|| SymbolicVmSelectors {
            create_address: selector_for("createAddress(string)"),
            create_bool: selector_for("createBool(string)"),
            create_bytes: selector_for("createBytes(string)"),
            create_bytes_sized: selector_for("createBytes(uint256,string)"),
            create_bytes32: selector_for("createBytes32(string)"),
            create_bytes4: selector_for("createBytes4(string)"),
            create_calldata: selector_for("createCalldata(string)"),
            create_int: selector_for("createInt(uint256,string)"),
            create_int256: selector_for("createInt256(string)"),
            create_string: selector_for("createString(string)"),
            create_string_sized: selector_for("createString(uint256,string)"),
            create_uint: selector_for("createUint(uint256,string)"),
            create_uint256: selector_for("createUint256(string)"),
            enable_symbolic_storage: selector_for("enableSymbolicStorage(address)"),
            snapshot_storage: selector_for("snapshotStorage(address)"),
        });
    &SELECTORS
}

/// Returns cached selectors for `createUint{bits}(string)` symbolic helpers.
fn symbolic_create_uint_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: std::sync::LazyLock<[(usize, [u8; 4]); 32]> =
        std::sync::LazyLock::new(|| {
            std::array::from_fn(|idx| {
                let bits = (idx + 1) * 8;
                (bits, selector_for(&format!("createUint{bits}(string)")))
            })
        });
    &SELECTORS
}

/// Returns cached selectors for `createInt{bits}(string)` symbolic helpers.
fn symbolic_create_int_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: std::sync::LazyLock<[(usize, [u8; 4]); 32]> =
        std::sync::LazyLock::new(|| {
            std::array::from_fn(|idx| {
                let bits = (idx + 1) * 8;
                (bits, selector_for(&format!("createInt{bits}(string)")))
            })
        });
    &SELECTORS
}

/// Returns cached selectors for `createBytes{bytes}(string)` symbolic helpers.
fn symbolic_create_bytes_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: std::sync::LazyLock<[(usize, [u8; 4]); 32]> =
        std::sync::LazyLock::new(|| {
            std::array::from_fn(|idx| {
                let bytes = idx + 1;
                (bytes, selector_for(&format!("createBytes{bytes}(string)")))
            })
        });
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
    Safe(SymbolicStats),
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
    /// Whether ordinary target-call reverts should be reported as failures.
    pub fail_on_revert: bool,
    /// Whether symbolic `vm.ffi` calls are allowed to execute subprocesses.
    pub ffi_enabled: bool,
}

/// Outcome of bounded symbolic invariant execution.
#[derive(Clone, Debug)]
pub enum SymbolicInvariantRunResult {
    /// No feasible invariant failure was found within the configured sequence depth.
    Safe(SymbolicStats),
    /// A feasible invariant or handler failure was found.
    Counterexample {
        /// Concrete sequence extracted from the solver model.
        sequence: Vec<SymbolicInvariantStep>,
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
}

/// SMT-LIB-backed symbolic executor.
///
/// This executor is intentionally separate from the concrete revm executor used by
/// Foundry. It consumes bytecode and state from an existing [`Executor`], explores
/// symbolic branches, and returns either a proof result, a counterexample candidate,
/// or an incomplete result.
pub struct SymbolicExecutor {
    config: SymbolicConfig,
    solver: Box<dyn runtime::SymbolicSolver>,
    deferred_incomplete: Option<DeferredIncomplete>,
}

#[derive(Clone, Copy, Debug)]
enum DeferredIncomplete {
    Unsupported(&'static str),
    SolverUnknown,
}

#[cfg(test)]
mod tests;
