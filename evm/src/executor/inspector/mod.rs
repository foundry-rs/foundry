#[macro_use]
mod utils;

mod logs;
pub use logs::LogCollector;

mod tracer;
pub use tracer::Tracer;

mod debugger;
pub use debugger::Debugger;

mod stack;
pub use stack::InspectorStack;

mod cheatcodes;
pub use cheatcodes::Cheatcodes;

use revm::BlockEnv;

#[derive(Default, Clone, Debug)]
pub struct InspectorStackConfig {
    /// Whether or not cheatcodes are enabled
    pub cheatcodes: bool,
    /// Whether or not the FFI cheatcode is enabled
    pub ffi: bool,
    /// The block environment
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: BlockEnv,
    /// Whether or not tracing is enabled
    pub tracing: bool,
    /// Whether or not the debugger is enabled
    pub debugger: bool,
}

impl InspectorStackConfig {
    pub fn stack(&self) -> InspectorStack {
        let mut stack = InspectorStack::new();

        stack.logs = Some(LogCollector::new());
        if self.cheatcodes {
            stack.cheatcodes = Some(Cheatcodes::new(self.ffi, self.block.clone()));
        }
        if self.tracing {
            stack.tracer = Some(Tracer::new());
        }
        if self.debugger {
            stack.debugger = Some(Debugger::new());
        }
        stack
    }
}
