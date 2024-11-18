//! EVM inspectors.

pub use foundry_cheatcodes::{self as cheatcodes, Cheatcodes, CheatsConfig};
pub use foundry_evm_coverage::CoverageCollector;
pub use foundry_evm_fuzz::Fuzzer;
pub use foundry_evm_traces::{StackSnapshotType, TracingInspector, TracingInspectorConfig};

pub use revm_inspectors::access_list::AccessListInspector;

mod chisel_state;
pub use chisel_state::ChiselState;

mod logs;
pub use logs::LogCollector;

mod stack;
pub use stack::{InspectorData, InspectorStack, InspectorStackBuilder};
