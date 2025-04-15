//! EVM inspectors.

pub mod cheatcodes;
pub mod chisel_state;
pub mod coverage;
pub mod debugger;
pub mod fuzz;
pub mod logs;
pub mod printer;
pub mod script;
pub mod stack;
pub mod tracer;

// Re-export inspectors
pub use cheatcodes::{Cheatcodes, CheatsConfig};
pub use chisel_state::ChiselState;
pub use coverage::CoverageCollector;
pub use debugger::Debugger;
pub use fuzz::Fuzzer;
pub use logs::LogCollector;
pub use printer::TracePrinter;
pub use script::ScriptExecutionInspector;
pub use stack::{InspectorData, InspectorStack, InspectorStackBuilder};
pub use tracer::TracingInspector;
