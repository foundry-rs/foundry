//! ENS Name resolving utilities.

#![allow(missing_docs)]

use self::EnsResolver::EnsResolverInstance;
use alloy_primitives::{address, Address, Keccak256, B256};
use alloy_provider::{Network, Provider};
use alloy_sol_types::sol;
use alloy_transport::Transport;
use async_trait::async_trait;
use std::{borrow::Cow, str::FromStr};

// ENS Registry and Resolver contracts.
sol! {
    /// ENS Registry contract.
    #[sol(rpc)]
    contract EnsRegistry {
        /// Returns the resolver for the specified node.
        function resolver(bytes32 node) view returns (address);
    }

    /// ENS Resolver interface.
    #[sol(rpc)]
    contract EnsResolver {
        /// Returns the address associated with the specified node.
        function addr(bytes32 node) view returns (address);

        /// Returns the name associated with an ENS node, for reverse records.
        function name(bytes32 node) view returns (string);
    }
}

/// ENS registry address (`0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e`)
pub const ENS_ADDRESS: Address = address!("00000000000C2E074eC69A0dFb2997BA6C7d2e1e");

pub const ENS_REVERSE_REGISTRAR_DOMAIN: &str = "addr.reverse";

/// Error type for ENS resolution.
#[derive(Debug, thiserror::Error)]
pub enum EnsError {
    /// Failed to get resolver from the ENS registry.
    #[error("Failed to get resolver from the ENS registry: {0}")]
    Resolver(alloy_contract::Error),
    /// Failed to get resolver from the ENS registry.
    #[error("ENS resolver not found for name {0:?}")]
    ResolverNotFound(String),
    /// Failed to lookup ENS name from an address.
    #[error("Failed to lookup ENS name from an address: {0}")]
    Lookup(alloy_contract::Error),
    /// Failed to resolve ENS name to an address.
    #[error("Failed to resolve ENS name to an address: {0}")]
    Resolve(alloy_contract::Error),
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
    ) -> Result<Address, EnsError> {
        match self {
            Self::Name(name) => provider.resolve_name(name).await,
            Self::Address(addr) => Ok(*addr),
        }
    }
}

impl From<String> for NameOrAddress {
    fn from(name: String) -> Self {
        Self::Name(name)
    }
}

impl From<&String> for NameOrAddress {
    fn from(name: &String) -> Self {
        Self::Name(name.clone())
    }
}

impl From<Address> for NameOrAddress {
    fn from(addr: Address) -> Self {
        Self::Address(addr)
    }
}

impl FromStr for NameOrAddress {
    type Err = <Address as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(addr) = Address::from_str(s) {
            Ok(Self::Address(addr))
        } else {
            Ok(Self::Name(s.to_string()))
        }
    }
}

/// Extension trait for ENS contract calls.
#[async_trait]
pub trait ProviderEnsExt<T: Transport + Clone, N: Network, P: Provider<T, N>> {
    /// Returns the resolver for the specified node. The `&str` is only used for error messages.
    async fn get_resolver(
        &self,
        node: B256,
        error_name: &str,
    ) -> Result<EnsResolverInstance<T, &P, N>, EnsError>;

    /// Performs a forward lookup of an ENS name to an address.
    async fn resolve_name(&self, name: &str) -> Result<Address, EnsError> {
        let node = namehash(name);
        let resolver = self.get_resolver(node, name).await?;
        let addr = resolver
            .addr(node)
            .call()
            .await
            .map_err(EnsError::Resolve)
            .inspect_err(|e| eprintln!("{e:?}"))?
            ._0;
        Ok(addr)
    }

    /// Performs a reverse lookup of an address to an ENS name.
    async fn lookup_address(&self, address: &Address) -> Result<String, EnsError> {
        let name = reverse_address(address);
        let node = namehash(&name);
        let resolver = self.get_resolver(node, &name).await?;
        let name = resolver.name(node).call().await.map_err(EnsError::Lookup)?._0;
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
    async fn get_resolver(
        &self,
        node: B256,
        error_name: &str,
    ) -> Result<EnsResolverInstance<T, &P, N>, EnsError> {
        let registry = EnsRegistry::new(ENS_ADDRESS, self);
        let address = registry.resolver(node).call().await.map_err(EnsError::Resolver)?._0;
        if address == Address::ZERO {
            return Err(EnsError::ResolverNotFound(error_name.to_string()));
        }
        Ok(EnsResolverInstance::new(address, self))
    }
}

/// Returns the ENS namehash as specified in [EIP-137](https://eips.ethereum.org/EIPS/eip-137)
pub fn namehash(name: &str) -> B256 {
    if name.is_empty() {
        return B256::ZERO
    }

    // Remove the variation selector `U+FE0F` if present.
    const VARIATION_SELECTOR: char = '\u{fe0f}';
    let name = if name.contains(VARIATION_SELECTOR) {
        Cow::Owned(name.replace(VARIATION_SELECTOR, ""))
    } else {
        Cow::Borrowed(name)
    };

    // Generate the node starting from the right.
    // This buffer is `[node @ [u8; 32], label_hash @ [u8; 32]]`.
    let mut buffer = [0u8; 64];
    for label in name.rsplit('.') {
        // node = keccak256([node, keccak256(label)])

        // Hash the label.
        let mut label_hasher = Keccak256::new();
        label_hasher.update(label.as_bytes());
        label_hasher.finalize_into(&mut buffer[32..]);

        // Hash both the node and the label hash, writing into the node.
        let mut buffer_hasher = Keccak256::new();
        buffer_hasher.update(buffer.as_slice());
        buffer_hasher.finalize_into(&mut buffer[..32]);
    }
    buffer[..32].try_into().unwrap()
}

/// Returns the reverse-registrar name of an address.
pub fn reverse_address(addr: &Address) -> String {
    format!("{addr:x}.{ENS_REVERSE_REGISTRAR_DOMAIN}")
}

#[cfg(test)]
mod test {
    use super::*;
    use alloy_primitives::hex;

    fn assert_hex(hash: B256, val: &str) {
        assert_eq!(hash.0[..], hex::decode(val).unwrap()[..]);
    }

    #[test]
    fn test_namehash() {
        for (name, expected) in &[
            ("", "0x0000000000000000000000000000000000000000000000000000000000000000"),
            ("eth", "0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae"),
            ("foo.eth", "0xde9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f"),
            ("alice.eth", "0x787192fc5378cc32aa956ddfdedbf26b24e8d78e40109add0eea2c1a012c3dec"),
            ("ret↩️rn.eth", "0x3de5f4c02db61b221e7de7f1c40e29b6e2f07eb48d65bf7e304715cd9ed33b24"),
        ] {
            assert_hex(namehash(name), expected);
        }
    }

    #[test]
    fn test_reverse_address() {
        for (addr, expected) in [
            (
                "0x314159265dd8dbb310642f98f50c066173c1259b",
                "314159265dd8dbb310642f98f50c066173c1259b.addr.reverse",
            ),
            (
                "0x28679A1a632125fbBf7A68d850E50623194A709E",
                "28679a1a632125fbbf7a68d850e50623194a709e.addr.reverse",
            ),
        ] {
            assert_eq!(reverse_address(&addr.parse().unwrap()), expected, "{addr}");
        }
    }
}
