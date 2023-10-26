//! EVM inspectors.

pub use foundry_evm_coverage::CoverageCollector;
pub use foundry_evm_fuzz::Fuzzer;
pub use foundry_evm_traces::Tracer;

macro_rules! try_or_continue {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return InstructionResult::Continue,
        }
    };
}

mod access_list;
pub use access_list::AccessListTracer;

#[allow(unreachable_pub)]
pub mod cheatcodes;
pub use cheatcodes::{Cheatcodes, CheatsConfig};

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
