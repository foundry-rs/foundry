#[macro_use]
mod macros;

mod logs;
pub use logs::LogCollector;

mod tracer;
pub use tracer::Tracer;

mod stack;
pub use stack::InspectorStack;

mod cheatcodes;
pub use cheatcodes::Cheatcodes;

#[derive(Default, Clone)]
pub struct InspectorStackConfig {
    /// Whether or not cheatcodes are enabled
    pub cheatcodes: bool,
    /// Whether or not the FFI cheatcode is enabled
    pub ffi: bool,
    /// Whether or not tracing is enabled
    pub tracing: bool,
}

impl InspectorStackConfig {
    pub fn stack(&self) -> InspectorStack {
        let mut stack = InspectorStack::new();

        stack.logs = Some(LogCollector::new());
        if self.cheatcodes {
            stack.cheatcodes = Some(Cheatcodes::new(self.ffi));
        }
        if self.tracing {
            stack.tracer = Some(Tracer::new());
        }
        stack
    }
}
