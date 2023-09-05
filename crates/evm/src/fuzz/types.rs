use crate::{coverage::HitMaps, debug::DebugArena, executor::RawCallResult, trace::CallTraceArena};
use ethers::types::Bytes;
use foundry_common::evm::Breakpoints;
use revm::interpreter::InstructionResult;
use serde::{Deserialize, Serialize};

/// Data of a single fuzz test case
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FuzzCase {
    /// The calldata used for this fuzz test
    pub calldata: Bytes,
    /// Consumed gas
    pub gas: u64,
    /// The initial gas stipend for the transaction
    pub stipend: u64,
}

/// Returned by a single fuzz in the case of a successful run
#[derive(Debug)]
pub struct CaseOutcome {
    /// Data of a single fuzz test case
    pub case: FuzzCase,
    /// The traces of the call
    pub traces: Option<CallTraceArena>,
    /// The coverage info collected during the call
    pub coverage: Option<HitMaps>,
    /// The debug nodes of the call
    pub debug: Option<DebugArena>,
    /// Breakpoints char pc map
    pub breakpoints: Breakpoints,
}

/// Returned by a single fuzz when a counterexample has been discovered
#[derive(Debug)]
pub struct CounterExampleOutcome {
    /// Minimal reproduction test case for failing test
    pub counterexample: (ethers::types::Bytes, RawCallResult),
    /// The status of the call
    pub exit_reason: InstructionResult,
    /// The debug nodes of the call
    pub debug: Option<DebugArena>,
    /// Breakpoints char pc map
    pub breakpoints: Breakpoints,
}

/// Outcome of a single fuzz
#[derive(Debug)]
pub enum FuzzOutcome {
    Case(CaseOutcome),
    CounterExample(CounterExampleOutcome),
}
