mod local;
pub use local::LocalTraceIdentifier;

mod etherscan;
pub use etherscan::EtherscanIdentifier;

use ethers::abi::{Abi, Address};
use std::borrow::Cow;

/// Trace identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub trait TraceIdentifier {
    // TODO: Update docs
    /// Attempts to identify an address in one or more call traces.
    ///
    /// The tuple is of the format `(contract, label, abi)`, where `contract` is intended to be of
    /// the format `"<artifact>:<contract>"`, e.g. `"Foo.json:Foo"`.
    fn identify_addresses(
        &self,
        addresses: Vec<(&Address, Option<&Vec<u8>>)>,
    ) -> Vec<(Address, Option<String>, Option<String>, Option<Cow<Abi>>)>;
}
