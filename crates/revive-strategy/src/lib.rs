//! This crate provides the Revive strategy for the Foundry EVM ExecutorStrategy.
//!
//! It is designed to work with the Revive runtime, allowing for the execution of smart contracts
//! in a Polkadot environment.
//!
//! It is heavily inspired from <https://github.com/matter-labs/foundry-zksync/tree/main/crates/strategy/zksync>
use foundry_evm::executors::ExecutorStrategy;

use crate::executor::{
    context::ReviveExecutorStrategyContext, runner::ReviveExecutorStrategyRunner,
};

mod backend;
mod cheatcodes;
mod executor;
mod tracing;

pub use tracing::trace;

/// Create Revive strategy for [ExecutorStrategy].
pub trait ReviveExecutorStrategyBuilder {
    /// Create new revive strategy.
    fn new_revive(resolc_startup: bool) -> Self;
}

impl ReviveExecutorStrategyBuilder for ExecutorStrategy {
    fn new_revive(resolc_startup: bool) -> Self {
        Self {
            // TODO: we need to spawn test externalities for each test
            runner: Box::leak(Box::new(ReviveExecutorStrategyRunner::new())),
            context: Box::new(ReviveExecutorStrategyContext::new(resolc_startup)),
        }
    }
}
