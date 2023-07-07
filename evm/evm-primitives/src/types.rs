use ethers_core::{
    abi::{Abi, Function},
    types::{Address, Bytes},
};
use hashbrown::HashMap;
use parking_lot::Mutex;
use revm::primitives::{Account, B160};
use std::{collections::BTreeMap, sync::Arc};

/// A mapping of addresses to their changed state.
pub type StateChangeset = HashMap<B160, Account>;

/// (Sender, (TargetContract, Calldata))
pub type BasicTxDetails = (Address, (Address, Bytes));

/// Targeted contracts for fuzzing
pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;
/// Identified Contracts for a fuzz run
pub type FuzzRunIdentifiedContracts = Arc<Mutex<TargetedContracts>>;
