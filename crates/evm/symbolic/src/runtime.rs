use super::{abi::*, *};

mod address;
mod bytes;
mod calldata;
mod cheatcodes;
mod control;
mod evm;
mod expr;
mod memory;
mod precompiles;
mod solver;
mod state;
mod symbols;

pub(crate) use address::*;
pub(crate) use bytes::*;
pub(crate) use calldata::*;
pub(crate) use cheatcodes::*;
pub(crate) use control::*;
pub(crate) use evm::*;
pub(crate) use expr::*;
pub(crate) use memory::*;
pub(crate) use precompiles::*;
pub use solver::PortfolioDiagnostics;
pub(crate) use solver::{
    SmtLibSubprocessSolver, SymbolicSolver, solver_portfolio_availability_warning,
};
#[cfg(test)]
pub(crate) use solver::{
    SolverCommand, SolverConfigError, SolverOutcome, SolverRunSummary, fallback_single_var_model,
    hard_arith_fallback_model, named_solver_command, normalize_bool_for_solver,
    normalize_constraints_for_solver, normalize_expr_for_solver, parse_model,
    product_monotonic_unsat, solver_commands_for_config, split_solver_command,
    validate_solver_model_output,
};
pub(crate) use state::*;
pub(crate) use symbols::*;

/// One comparison site from a fuzz branch frontier to target during symbolic execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolicBranchTarget {
    /// Contract address where the comparison executed.
    address: Address,
    /// Program counter of the comparison opcode.
    pc: usize,
    /// Comparison opcode.
    opcode: u8,
    /// Concrete result observed by fuzzing. Symbolic execution targets the opposite result.
    result: bool,
}

impl SymbolicBranchTarget {
    pub const fn new(address: Address, pc: usize, opcode: u8, result: bool) -> Self {
        Self { address, pc, opcode, result }
    }

    pub(crate) const fn result(self) -> bool {
        self.result
    }

    pub(crate) fn matches(self, address: Address, pc: usize, opcode: u8) -> bool {
        self.address == address && self.pc == pc && self.opcode == opcode
    }
}

pub struct SymbolicRunInput<'a, FEN: FoundryEvmNetwork> {
    /// Concrete Foundry executor used as the source of deployed bytecode and backend state.
    pub executor: &'a Executor<FEN>,
    /// Address of the deployed test contract whose runtime bytecode will be explored.
    pub target: Address,
    /// Sender used for symbolic execution environment opcodes such as `CALLER` and `ORIGIN`.
    pub sender: Address,
    /// ABI function to invoke symbolically.
    pub function: &'a Function,
    /// Call value exposed to the symbolic execution through `CALLVALUE`.
    pub value: U256,
    /// Whether symbolic `vm.ffi` calls are allowed to execute subprocesses.
    pub ffi_enabled: bool,
    /// Whether to return one successful concrete input when execution is safe.
    pub collect_success_input: bool,
    /// Concrete fuzz corpus entries used as path-priority hints.
    pub corpus_seeds: Vec<SymbolicConcreteInput>,
    /// Optional comparison site whose opposite branch should be solved.
    pub branch_target: Option<SymbolicBranchTarget>,
}

/// Error returned by the internal symbolic executor.
///
/// Public callers normally receive these errors as [`SymbolicRunResult::Incomplete`]
/// through [`SymbolicExecutor::run`]. The enum is public so integration code and tests
/// can inspect exact failure causes when using lower-level helpers in this crate.
#[derive(Debug, Error)]
pub enum SymbolicError {
    /// The target account was not present in the executor backend.
    #[error("missing account {0}")]
    MissingAccount(Address),
    /// The target account had no runtime bytecode.
    #[error("missing code for account {0}")]
    MissingCode(Address),
    /// The concrete backend returned an error while reading account state.
    #[error("backend error: {0}")]
    Backend(String),
    /// The function ABI contains a type that is not supported by the V1 symbolic calldata model.
    #[error("unsupported ABI type for symbolic execution: {0}")]
    UnsupportedAbi(String),
    /// Symbolic calldata variant expansion exceeded the configured path-width budget.
    #[error("symbolic calldata variant limit exceeded ({0})")]
    CalldataVariantLimit(usize),
    /// Symbolic execution reached a feature that is not implemented yet.
    #[error("unsupported symbolic execution feature: {0}")]
    Unsupported(&'static str),
    /// Symbolic execution reached an opcode that is not implemented yet.
    #[error("unsupported opcode 0x{0:02x}")]
    UnsupportedOpcode(u8),
    /// Runtime bytecode was malformed in a way that prevents symbolic execution.
    #[error("invalid bytecode: {0}")]
    InvalidBytecode(&'static str),
    /// A jump targeted a byte offset that is not a valid `JUMPDEST`.
    #[error("invalid jump destination {0}")]
    InvalidJump(usize),
    /// The symbolic stack was popped without enough values.
    #[error("stack underflow")]
    StackUnderflow,
    /// The symbolic stack exceeded the EVM stack limit.
    #[error("stack overflow")]
    StackOverflow,
    /// The solver process failed, timed out, or returned an unexpected response.
    #[error("solver error: {0}")]
    Solver(String),
    /// The solver returned `unknown`.
    #[error("solver returned unknown")]
    SolverUnknown,
    /// The configured maximum number of solver queries was reached.
    #[error("symbolic solver query limit exceeded ({0})")]
    SolverQueryLimit(usize),
    /// ABI encoding failed while constructing a concrete counterexample call.
    #[error(transparent)]
    Abi(#[from] alloy_dyn_abi::Error),
}

impl SymbolicError {
    pub(super) const fn stop_reason(&self) -> SymbolicStopReason {
        match self {
            Self::Unsupported(_)
            | Self::CalldataVariantLimit(_)
            | Self::UnsupportedOpcode(_)
            | Self::SolverQueryLimit(_) => SymbolicStopReason::Stuck,
            Self::SolverUnknown => SymbolicStopReason::Timeout,
            Self::Solver(_)
            | Self::MissingAccount(_)
            | Self::MissingCode(_)
            | Self::Backend(_)
            | Self::UnsupportedAbi(_)
            | Self::InvalidBytecode(_)
            | Self::InvalidJump(_)
            | Self::StackUnderflow
            | Self::StackOverflow
            | Self::Abi(_) => SymbolicStopReason::Error,
        }
    }
}

impl fmt::Display for SymbolicRunResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Safe { stats, .. } => write!(f, "safe after {} paths", stats.paths),
            Self::Counterexample { stats, .. } => {
                write!(f, "counterexample after {} paths", stats.paths)
            }
            Self::Incomplete { kind, reason, .. } => {
                write!(f, "incomplete symbolic execution ({kind:?}): {reason}")
            }
        }
    }
}
