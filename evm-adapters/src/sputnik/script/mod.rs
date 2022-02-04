//! support for writing scripts with solidity

pub mod fs;

// pub type TestSputnikVM<'a, B> = Executor<
//     // state
//     CheatcodeStackState<'a, B>,
//     // actual stack executor
//     CheatcodeStackExecutor<'a, 'a, B, BTreeMap<Address, PrecompileFn>>,
// >;

// /// A [`MemoryStackStateOwned`] state instantiated over a [`CheatcodeBackend`]
// pub type CheatcodeStackState<'a, B> = MemoryStackStateOwned<'a, CheatcodeBackend<B>>;

// /// A [`CheatcodeHandler`] which uses a [`CheatcodeStackState`] to store its state and a
// /// [`StackExecutor`] for executing transactions.
// pub type CheatcodeStackExecutor<'a, 'b, B, P> =
// CheatcodeHandler<StackExecutor<'a, 'b, CheatcodeStackState<'a, B>, P>>;
