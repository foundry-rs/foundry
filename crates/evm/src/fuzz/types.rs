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
    pub case: FuzzCase,
    pub gas_used: u64,
    pub stipend: u64,
    pub traces: Option<CallTraceArena>,
    pub coverage: Option<HitMaps>,
    pub debug: Option<DebugArena>,
    pub breakpoints: Breakpoints,
}

/// Returned by a single fuzz when a counterexample has been discovered
#[derive(Debug)]
pub struct CounterExampleOutcome {
    pub counterexample: (ethers::types::Bytes, RawCallResult),
    pub exit_reason: InstructionResult,
    pub debug: Option<DebugArena>,
    pub breakpoints: Breakpoints,
}

/// Outcome of a single fuzz
#[derive(Debug)]
pub enum FuzzOutcome {
    Case(CaseOutcome),
    CounterExample(CounterExampleOutcome),
}
