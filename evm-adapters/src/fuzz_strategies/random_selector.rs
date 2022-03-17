use ethers_core::rand::prelude::IteratorRandom;
use proptest::prelude::Arbitrary;
use proptest::test_runner::TestRng;
use proptest::num::u64::BinarySearch;

use proptest::{
    strategy::{NewTree, Strategy, ValueTree},
    test_runner::TestRunner,
};

#[derive(Clone, Debug)]
pub struct Selector {
    rng: TestRng,
}

/// Strategy to create `Selector`s.
///
/// Created via `any::<Selector>()`.
#[derive(Debug)]
pub struct SelectorStrategy {
    _nonexhaustive: (),
}

/// `ValueTree` corresponding to `SelectorStrategy`.
#[derive(Debug)]
pub struct SelectorValueTree {
    rng: TestRng,
    reverse_bias_increment: BinarySearch,
}

impl SelectorStrategy {
    pub(crate) fn new() -> Self {
        SelectorStrategy { _nonexhaustive: () }
    }
}

impl Strategy for SelectorStrategy {
    type Tree = SelectorValueTree;
    type Value = Selector;

    fn new_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        Ok(SelectorValueTree {
            rng: runner.new_rng(),
            reverse_bias_increment: BinarySearch::new(u64::MAX),
        })
    }
}

impl ValueTree for SelectorValueTree {
    type Value = Selector;

    fn current(&self) -> Selector {
        Selector {
            rng: self.rng.clone()
        }
    }

    fn simplify(&mut self) -> bool {
        self.reverse_bias_increment.simplify()
    }

    fn complicate(&mut self) -> bool {
        self.reverse_bias_increment.complicate()
    }
}

impl Selector {
    /// Pick a random element from iterable `it`.
    ///
    /// The selection is unaffected by the elements themselves, and is
    /// dependent only on the actual length of `it`.
    ///
    /// `it` is always iterated completely.
    ///
    /// ## Panics
    ///
    /// Panics if `it` has no elements.
    pub fn select<'a, T: IteratorRandom>(&self, it: T) -> <T as Iterator>::Item
    where
        T: Iterator<Item = &'a [u8; 32]>
    {
        self.try_select(it).expect("select from empty iterator")
    }

    /// Pick a random element from iterable `it`.
    ///
    /// Returns `None` if `it` is empty.
    ///
    /// The selection is unaffected by the elements themselves, and is
    /// dependent only on the actual length of `it`.
    pub fn try_select<'a, T: IteratorRandom>(&self, it: T) -> Option<<T as Iterator>::Item> 
    where
        T: Iterator<Item = &'a [u8; 32]>
    {
        let mut rng = self.rng.clone();
        it.choose(&mut rng)
    }
}

impl Arbitrary for Selector {
    type Parameters = ();

    type Strategy = SelectorStrategy;

    fn arbitrary_with(_: ()) -> SelectorStrategy {
        SelectorStrategy::new()
    }
}