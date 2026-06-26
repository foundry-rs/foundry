//! Foundry's symbolic EVM executor.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, B256, Bytes, I256, U256, hex, keccak256,
    map::{DefaultHashBuilder, HashMap, HashSet, IndexSet},
};
use alloy_signer::SignerSync;
use alloy_signer_local::{
    PrivateKeySigner,
    coins_bip39::{English, Wordlist},
};
use alloy_sol_types::SolCall;
use base64::prelude::*;
use foundry_config::{
    SymbolicConfig, SymbolicExplorationOrder, SymbolicStorageLayout, split_quoted_args,
};
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
use std::{
    collections::{BTreeMap, VecDeque},
    fmt::{self, Write as _},
    io::{Read, Write},
    num::NonZeroU32,
    ops::{ControlFlow, Deref, DerefMut},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, LazyLock,
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

mod symbolic_vm_calls {
    alloy_sol_types::sol! {
        interface SymbolicVm {
            function createAddress(string calldata name) external returns (address value);
            function createBool(string calldata name) external returns (bool value);
            function createBytes(string calldata name) external returns (bytes memory value);
            function createBytes(uint256 len, string calldata name) external returns (bytes memory value);
            function createCalldata(string calldata name) external returns (bytes memory value);
            function createInt(uint256 bits, string calldata name) external returns (int256 value);
            function createString(string calldata name) external returns (string memory value);
            function createString(uint256 len, string calldata name) external returns (string memory value);
            function createUint(uint256 bits, string calldata name) external returns (uint256 value);
            function enableSymbolicStorage(address target) external;
            function snapshotStorage(address target) external returns (uint256 id);

            function createUint8(string calldata name) external returns (uint8 value);
            function createUint16(string calldata name) external returns (uint16 value);
            function createUint24(string calldata name) external returns (uint24 value);
            function createUint32(string calldata name) external returns (uint32 value);
            function createUint40(string calldata name) external returns (uint40 value);
            function createUint48(string calldata name) external returns (uint48 value);
            function createUint56(string calldata name) external returns (uint56 value);
            function createUint64(string calldata name) external returns (uint64 value);
            function createUint72(string calldata name) external returns (uint72 value);
            function createUint80(string calldata name) external returns (uint80 value);
            function createUint88(string calldata name) external returns (uint88 value);
            function createUint96(string calldata name) external returns (uint96 value);
            function createUint104(string calldata name) external returns (uint104 value);
            function createUint112(string calldata name) external returns (uint112 value);
            function createUint120(string calldata name) external returns (uint120 value);
            function createUint128(string calldata name) external returns (uint128 value);
            function createUint136(string calldata name) external returns (uint136 value);
            function createUint144(string calldata name) external returns (uint144 value);
            function createUint152(string calldata name) external returns (uint152 value);
            function createUint160(string calldata name) external returns (uint160 value);
            function createUint168(string calldata name) external returns (uint168 value);
            function createUint176(string calldata name) external returns (uint176 value);
            function createUint184(string calldata name) external returns (uint184 value);
            function createUint192(string calldata name) external returns (uint192 value);
            function createUint200(string calldata name) external returns (uint200 value);
            function createUint208(string calldata name) external returns (uint208 value);
            function createUint216(string calldata name) external returns (uint216 value);
            function createUint224(string calldata name) external returns (uint224 value);
            function createUint232(string calldata name) external returns (uint232 value);
            function createUint240(string calldata name) external returns (uint240 value);
            function createUint248(string calldata name) external returns (uint248 value);
            function createUint256(string calldata name) external returns (uint256 value);

            function createInt8(string calldata name) external returns (int8 value);
            function createInt16(string calldata name) external returns (int16 value);
            function createInt24(string calldata name) external returns (int24 value);
            function createInt32(string calldata name) external returns (int32 value);
            function createInt40(string calldata name) external returns (int40 value);
            function createInt48(string calldata name) external returns (int48 value);
            function createInt56(string calldata name) external returns (int56 value);
            function createInt64(string calldata name) external returns (int64 value);
            function createInt72(string calldata name) external returns (int72 value);
            function createInt80(string calldata name) external returns (int80 value);
            function createInt88(string calldata name) external returns (int88 value);
            function createInt96(string calldata name) external returns (int96 value);
            function createInt104(string calldata name) external returns (int104 value);
            function createInt112(string calldata name) external returns (int112 value);
            function createInt120(string calldata name) external returns (int120 value);
            function createInt128(string calldata name) external returns (int128 value);
            function createInt136(string calldata name) external returns (int136 value);
            function createInt144(string calldata name) external returns (int144 value);
            function createInt152(string calldata name) external returns (int152 value);
            function createInt160(string calldata name) external returns (int160 value);
            function createInt168(string calldata name) external returns (int168 value);
            function createInt176(string calldata name) external returns (int176 value);
            function createInt184(string calldata name) external returns (int184 value);
            function createInt192(string calldata name) external returns (int192 value);
            function createInt200(string calldata name) external returns (int200 value);
            function createInt208(string calldata name) external returns (int208 value);
            function createInt216(string calldata name) external returns (int216 value);
            function createInt224(string calldata name) external returns (int224 value);
            function createInt232(string calldata name) external returns (int232 value);
            function createInt240(string calldata name) external returns (int240 value);
            function createInt248(string calldata name) external returns (int248 value);
            function createInt256(string calldata name) external returns (int256 value);

            function createBytes1(string calldata name) external returns (bytes1 value);
            function createBytes2(string calldata name) external returns (bytes2 value);
            function createBytes3(string calldata name) external returns (bytes3 value);
            function createBytes4(string calldata name) external returns (bytes4 value);
            function createBytes5(string calldata name) external returns (bytes5 value);
            function createBytes6(string calldata name) external returns (bytes6 value);
            function createBytes7(string calldata name) external returns (bytes7 value);
            function createBytes8(string calldata name) external returns (bytes8 value);
            function createBytes9(string calldata name) external returns (bytes9 value);
            function createBytes10(string calldata name) external returns (bytes10 value);
            function createBytes11(string calldata name) external returns (bytes11 value);
            function createBytes12(string calldata name) external returns (bytes12 value);
            function createBytes13(string calldata name) external returns (bytes13 value);
            function createBytes14(string calldata name) external returns (bytes14 value);
            function createBytes15(string calldata name) external returns (bytes15 value);
            function createBytes16(string calldata name) external returns (bytes16 value);
            function createBytes17(string calldata name) external returns (bytes17 value);
            function createBytes18(string calldata name) external returns (bytes18 value);
            function createBytes19(string calldata name) external returns (bytes19 value);
            function createBytes20(string calldata name) external returns (bytes20 value);
            function createBytes21(string calldata name) external returns (bytes21 value);
            function createBytes22(string calldata name) external returns (bytes22 value);
            function createBytes23(string calldata name) external returns (bytes23 value);
            function createBytes24(string calldata name) external returns (bytes24 value);
            function createBytes25(string calldata name) external returns (bytes25 value);
            function createBytes26(string calldata name) external returns (bytes26 value);
            function createBytes27(string calldata name) external returns (bytes27 value);
            function createBytes28(string calldata name) external returns (bytes28 value);
            function createBytes29(string calldata name) external returns (bytes29 value);
            function createBytes30(string calldata name) external returns (bytes30 value);
            function createBytes31(string calldata name) external returns (bytes31 value);
            function createBytes32(string calldata name) external returns (bytes32 value);
        }
    }
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
            symbolic_vm_calls::SymbolicVm::createAddressCall::SELECTOR => Some(Self::CreateAddress),
            symbolic_vm_calls::SymbolicVm::createBoolCall::SELECTOR => Some(Self::CreateBool),
            symbolic_vm_calls::SymbolicVm::createBytes_0Call::SELECTOR => Some(Self::CreateBytes),
            symbolic_vm_calls::SymbolicVm::createBytes_1Call::SELECTOR => {
                Some(Self::CreateBytesSized)
            }
            symbolic_vm_calls::SymbolicVm::createCalldataCall::SELECTOR => {
                Some(Self::CreateCalldata)
            }
            symbolic_vm_calls::SymbolicVm::createIntCall::SELECTOR => Some(Self::CreateInt),
            symbolic_vm_calls::SymbolicVm::createString_0Call::SELECTOR => Some(Self::CreateString),
            symbolic_vm_calls::SymbolicVm::createString_1Call::SELECTOR => {
                Some(Self::CreateStringSized)
            }
            symbolic_vm_calls::SymbolicVm::createUintCall::SELECTOR => Some(Self::CreateUint),
            symbolic_vm_calls::SymbolicVm::enableSymbolicStorageCall::SELECTOR
            | foundry_cheatcodes_spec::Vm::setArbitraryStorage_0Call::SELECTOR => {
                Some(Self::EnableSymbolicStorage)
            }
            symbolic_vm_calls::SymbolicVm::snapshotStorageCall::SELECTOR => {
                Some(Self::SnapshotStorage)
            }
            foundry_cheatcodes_spec::Vm::snapshotStateCall::SELECTOR => Some(Self::SnapshotState),
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
        (8, symbolic_vm_calls::SymbolicVm::createUint8Call::SELECTOR),
        (16, symbolic_vm_calls::SymbolicVm::createUint16Call::SELECTOR),
        (24, symbolic_vm_calls::SymbolicVm::createUint24Call::SELECTOR),
        (32, symbolic_vm_calls::SymbolicVm::createUint32Call::SELECTOR),
        (40, symbolic_vm_calls::SymbolicVm::createUint40Call::SELECTOR),
        (48, symbolic_vm_calls::SymbolicVm::createUint48Call::SELECTOR),
        (56, symbolic_vm_calls::SymbolicVm::createUint56Call::SELECTOR),
        (64, symbolic_vm_calls::SymbolicVm::createUint64Call::SELECTOR),
        (72, symbolic_vm_calls::SymbolicVm::createUint72Call::SELECTOR),
        (80, symbolic_vm_calls::SymbolicVm::createUint80Call::SELECTOR),
        (88, symbolic_vm_calls::SymbolicVm::createUint88Call::SELECTOR),
        (96, symbolic_vm_calls::SymbolicVm::createUint96Call::SELECTOR),
        (104, symbolic_vm_calls::SymbolicVm::createUint104Call::SELECTOR),
        (112, symbolic_vm_calls::SymbolicVm::createUint112Call::SELECTOR),
        (120, symbolic_vm_calls::SymbolicVm::createUint120Call::SELECTOR),
        (128, symbolic_vm_calls::SymbolicVm::createUint128Call::SELECTOR),
        (136, symbolic_vm_calls::SymbolicVm::createUint136Call::SELECTOR),
        (144, symbolic_vm_calls::SymbolicVm::createUint144Call::SELECTOR),
        (152, symbolic_vm_calls::SymbolicVm::createUint152Call::SELECTOR),
        (160, symbolic_vm_calls::SymbolicVm::createUint160Call::SELECTOR),
        (168, symbolic_vm_calls::SymbolicVm::createUint168Call::SELECTOR),
        (176, symbolic_vm_calls::SymbolicVm::createUint176Call::SELECTOR),
        (184, symbolic_vm_calls::SymbolicVm::createUint184Call::SELECTOR),
        (192, symbolic_vm_calls::SymbolicVm::createUint192Call::SELECTOR),
        (200, symbolic_vm_calls::SymbolicVm::createUint200Call::SELECTOR),
        (208, symbolic_vm_calls::SymbolicVm::createUint208Call::SELECTOR),
        (216, symbolic_vm_calls::SymbolicVm::createUint216Call::SELECTOR),
        (224, symbolic_vm_calls::SymbolicVm::createUint224Call::SELECTOR),
        (232, symbolic_vm_calls::SymbolicVm::createUint232Call::SELECTOR),
        (240, symbolic_vm_calls::SymbolicVm::createUint240Call::SELECTOR),
        (248, symbolic_vm_calls::SymbolicVm::createUint248Call::SELECTOR),
        (256, symbolic_vm_calls::SymbolicVm::createUint256Call::SELECTOR),
    ];
    &SELECTORS
}

