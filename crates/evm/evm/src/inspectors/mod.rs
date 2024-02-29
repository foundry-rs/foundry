//! EVM inspectors.

pub use foundry_cheatcodes::{self as cheatcodes, Cheatcodes, CheatsConfig};
pub use foundry_evm_coverage::CoverageCollector;
pub use foundry_evm_fuzz::Fuzzer;
pub use foundry_evm_traces::{StackSnapshotType, TracingInspector, TracingInspectorConfig};

mod access_list;
pub use access_list::AccessListTracer;

mod chisel_state;
pub use chisel_state::ChiselState;

mod debugger;
pub use debugger::Debugger;

mod logs;
pub use logs::LogCollector;

mod printer;
pub use printer::TracePrinter;

mod stack;
pub use stack::{InspectorData, InspectorStack, InspectorStackBuilder};
