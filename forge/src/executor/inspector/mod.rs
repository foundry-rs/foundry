#[macro_use]
mod macros;

mod logs;
pub use logs::LogCollector;

mod stack;
use revm::Database;
pub use stack::InspectorStack;

mod cheatcodes;
pub use cheatcodes::Cheatcodes;

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
        if self.cheatcodes {
            stack.insert(Cheatcodes::new(self.ffi));
        }
        stack
    }
}
