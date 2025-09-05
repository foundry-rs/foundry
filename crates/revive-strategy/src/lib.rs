//! This crate provides the Revive strategy for the Foundry EVM ExecutorStrategy.
//!
//! It is designed to work with the Revive runtime, allowing for the execution of smart contracts
//! in a Polkadot environment.
//!
//! It is heavily inspired from <https://github.com/matter-labs/foundry-zksync/tree/main/crates/strategy/zksync>
use foundry_evm::executors::ExecutorStrategy;
use polkadot_sdk::{
    sp_core::{self, H160},
    sp_io,
    sp_state_machine::InMemoryBackend,
};
use revive_env::ExtBuilder;

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
            runner: Box::leak(Box::new(ReviveExecutorStrategyRunner::new())),
            context: Box::new(ReviveExecutorStrategyContext::new(resolc_startup)),
        }
    }
}

// TODO: rewrite this to something proper rather than a thread local variable
std::thread_local! {
    pub static TEST_EXTERNALITIES: std::cell::RefCell<sp_io::TestExternalities> = std::cell::RefCell::new(ExtBuilder::default()
    .balance_genesis_config(vec![(H160::from_low_u64_be(1), 1000)])
    .build());

    pub static CHECKPOINT : std::cell::RefCell<InMemoryBackend<sp_core::Blake2Hasher> > = panic!("not set");
}

fn execute_with_externalities<R, F: FnOnce(&mut sp_io::TestExternalities) -> R>(f: F) -> R {
    TEST_EXTERNALITIES.with_borrow_mut(f)
}

pub fn with_externalities<R, F: FnOnce() -> R>(mut backend: Backend, f: F) -> R {
    let mut test_externalities = ExtBuilder::default().build();
    std::mem::swap(&mut test_externalities.backend, &mut backend.0);
    TEST_EXTERNALITIES.set(test_externalities);
    f()
}

fn save_checkpoint() {
    TEST_EXTERNALITIES.with_borrow_mut(|f| CHECKPOINT.set(f.as_backend()))
}

fn return_to_checkpoint() {
    let mut test_externalities = ExtBuilder::default().build();
    let mut backend = CHECKPOINT.take();
    std::mem::swap(&mut test_externalities.backend, &mut backend);

    TEST_EXTERNALITIES.set(test_externalities)
}

#[derive(Clone)]
pub struct Backend(InMemoryBackend<sp_core::Blake2Hasher>);

impl Backend {
    /// Get the backend of test_externalities
    pub fn get() -> Self {
        TEST_EXTERNALITIES.with_borrow_mut(|f| Self(f.as_backend()))
    }
}
