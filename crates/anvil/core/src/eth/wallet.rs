use alloy_primitives::{map::HashMap, Address, ChainId};
use serde::{Deserialize, Serialize};

/// The capability to perform [EIP-7702][eip-7702] delegations, sponsored by the sequencer.
///
/// The sequencer will only perform delegations, and act on behalf of delegated accounts, if the
/// account delegates to one of the addresses specified within this capability.
///
/// [eip-7702]: https://eips.ethereum.org/EIPS/eip-7702
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct DelegationCapability {
    /// A list of valid delegation contracts.
    pub addresses: Vec<Address>,
}

/// Wallet capabilities for a specific chain.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct Capabilities {
    /// The capability to delegate.
    pub delegation: DelegationCapability,
}

/// A map of wallet capabilities per chain ID.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize, Default)]
pub struct WalletCapabilities(pub HashMap<ChainId, Capabilities>);

impl WalletCapabilities {
    /// Get the capabilities of the wallet API for the specified chain ID.
    pub fn get(&self, chain_id: ChainId) -> Option<&Capabilities> {
        self.0.get(&chain_id)
    }
}
