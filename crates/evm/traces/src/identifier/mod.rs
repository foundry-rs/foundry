use alloy_json_abi::JsonAbi as Abi;
use alloy_primitives::Address;
use foundry_compilers::ArtifactId;
use std::borrow::Cow;

mod local;
pub use local::LocalTraceIdentifier;

mod etherscan;
pub use etherscan::EtherscanIdentifier;

mod signatures;
pub use signatures::{SignaturesIdentifier, SingleSignaturesIdentifier};

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
    /// The artifact ID of the contract, if any.
    pub artifact_id: Option<ArtifactId>,
}

/// Trace identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub trait TraceIdentifier {
    /// Attempts to identify an address in one or more call traces.
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>;
}
