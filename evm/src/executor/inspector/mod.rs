#[macro_use]
mod utils;

mod logs;
pub use logs::LogCollector;

mod tracer;
pub use tracer::Tracer;

mod debugger;
pub use debugger::Debugger;

mod stack;
pub use stack::{InspectorData, InspectorStack};

mod cheatcodes;
pub use cheatcodes::{Cheatcodes, DEFAULT_CREATE2_DEPLOYER};

use ethers::types::U256;
use revm::BlockEnv;

#[derive(Default, Clone, Debug)]
pub struct InspectorStackConfig {
    /// The cheatcode inspector and its state, if cheatcodes are enabled.
    /// Whether or not cheatcodes are enabled
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
    /// Whether or not tracing is enabled
    pub tracing: bool,
    /// Whether or not the debugger is enabled
    pub debugger: bool,
}

impl InspectorStackConfig {
    pub fn stack(&self) -> InspectorStack {
        let mut stack =
            InspectorStack { logs: Some(LogCollector::default()), ..Default::default() };

        stack.cheatcodes = self.cheatcodes.clone();
        if let Some(ref mut cheatcodes) = stack.cheatcodes {
            cheatcodes.block = Some(self.block.clone());
            cheatcodes.gas_price = Some(self.gas_price);
        }

        if self.tracing {
            stack.tracer = Some(Tracer::default());
        }
        if self.debugger {
            stack.debugger = Some(Debugger::default());
        }
        stack
    }
}
