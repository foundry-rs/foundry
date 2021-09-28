use std::ops::Deref;

use super::Executor;
use sputnik::{
    backend::Backend,
    executor::{MemoryStackState, Precompile, StackExecutor, StackState, StackSubstateMetadata},
    Config,
};
use std::marker::PhantomData;

struct CheatcodeStackExecutor<'backend, 'config, S, B> {
    executor: StackExecutor<'config, S>,
    backend: &'backend B,
}

impl<'b, S, B> CheatcodeStackExecutor<'b, 'b, S, B>
where
    S: StackState<'b>,
{
    pub fn new_with_precompile(
        backend: &'b B,
        state: S,
        config: &'b Config,
        precompile: Precompile,
    ) -> Self {
        Self { executor: StackExecutor::new_with_precompile(state, config, precompile), backend }
    }
}

impl<'backend, 'config, S, B> Deref for CheatcodeStackExecutor<'backend, 'config, S, B> {
    type Target = StackExecutor<'config, S>;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

// Concrete implementation over the in-memory backend
impl<'a, B: Backend>
    Executor<
        MemoryStackState<'a, 'a, B>,
        Config,
        CheatcodeStackExecutor<'a, 'a, MemoryStackState<'a, 'a, B>, B>,
    >
{
    /// Given a gas limit, vm version, initial chain configuration and initial state
    // TOOD: See if we can make lifetimes better here
    pub fn new_with_cheatcode(gas_limit: u64, config: &'a Config, backend: &'a B) -> Self {
        // setup gasometer
        let metadata = StackSubstateMetadata::new(gas_limit, config);
        // setup state
        let state = MemoryStackState::new(metadata, backend);
        // setup executor
        let executor =
            CheatcodeStackExecutor::new_with_precompile(backend, state, config, Default::default());

        Self { executor, gas_limit, marker: PhantomData }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intercepts_cheat_code() {}
}
