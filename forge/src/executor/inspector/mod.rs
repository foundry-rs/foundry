#[macro_use]
mod macros;

mod logs;
pub use logs::LogCollector;

use ethers::abi::RawLog;

// TODO: Move
#[derive(Default, Debug)]
pub struct ExecutorState {
    pub logs: Vec<RawLog>,
}

impl ExecutorState {
    pub fn new() -> Self {
        Self::default()
    }
}
