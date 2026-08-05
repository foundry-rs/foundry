use crate::opts::ForkContext;
use alloy_eips::{BlockId, BlockNumHash};
use alloy_primitives::{B256, BlockNumber};
use std::fmt;

/// A fork selector and block identity resolved from a configured RPC source.
///
/// This context binds exact preflight reads and EVM environment reconstruction to the source,
/// selector, and block that were resolved together. The fork database itself remains
/// number-pinned.
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
}
