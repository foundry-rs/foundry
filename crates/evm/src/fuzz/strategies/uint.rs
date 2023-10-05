use ethers::core::rand::Rng;
use proptest::{
    strategy::{NewTree, Strategy, ValueTree},
    test_runner::TestRunner,
};

use alloy_primitives::U256;

/// Value tree for unsigned ints (up to uint256).
/// This is very similar to [proptest::BinarySearch]
pub struct UintValueTree {
    /// Lower base
    lo: U256,
    /// Current value
    curr: U256,
    /// Higher base
    hi: U256,
    /// If true cannot be simplified or complexified
    fixed: bool,
}
impl UintValueTree {
    /// Create a new tree
    /// # Arguments
    /// * `start` - Starting value for the tree
    /// * `fixed` - If `true` the tree would only contain one element and won't be simplified.
    fn new(start: U256, fixed: bool) -> Self {
        Self { lo: U256::ZERO, curr: start, hi: start, fixed }
    }

    fn reposition(&mut self) -> bool {
        let interval = self.hi - self.lo;
        let new_mid = self.lo + interval / U256::from(2);

        if new_mid == self.curr {
            false
        } else {
            self.curr = new_mid;
            true
        }
    }
}
impl ValueTree for UintValueTree {
    type Value = U256;
    fn current(&self) -> Self::Value {
        self.curr
    }
    fn simplify(&mut self) -> bool {
        if self.fixed || (self.hi <= self.lo) {
            return false
        }
        self.hi = self.curr;
        self.reposition()
    }
    fn complicate(&mut self) -> bool {
        if self.fixed || (self.hi <= self.lo) {
            return false
        }

        self.lo = self.curr + U256::from(1);
        self.reposition()
    }
}
/// Value tree for unsigned ints (up to uint256).
/// The strategy combines 3 different strategies, each assigned a specific weight:
/// 1. Generate purely random value in a range. This will first choose bit size uniformly (up `bits`
/// param). Then generate a value for this bit size.
/// 2. Generate a random value around the edges (+/- 3 around 0 and max possible value)
/// 3. Generate a value from a predefined fixtures set
#[derive(Debug)]
pub struct UintStrategy {
    /// Bit size of uint (e.g. 256)
    bits: usize,
    /// A set of fixtures to be generated
    fixtures: Vec<U256>,
    /// The weight for edge cases (+/- 3 around 0 and max possible value)
    edge_weight: usize,
    /// The weight for fixtures
    fixtures_weight: usize,
    /// The weight for purely random values
    random_weight: usize,
}
impl UintStrategy {
    /// Create a new strategy.
    /// #Arguments
    /// * `bits` - Size of uint in bits
    /// * `fixtures` - A set of fixed values to be generated (according to fixtures weight)
    pub fn new(bits: usize, fixtures: Vec<U256>) -> Self {
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
        // Choose if we want values around 0 or max
        let is_min = rng.gen_bool(0.5);
        let offset = U256::from(rng.gen_range(0..4));
        let max = if self.bits < 256 {
            (U256::from(1u8).rotate_left(self.bits)) - U256::from(1)
        } else {
            U256::MAX
        };
        let start = if is_min { offset } else { max - offset };
        Ok(UintValueTree::new(start, false))
    }
    fn generate_fixtures_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        // generate edge cases if there's no fixtures
        if self.fixtures.is_empty() {
            return self.generate_edge_tree(runner)
        }
        let idx = runner.rng().gen_range(0..self.fixtures.len());
        Ok(UintValueTree::new(self.fixtures[idx], false))
    }
    fn generate_random_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let rng = runner.rng();
        // generate random number of bits uniformly
        let bits = rng.gen_range(0..=self.bits);
        // init 2 128-bit randoms
        let mut higher: u128 = rng.gen_range(0..=u128::MAX);
        let mut lower: u128 = rng.gen_range(0..=u128::MAX);
        // cut 2 randoms according to bits size
        match bits {
            x if x < 128 => {
                lower &= (1u128 << x) - 1;
                higher = 0;
            }
            x if (128..256).contains(&x) => higher &= (1u128 << (x - 128)) - 1,
            _ => {}
        };
        // init U256 from 2 randoms
        let mut inner: [u64; 4] = [0; 4];
        let mask64 = (1 << 65) - 1;
        inner[0] = (lower & mask64) as u64;
        inner[1] = (lower >> 64) as u64;
        inner[2] = (higher & mask64) as u64;
        inner[3] = (higher >> 64) as u64;
        let start: U256 = U256::from_limbs(inner);

        Ok(UintValueTree::new(start, false))
    }
}
impl Strategy for UintStrategy {
    type Tree = UintValueTree;
    type Value = U256;
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
