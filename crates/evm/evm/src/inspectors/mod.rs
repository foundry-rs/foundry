//! EVM inspectors.

pub use foundry_cheatcodes::{self as cheatcodes, Cheatcodes, CheatsConfig};
pub use foundry_evm_coverage::{
    CmpOperands, EdgeCovConfig, EdgeCovHit, EdgeCovInspector, EdgeCovKind, EdgeCoverage,
    EdgeIndexMap, EdgeKey, LineCoverageCollector,
};
pub use foundry_evm_fuzz::Fuzzer;
pub use foundry_evm_traces::{StackSnapshotType, TracingInspector, TracingInspectorConfig};

pub use evm2_inspectors::access_list::AccessListInspector;

mod custom_printer;
pub use custom_printer::CustomPrintTracer;

mod chisel_state;
pub use chisel_state::ChiselState;

mod debug_recorder;
pub use debug_recorder::DebugRecorder;

mod logs;
pub use logs::LogCollector;

mod script;
pub use script::ScriptExecutionInspector;

mod stack;
pub use stack::{InspectorData, InspectorStack, InspectorStackBuilder, InspectorStackRefMut};

mod revert_diagnostic;
pub use revert_diagnostic::RevertDiagnostic;

mod tempo_labels;
pub(crate) use tempo_labels::TempoLabels;
