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
mod executor;

/// Create Revive strategy for [ExecutorStrategy].
pub trait ReviveExecutorStrategyBuilder {
    /// Create new revive strategy.
    fn new_revive() -> Self;
}

impl ReviveExecutorStrategyBuilder for ExecutorStrategy {
    fn new_revive() -> Self {
        Self {
            runner: &ReviveExecutorStrategyRunner,
            context: Box::new(ReviveExecutorStrategyContext::default()),
        }
    }
}
