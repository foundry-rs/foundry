//! Common EVM-related types shared across crates.

use alloy_primitives::{Address, map::HashMap};

/// Map keyed by breakpoints char to their location (contract address, pc)
pub type Breakpoints = HashMap<char, (Address, usize)>;