fn symbolic_create_int_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: [(usize, [u8; 4]); 32] = [
        (8, symbolic_vm_calls::SymbolicVm::createInt8Call::SELECTOR),
        (16, symbolic_vm_calls::SymbolicVm::createInt16Call::SELECTOR),
        (24, symbolic_vm_calls::SymbolicVm::createInt24Call::SELECTOR),
        (32, symbolic_vm_calls::SymbolicVm::createInt32Call::SELECTOR),
        (40, symbolic_vm_calls::SymbolicVm::createInt40Call::SELECTOR),
        (48, symbolic_vm_calls::SymbolicVm::createInt48Call::SELECTOR),
        (56, symbolic_vm_calls::SymbolicVm::createInt56Call::SELECTOR),
        (64, symbolic_vm_calls::SymbolicVm::createInt64Call::SELECTOR),
        (72, symbolic_vm_calls::SymbolicVm::createInt72Call::SELECTOR),
        (80, symbolic_vm_calls::SymbolicVm::createInt80Call::SELECTOR),
        (88, symbolic_vm_calls::SymbolicVm::createInt88Call::SELECTOR),
        (96, symbolic_vm_calls::SymbolicVm::createInt96Call::SELECTOR),
        (104, symbolic_vm_calls::SymbolicVm::createInt104Call::SELECTOR),
        (112, symbolic_vm_calls::SymbolicVm::createInt112Call::SELECTOR),
        (120, symbolic_vm_calls::SymbolicVm::createInt120Call::SELECTOR),
        (128, symbolic_vm_calls::SymbolicVm::createInt128Call::SELECTOR),
        (136, symbolic_vm_calls::SymbolicVm::createInt136Call::SELECTOR),
        (144, symbolic_vm_calls::SymbolicVm::createInt144Call::SELECTOR),
        (152, symbolic_vm_calls::SymbolicVm::createInt152Call::SELECTOR),
        (160, symbolic_vm_calls::SymbolicVm::createInt160Call::SELECTOR),
        (168, symbolic_vm_calls::SymbolicVm::createInt168Call::SELECTOR),
        (176, symbolic_vm_calls::SymbolicVm::createInt176Call::SELECTOR),
        (184, symbolic_vm_calls::SymbolicVm::createInt184Call::SELECTOR),
        (192, symbolic_vm_calls::SymbolicVm::createInt192Call::SELECTOR),
        (200, symbolic_vm_calls::SymbolicVm::createInt200Call::SELECTOR),
        (208, symbolic_vm_calls::SymbolicVm::createInt208Call::SELECTOR),
        (216, symbolic_vm_calls::SymbolicVm::createInt216Call::SELECTOR),
        (224, symbolic_vm_calls::SymbolicVm::createInt224Call::SELECTOR),
        (232, symbolic_vm_calls::SymbolicVm::createInt232Call::SELECTOR),
        (240, symbolic_vm_calls::SymbolicVm::createInt240Call::SELECTOR),
        (248, symbolic_vm_calls::SymbolicVm::createInt248Call::SELECTOR),
        (256, symbolic_vm_calls::SymbolicVm::createInt256Call::SELECTOR),
    ];
    &SELECTORS
}

