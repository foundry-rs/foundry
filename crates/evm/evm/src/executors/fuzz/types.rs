use crate::executors::RawCallResult;
use alloy_primitives::Bytes;
use foundry_common::evm::Breakpoints;
use foundry_evm_core::debug::DebugArena;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::FuzzCase;
use foundry_evm_traces::CallTraceArena;
use revm::interpreter::InstructionResult;

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
    pub counterexample: (Bytes, RawCallResult),
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
