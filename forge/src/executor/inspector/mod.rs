#[macro_use]
mod macros;

mod logs;
pub use logs::LogCollector;

mod stack;
use revm::Database;
pub use stack::InspectorStack;

#[derive(Default, Clone)]
pub struct InspectorStackConfig {
    /// Whether or not cheatcodes are enabled
    pub cheatcodes: bool,
    /// Whether or not the FFI cheatcode is enabled
    pub ffi: bool,
}

impl InspectorStackConfig {
    pub fn stack<DB>(&self) -> InspectorStack<DB>
    where
        DB: Database,
    {
        let mut stack = InspectorStack::new();

        stack.insert(LogCollector::new());
        stack
    }
}
