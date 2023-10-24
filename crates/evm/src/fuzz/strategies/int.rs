use std::ops::{Add, Shl, Sub};

use ethers::core::rand::Rng;
use proptest::{
    strategy::{NewTree, Strategy, ValueTree},
    test_runner::TestRunner,
};

use alloy_primitives::{Sign, I256, U256};

/// Value tree for signed ints (up to int256).
/// This is very similar to [proptest::BinarySearch]
pub struct IntValueTree {
    /// Lower base (by absolute value)
    lo: I256,
    /// Current value
    curr: I256,
    /// Higher base (by absolute value)
    hi: I256,
    /// If true cannot be simplified or complexified
    fixed: bool,
}
impl IntValueTree {
    /// Create a new tree
    /// # Arguments
    /// * `start` - Starting value for the tree
    /// * `fixed` - If `true` the tree would only contain one element and won't be simplified.
    fn new(start: I256, fixed: bool) -> Self {
        Self { lo: I256::ZERO, curr: start, hi: start, fixed }
    }

    fn reposition(&mut self) -> bool {
        let interval = self.hi - self.lo;
        let new_mid = self.lo + interval / I256::from_raw(U256::from(2));

        if new_mid == self.curr {
            false
        } else {
            self.curr = new_mid;
            true
        }
    }
    fn magnitude_greater(lhs: I256, rhs: I256) -> bool {
        if lhs.is_zero() {
            return false
        }
        (lhs > rhs) ^ (lhs.is_negative())
    }
}
impl ValueTree for IntValueTree {
    type Value = I256;
    fn current(&self) -> Self::Value {
        self.curr
    }
    fn simplify(&mut self) -> bool {
        if self.fixed || !IntValueTree::magnitude_greater(self.hi, self.lo) {
            return false
        }
        self.hi = self.curr;
        self.reposition()
    }
    fn complicate(&mut self) -> bool {
        if self.fixed || !IntValueTree::magnitude_greater(self.hi, self.lo) {
            return false
        }

        self.lo = self.curr + if self.hi.is_negative() { I256::MINUS_ONE } else { I256::ONE };

        self.reposition()
    }
}
/// Value tree for signed ints (up to int256).
/// The strategy combines 3 different strategies, each assigned a specific weight:
/// 1. Generate purely random value in a range. This will first choose bit size uniformly (up `bits`
/// param). Then generate a value for this bit size.
/// 2. Generate a random value around the edges (+/- 3 around min, 0 and max possible value)
/// 3. Generate a value from a predefined fixtures set
#[derive(Debug)]
pub struct IntStrategy {
    /// Bit size of int (e.g. 256)
    bits: usize,
    /// A set of fixtures to be generated
    fixtures: Vec<I256>,
    /// The weight for edge cases (+/- 3 around 0 and max possible value)
    edge_weight: usize,
    /// The weight for fixtures
    fixtures_weight: usize,
    /// The weight for purely random values
    random_weight: usize,
}
impl IntStrategy {
    /// Create a new strategy.
    /// #Arguments
    /// * `bits` - Size of uint in bits
    /// * `fixtures` - A set of fixed values to be generated (according to fixtures weight)
    pub fn new(bits: usize, fixtures: Vec<I256>) -> Self {
        Self {
            bits,
            fixtures,
            edge_weight: 10usize,
            fixtures_weight: 40usize,
            random_weight: 50usize,
        }
    }
    fn generate_edge_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let rng = runner.rng();

        let offset = I256::from_raw(U256::from(rng.gen_range(0..4)));
        let umax: U256 = (U256::from(1u8).shl(self.bits - 1)).sub(U256::from(1u8));
        // Choose if we want values around min, -0, +0, or max
        let kind = rng.gen_range(0..4);
        let start = match kind {
            0 => {
                I256::overflowing_from_sign_and_abs(Sign::Negative, umax.add(U256::from(1))).0 +
                    offset
            }
            1 => -offset - I256::ONE,
            2 => offset,
            3 => I256::overflowing_from_sign_and_abs(Sign::Positive, umax).0 - offset,
            _ => unreachable!(),
        };
        Ok(IntValueTree::new(start, false))
    }
    fn generate_fixtures_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        // generate edge cases if there's no fixtures
        if self.fixtures.is_empty() {
            return self.generate_edge_tree(runner)
        }
        let idx = runner.rng().gen_range(0..self.fixtures.len());
        Ok(IntValueTree::new(self.fixtures[idx], false))
    }
    fn generate_random_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let rng = runner.rng();
        // generate random number of bits uniformly
        let bits = rng.gen_range(0..=self.bits);

        if bits == 0 {
            return Ok(IntValueTree::new(I256::ZERO, false))
        }

        // init 2 128-bit randoms
        let mut higher: u128 = rng.gen_range(0..=u128::MAX);
        let mut lower: u128 = rng.gen_range(0..=u128::MAX);
        // cut 2 randoms according to bits size
        match bits - 1 {
            x if x < 128 => {
                lower &= (1u128 << x) - 1;
                higher = 0;
            }
            x if (128..256).contains(&x) => higher &= (1u128 << (x - 128)) - 1,
            _ => {}
        };

        // init I256 from 2 randoms
        let mut inner: [u64; 4] = [0; 4];
        let mask64 = (1 << 65) - 1;
        inner[0] = (lower & mask64) as u64;
        inner[1] = (lower >> 64) as u64;
        inner[2] = (higher & mask64) as u64;
        inner[3] = (higher >> 64) as u64;
        let sign = if rng.gen_bool(0.5) { Sign::Positive } else { Sign::Negative };
        // we have a small bias here, i.e. intN::min will never be generated
        // but it's ok since it's generated in `fn generate_edge_tree(...)`
        let (start, _) = I256::overflowing_from_sign_and_abs(sign, U256::from_limbs(inner));

        Ok(IntValueTree::new(start, false))
    }
}
impl Strategy for IntStrategy {
    type Tree = IntValueTree;
    type Value = I256;
    fn new_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let total_weight = self.random_weight + self.fixtures_weight + self.edge_weight;
        let bias = runner.rng().gen_range(0..total_weight);
        // randomly select one of 3 strategies
        match bias {
            x if x < self.edge_weight => self.generate_edge_tree(runner),
            x if x < self.edge_weight + self.fixtures_weight => self.generate_fixtures_tree(runner),
            _ => self.generate_random_tree(runner),
        }
    }
}