fn symbolic_create_bytes_selectors() -> &'static [(usize, [u8; 4]); 32] {
    static SELECTORS: [(usize, [u8; 4]); 32] = [
        (1, symbolic_vm_calls::SymbolicVm::createBytes1Call::SELECTOR),
        (2, symbolic_vm_calls::SymbolicVm::createBytes2Call::SELECTOR),
        (3, symbolic_vm_calls::SymbolicVm::createBytes3Call::SELECTOR),
        (4, symbolic_vm_calls::SymbolicVm::createBytes4Call::SELECTOR),
        (5, symbolic_vm_calls::SymbolicVm::createBytes5Call::SELECTOR),
        (6, symbolic_vm_calls::SymbolicVm::createBytes6Call::SELECTOR),
        (7, symbolic_vm_calls::SymbolicVm::createBytes7Call::SELECTOR),
        (8, symbolic_vm_calls::SymbolicVm::createBytes8Call::SELECTOR),
        (9, symbolic_vm_calls::SymbolicVm::createBytes9Call::SELECTOR),
        (10, symbolic_vm_calls::SymbolicVm::createBytes10Call::SELECTOR),
        (11, symbolic_vm_calls::SymbolicVm::createBytes11Call::SELECTOR),
        (12, symbolic_vm_calls::SymbolicVm::createBytes12Call::SELECTOR),
        (13, symbolic_vm_calls::SymbolicVm::createBytes13Call::SELECTOR),
        (14, symbolic_vm_calls::SymbolicVm::createBytes14Call::SELECTOR),
        (15, symbolic_vm_calls::SymbolicVm::createBytes15Call::SELECTOR),
        (16, symbolic_vm_calls::SymbolicVm::createBytes16Call::SELECTOR),
        (17, symbolic_vm_calls::SymbolicVm::createBytes17Call::SELECTOR),
        (18, symbolic_vm_calls::SymbolicVm::createBytes18Call::SELECTOR),
        (19, symbolic_vm_calls::SymbolicVm::createBytes19Call::SELECTOR),
        (20, symbolic_vm_calls::SymbolicVm::createBytes20Call::SELECTOR),
        (21, symbolic_vm_calls::SymbolicVm::createBytes21Call::SELECTOR),
        (22, symbolic_vm_calls::SymbolicVm::createBytes22Call::SELECTOR),
        (23, symbolic_vm_calls::SymbolicVm::createBytes23Call::SELECTOR),
        (24, symbolic_vm_calls::SymbolicVm::createBytes24Call::SELECTOR),
        (25, symbolic_vm_calls::SymbolicVm::createBytes25Call::SELECTOR),
        (26, symbolic_vm_calls::SymbolicVm::createBytes26Call::SELECTOR),
        (27, symbolic_vm_calls::SymbolicVm::createBytes27Call::SELECTOR),
        (28, symbolic_vm_calls::SymbolicVm::createBytes28Call::SELECTOR),
        (29, symbolic_vm_calls::SymbolicVm::createBytes29Call::SELECTOR),
        (30, symbolic_vm_calls::SymbolicVm::createBytes30Call::SELECTOR),
        (31, symbolic_vm_calls::SymbolicVm::createBytes31Call::SELECTOR),
        (32, symbolic_vm_calls::SymbolicVm::createBytes32Call::SELECTOR),
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
