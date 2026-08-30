use crate::opts::ForkContext;
use alloy_eips::{BlockId, BlockNumHash};
use alloy_primitives::{B256, BlockNumber, keccak256};
use std::fmt;

/// An exact fork snapshot resolved from a fully configured RPC source.
///
/// The snapshot binds three layers that must travel together: the source URL, request headers, and
/// JWT; the configured selector (`latest` or a block number); and the observed exact block (number
/// and hash) plus endpoint context. `latest` is retained as the configured selector, while `block`
/// is always exact. Reusing this value keeps preflight reads, environment reconstruction, cache
/// identity, and backend construction on the same remote state.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResolvedFork {
    source: ForkSource,
    selector: Option<BlockNumber>,
    block: BlockNumHash,
    context: ForkContext,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ForkSource {
    url: String,
    headers: Vec<String>,
    jwt: Option<String>,
}

impl ResolvedFork {
    pub(crate) fn new(
        url: &str,
        headers: Option<&[String]>,
        jwt: Option<&str>,
        selector: Option<BlockNumber>,
        block: BlockNumHash,
        context: ForkContext,
    ) -> Self {
        debug_assert_eq!(block.number, context.block_number);
        Self {
            source: ForkSource {
                url: url.to_string(),
                headers: headers.unwrap_or_default().to_vec(),
                jwt: jwt.map(str::to_string),
            },
            selector,
            block,
            context,
        }
    }

    pub(crate) fn matches(
        &self,
        url: &str,
        headers: Option<&[String]>,
        jwt: Option<&str>,
        selector: Option<BlockNumber>,
    ) -> bool {
        self.matches_source(url, headers, jwt) && self.selector == selector
    }

    /// Returns whether the configured RPC source still matches this resolved fork.
    pub(crate) fn matches_source(
        &self,
        url: &str,
        headers: Option<&[String]>,
        jwt: Option<&str>,
    ) -> bool {
        self.source.url == url
            && self.source.headers.as_slice() == headers.unwrap_or_default()
            && self.source.jwt.as_deref() == jwt
    }

    /// Returns the resolved block number.
    pub const fn number(&self) -> BlockNumber {
        self.block.number
    }

    /// Returns the resolved block hash.
    pub const fn hash(&self) -> B256 {
        self.block.hash
    }

    /// Returns the endpoint and network identity resolved with this block.
    pub const fn context(&self) -> ForkContext {
        self.context
    }

    /// Returns an EIP-1898 selector for the exact resolved block.
    ///
    /// The block is not required to remain canonical so callers can still query the resolved
    /// state after a reorganization.
    pub fn exact_block_id(&self) -> BlockId {
        BlockId::from((self.hash(), Some(false)))
    }

    /// Returns the resolved block number and hash.
    pub(crate) const fn block(&self) -> BlockNumHash {
        self.block
    }

    /// Returns this resolution advanced to another exact block on the same RPC source.
    pub(crate) fn at_block(&self, block: BlockNumHash) -> Self {
        let mut resolved = self.clone();
        resolved.selector = Some(block.number);
        resolved.block = block;
        resolved.context.block_number = block.number;
        resolved
    }

    /// Returns an opaque identity for the complete configured RPC source.
    pub(crate) fn source_id(&self) -> B256 {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(b"foundry-resolved-fork-source-v1");
        encode_source_part(&mut encoded, self.source.url.as_bytes());
        encoded.extend_from_slice(
            &u64::try_from(self.source.headers.len())
                .expect("fork header count exceeds u64")
                .to_be_bytes(),
        );
        for header in &self.source.headers {
            encode_source_part(&mut encoded, header.as_bytes());
        }
        if let Some(jwt) = &self.source.jwt {
            encoded.push(1);
            encode_source_part(&mut encoded, jwt.as_bytes());
        } else {
            encoded.push(0);
        }
        keccak256(encoded)
    }

    /// Returns a redacted, opaque fingerprint of the complete resolved fork identity.
    pub fn fingerprint(&self) -> B256 {
        let encoded = serde_json::to_vec(&(
            "foundry-resolved-fork-v1",
            self.source_id(),
            self.block,
            self.context,
        ))
        .expect("resolved fork identity is serializable");
        keccak256(encoded)
    }
}

fn encode_source_part(encoded: &mut Vec<u8>, part: &[u8]) {
    let len = u64::try_from(part.len()).expect("source identity part length exceeds u64");
    encoded.extend_from_slice(&len.to_be_bytes());
    encoded.extend_from_slice(part);
}

impl fmt::Debug for ResolvedFork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ResolvedFork");
        debug.field("source", &"<redacted>");
        if let Some(number) = self.selector {
            debug.field("selector", &number);
        } else {
            debug.field("selector", &"latest");
        }
        debug.field("number", &self.number()).field("hash", &self.hash()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
    use serde_json::json;
    use std::collections::HashSet;

    fn context(block_number: BlockNumber) -> ForkContext {
        ForkContext {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            block_number,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        }
    }

    #[test]
    fn exact_block_id_serializes_as_eip_1898_object() {
        let hash = B256::with_last_byte(1);
        let fork = ResolvedFork::new(
            "http://localhost:8545",
            None,
            None,
            None,
            BlockNumHash::new(1, hash),
            context(1),
        );

        assert_eq!(
            serde_json::to_value(fork.exact_block_id()).unwrap(),
            json!({
                "blockHash": hash,
                "requireCanonical": false,
            })
        );
    }

    #[test]
    fn endpoint_identity_participates_in_equality_and_hashing() {
        let block = BlockNumHash::new(1, B256::with_last_byte(1));
        let first = ResolvedFork::new("http://localhost:8545", None, None, None, block, context(1));
        let mut changed_context = context(1);
        changed_context.instance_id = Some(B256::with_last_byte(2));
        let second =
            ResolvedFork::new("http://localhost:8545", None, None, None, block, changed_context);

        assert_ne!(first, second);
        assert_eq!(HashSet::from([first, second]).len(), 2);
    }

    #[test]
    fn configured_source_identity_is_unambiguous() {
        let block = BlockNumHash::new(1, B256::with_last_byte(1));
        let context = context(1);
        let plain = ResolvedFork::new("http://localhost:8545", None, None, None, block, context);
        let header = ResolvedFork::new(
            "http://localhost:8545",
            Some(&["secret".to_string()]),
            None,
            None,
            block,
            context,
        );
        let jwt =
            ResolvedFork::new("http://localhost:8545", None, Some("secret"), None, block, context);

        assert_ne!(plain.source_id(), header.source_id());
        assert_ne!(plain.source_id(), jwt.source_id());
        assert_ne!(header.source_id(), jwt.source_id());
        assert_ne!(plain.fingerprint(), header.fingerprint());
        assert_ne!(plain.fingerprint(), jwt.fingerprint());
    }
}
