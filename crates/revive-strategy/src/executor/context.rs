use foundry_compilers::{
    resolc::dual_compiled_contracts::DualCompiledContracts, ProjectCompileOutput,
};
use foundry_evm::executors::ExecutorStrategyContext;

/// Defines the context for [crate::ReviveExecutorStrategyRunner].
#[derive(Debug, Default, Clone)]
pub struct ReviveExecutorStrategyContext {
    /// Temporary flag to indicate if the executor is in PVM mode.
    pub(crate) wip_in_pvm: bool,
    /// Whether to start in PVM mode (from config)
    pub(crate) resolc_startup: bool,
    /// Dual compiled contracts.
    pub(crate) dual_compiled_contracts: DualCompiledContracts,
    /// Compilation output.
    pub(crate) compilation_output: Option<ProjectCompileOutput>,
}

impl ReviveExecutorStrategyContext {
    pub fn new(resolc_startup: bool) -> Self {
        Self { resolc_startup, ..Default::default() }
    }
}

impl ExecutorStrategyContext for ReviveExecutorStrategyContext {
    fn new_cloned(&self) -> Box<dyn ExecutorStrategyContext> {
        Box::new(self.clone())
    }

    fn as_any_ref(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
