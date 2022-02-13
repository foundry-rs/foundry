use std::{collections::HashSet, cell::RefCell, rc::Rc};
use ethers_core::rand::Rng;
use proptest::{strategy::{Strategy, ValueTree, NewTree}, test_runner::TestRunner};

use ethers::{
    types::{Address, Bytes, I256, U256},
};

pub struct UintValueTree {
    lo: U256,
    curr: U256,
    hi: U256,
    fixed: bool
}

impl UintValueTree {
    fn new(start: U256, fixed: bool) -> Self {
        Self {
            lo: 0.into(),
            curr: start,
            hi: start,
            fixed
        }
    }

    fn reposition(&mut self) -> bool {
        let interval = self.hi - self.lo;
        let new_mid = self.lo + interval / 2;

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
        return self.curr;
    }

    fn simplify(&mut self) -> bool {
        if self.fixed || (self.hi <= self.lo){
            return false;
        }

        self.hi = self.curr;
        self.reposition()
    }

    fn complicate(&mut self) -> bool {
        if self.fixed || (self.hi <= self.lo){
            return false;
        }

        self.lo = self.curr + 1;
        self.reposition()
    }
}

#[derive(Debug)]
pub struct UintStrategy {
    bits: usize,
    fixtures: Vec<U256>,
    edge_weight: usize,
    fixtures_weight: usize,
    random_weight: usize,
}

impl UintStrategy {
    pub fn new(bits: usize) -> Self {
        Self {
            bits,
            fixtures: Vec::new(),
            edge_weight: 10usize,
            fixtures_weight: 40usize,
            random_weight: 50usize,
        }
    }

    fn fixtures(&self) -> &[U256] {
        &self.fixtures
    }

    fn set_fixtures(&mut self, fixtures: Vec<U256>) {
        self.fixtures = fixtures
    }

    fn generate_edge_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let rng = runner.rng();
        let is_min = rng.gen_bool(0.5);
        let offset = U256::from(rng.gen_range(0..3));
        let max = if self.bits < 256 {U256::from(1u8) << U256::from(self.bits) - 1} else {U256::MAX};
        let start = if is_min { offset } else { max - offset };
        Ok(UintValueTree::new(start, true))
    }

    fn generate_fixtures_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        if self.fixtures.len() == 0 {
            return self.generate_edge_tree(runner);
        }
        let idx = runner.rng().gen_range(0..self.fixtures.len());
        Ok(UintValueTree::new(self.fixtures[idx], true))
    }

    fn generate_random_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let rng = runner.rng();
        let bits = rng.gen_range(0..=self.bits);
        let mut higher: u128 = rng.gen_range(0..=u128::MAX);
        let mut lower: u128 = rng.gen_range(0..=u128::MAX);
        match bits {
            x if x < 128 => lower = lower & ((1u128 << x) - 1),
            x if (x >= 128) && (x < 256) => higher = higher & ((1u128 << (x - 128)) - 1),
            _ => {},
        };
        let mut inner: [u64; 4] = [0; 4];
        let mask64 = (1 << 65) - 1;
        inner[0] = (lower & mask64) as u64;
        inner[1] = (lower >> 64) as u64;
        inner[2] = (higher & mask64) as u64;
        inner[3] = (higher >> 64) as u64;
        let start: U256 = U256(inner);
        Ok(UintValueTree::new(start, false))
    }
}

impl Strategy for UintStrategy {
    type Tree = UintValueTree;
    type Value = U256;
    fn new_tree(&self, runner: &mut TestRunner) -> NewTree<Self> {
        let total_weight = self.random_weight + self.fixtures_weight + self.edge_weight;
        let bias = runner.rng().gen_range(0..total_weight);
        match bias {
            x if x < self.edge_weight => self.generate_edge_tree(runner),
            x if x < self.edge_weight + self.fixtures_weight => self.generate_fixtures_tree(runner),
            _ => self.generate_random_tree(runner)
        }
    }    
}
