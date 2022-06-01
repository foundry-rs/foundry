mod local;
pub use local::LocalTraceIdentifier;

mod etherscan;
pub use etherscan::EtherscanIdentifier;

use ethers::abi::{Abi, Address};
use std::borrow::Cow;

/// An address identity
pub struct AddressIdentity<'a> {
    /// The address this identity belongs to
    pub address: Address,
    /// The label for the address
    pub label: Option<String>,
    /// The contract this address represents
    ///
    /// Note: This may be in the format `"<artifact>:<contract>"`.
    pub contract: Option<String>,
    /// The ABI of the contract at this address
    pub abi: Option<Cow<'a, Abi>>,
}

/// Trace identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub trait TraceIdentifier {
    // TODO: Update docs
    /// Attempts to identify an address in one or more call traces.
    #[allow(clippy::type_complexity)]
    fn identify_addresses(
        &self,
        addresses: Vec<(&Address, Option<&Vec<u8>>)>,
    ) -> Vec<AddressIdentity>;
}
