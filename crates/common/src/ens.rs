//! ENS Name resolving utilities.
#![allow(missing_docs)]
use alloy_primitives::{address, keccak256, Address, B256};
use alloy_provider::{Network, Provider};
use alloy_sol_types::sol;
use alloy_transport::Transport;
use async_trait::async_trait;
use std::str::FromStr;

use self::EnsResolver::EnsResolverInstance;

// ENS Registry and Resolver contracts.
sol! {
    #[sol(rpc)]
    // ENS Registry contract.
    contract EnsRegistry {
        /// Returns the resolver for the specified node.
        function resolver(bytes32 node) view returns (address);
    }

    #[sol(rpc)]
    // ENS Resolver interface.
    contract EnsResolver {
        // Returns the address associated with the specified node.
        function addr(bytes32 node) view returns (address);

        // Returns the name associated with an ENS node, for reverse records.
        function name(bytes32 node) view returns (string);
    }
}

/// ENS registry address (`0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e`)
pub const ENS_ADDRESS: Address = address!("00000000000C2E074eC69A0dFb2997BA6C7d2e1e");

pub const ENS_REVERSE_REGISTRAR_DOMAIN: &str = "addr.reverse";

/// Error type for ENS resolution.
#[derive(Debug, thiserror::Error)]
pub enum EnsResolutionError {
    /// Failed to resolve ENS registry.
    #[error("Failed to get resolver from ENS registry: {0}")]
    EnsRegistryResolutionFailed(String),
    /// Failed to resolve ENS name to an address.
    #[error("Failed to resolve ENS name to an address: {0}")]
    EnsResolutionFailed(String),
}

/// ENS name or Ethereum Address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameOrAddress {
    /// An ENS Name (format does not get checked)
    Name(String),
    /// An Ethereum Address
    Address(Address),
}

impl NameOrAddress {
    /// Resolves the name to an Ethereum Address.
    pub async fn resolve<N: Network, T: Transport + Clone, P: Provider<T, N>>(
        &self,
        provider: &P,
    ) -> Result<Address, EnsResolutionError> {
        match self {
            NameOrAddress::Name(name) => provider.resolve_name(name).await,
            NameOrAddress::Address(addr) => Ok(*addr),
        }
    }
}

impl From<String> for NameOrAddress {
    fn from(name: String) -> Self {
        NameOrAddress::Name(name)
    }
}

impl From<&String> for NameOrAddress {
    fn from(name: &String) -> Self {
        NameOrAddress::Name(name.clone())
    }
}

impl From<Address> for NameOrAddress {
    fn from(addr: Address) -> Self {
        NameOrAddress::Address(addr)
    }
}

impl FromStr for NameOrAddress {
    type Err = <Address as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(addr) = Address::from_str(s) {
            Ok(NameOrAddress::Address(addr))
        } else {
            Ok(NameOrAddress::Name(s.to_string()))
        }
    }
}

#[async_trait]
pub trait ProviderEnsExt<T: Transport + Clone, N: Network, P: Provider<T, N>> {
    async fn get_resolver(&self) -> Result<EnsResolverInstance<T, &P, N>, EnsResolutionError>;

    async fn resolve_name(&self, name: &str) -> Result<Address, EnsResolutionError> {
        let node = namehash(name);
        let addr = self
            .get_resolver()
            .await?
            .addr(node)
            .call()
            .await
            .map_err(|err| EnsResolutionError::EnsResolutionFailed(err.to_string()))?
            ._0;

        Ok(addr)
    }

    async fn lookup_address(&self, address: Address) -> Result<String, EnsResolutionError> {
        let node = namehash(&reverse_address(address));
        let name = self
            .get_resolver()
            .await?
            .name(node)
            .call()
            .await
            .map_err(|err| EnsResolutionError::EnsResolutionFailed(err.to_string()))?
            ._0;

        Ok(name)
    }
}

#[async_trait]
impl<T, N, P> ProviderEnsExt<T, N, P> for P
where
    P: Provider<T, N>,
    N: Network,
    T: Transport + Clone,
{
    async fn get_resolver(&self) -> Result<EnsResolverInstance<T, &P, N>, EnsResolutionError> {
        let registry = EnsRegistry::new(ENS_ADDRESS, self);
        let address = registry
            .resolver(namehash("eth"))
            .call()
            .await
            .map_err(|err| EnsResolutionError::EnsRegistryResolutionFailed(err.to_string()))?
            ._0;

        Ok(EnsResolverInstance::new(address, self))
    }
}

/// Returns the ENS namehash as specified in [EIP-137](https://eips.ethereum.org/EIPS/eip-137)
pub fn namehash(name: &str) -> B256 {
    if name.is_empty() {
        return B256::ZERO
    }

    // Remove the variation selector U+FE0F
    let name = name.replace('\u{fe0f}', "");

    // Generate the node starting from the right
    name.rsplit('.')
        .fold([0u8; 32], |node, label| *keccak256([node, *keccak256(label.as_bytes())].concat()))
        .into()
}

/// Returns the reverse-registrar name of an address.
pub fn reverse_address(addr: Address) -> String {
    format!("{addr:?}.{ENS_REVERSE_REGISTRAR_DOMAIN}")[2..].to_string()
}

#[cfg(test)]
mod test {
    use super::*;

    fn assert_hex(hash: B256, val: &str) {
        assert_eq!(hash.0.to_vec(), hex::decode(val).unwrap());
    }

    #[test]
    fn test_namehash() {
        for (name, expected) in &[
            ("", "0000000000000000000000000000000000000000000000000000000000000000"),
            ("foo.eth", "de9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f"),
            ("eth", "0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae"),
            ("alice.eth", "0x787192fc5378cc32aa956ddfdedbf26b24e8d78e40109add0eea2c1a012c3dec"),
            ("ret↩️rn.eth", "0x3de5f4c02db61b221e7de7f1c40e29b6e2f07eb48d65bf7e304715cd9ed33b24"),
        ] {
            assert_hex(namehash(name), expected);
        }
    }
}
