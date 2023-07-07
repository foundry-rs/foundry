use ethers::types::{Address, Bytes};

pub mod random_call_generator;

mod filters;
pub use filters::{ArtifactFilters, SenderFilters};

/// (Sender, (TargetContract, Calldata))
pub type BasicTxDetails = (Address, (Address, Bytes));
