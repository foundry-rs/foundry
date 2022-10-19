#[macro_use]
mod utils;

mod logs;

pub use logs::LogCollector;
use std::{cell::RefCell, rc::Rc};

mod access_list;
pub use access_list::AccessListTracer;

mod tracer;
pub use tracer::Tracer;

mod debugger;
pub use debugger::Debugger;

mod coverage;
pub use coverage::CoverageCollector;

mod stack;
pub use stack::{InspectorData, InspectorStack};

pub mod cheatcodes;
pub use cheatcodes::{Cheatcodes, CheatsConfig, DEFAULT_CREATE2_DEPLOYER};

use ethers::types::U256;

use revm::{BlockEnv, GasInspector};

mod fuzzer;
pub use fuzzer::Fuzzer;

#[derive(Default, Clone, Debug)]
pub struct InspectorStackConfig {
    /// The cheatcode inspector and its state, if cheatcodes are enabled.
    /// Whether cheatcodes are enabled
    pub cheatcodes: Option<Cheatcodes>,
    /// The block environment
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: BlockEnv,
    /// The gas price
    ///
    /// Used in the cheatcode handler to overwrite the gas price separately from the gas price
    /// in the execution environment.
    pub gas_price: U256,
    /// Whether tracing is enabled
    pub tracing: bool,
    /// Whether the debugger is enabled
    pub debugger: bool,
    /// The fuzzer inspector and its state, if it exists.
    pub fuzzer: Option<Fuzzer>,
    /// Whether coverage info should be collected
    pub coverage: bool,
}

impl InspectorStackConfig {
    /// Returns the stack of inspectors to use when transacting/committing on the EVM
    ///
    /// See also [`revm::Evm::inspect_ref`] and  [`revm::Evm::commit_ref`]
    pub fn stack(&self) -> InspectorStack {
        let mut stack =
            InspectorStack { logs: Some(LogCollector::default()), ..Default::default() };

        stack.cheatcodes = self.create_cheatcodes();
        if let Some(ref mut cheatcodes) = stack.cheatcodes {
            cheatcodes.block = Some(self.block.clone());
            cheatcodes.gas_price = Some(self.gas_price);
        }

        if self.tracing {
            stack.tracer = Some(Tracer::default());
        }
        if self.debugger {
            let gas_inspector = Rc::new(RefCell::new(GasInspector::default()));
            stack.gas = Some(gas_inspector.clone());
            stack.debugger = Some(Debugger::new(gas_inspector));
        }
        stack.fuzzer = self.fuzzer.clone();

        if self.coverage {
            stack.coverage = Some(CoverageCollector::default());
        }
        stack
    }

    /// Configures the cheatcode inspector with a new and empty context
    ///
    /// Returns `None` if no cheatcodes inspector is set
    fn create_cheatcodes(&self) -> Option<Cheatcodes> {
        let cheatcodes = self.cheatcodes.clone();

        cheatcodes.map(|cheatcodes| Cheatcodes { context: Default::default(), ..cheatcodes })
    }
}
