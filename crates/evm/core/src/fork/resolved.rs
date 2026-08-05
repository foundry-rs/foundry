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
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ForkSource {
    url: String,
    headers: Vec<String>,
}

impl ResolvedFork {
    pub(crate) fn new(
        url: &str,
        headers: Option<&[String]>,
        selector: Option<BlockNumber>,
        block: BlockNumHash,
    ) -> Self {
        Self {
            source: ForkSource {
                url: url.to_string(),
                headers: headers.unwrap_or_default().to_vec(),
            },
            selector,
            block,
        }
    }

    pub(crate) fn matches(
        &self,
        url: &str,
        headers: Option<&[String]>,
        selector: Option<BlockNumber>,
    ) -> bool {
        self.source.url == url
            && self.source.headers.as_slice() == headers.unwrap_or_default()
            && self.selector == selector
    }

    /// Returns the resolved block number.
    pub const fn number(&self) -> BlockNumber {
        self.block.number
    }

    /// Returns the resolved block hash.
    pub const fn hash(&self) -> B256 {
        self.block.hash
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
    use serde_json::json;

    #[test]
    fn exact_block_id_serializes_as_eip_1898_object() {
        let hash = B256::with_last_byte(1);
        let fork =
            ResolvedFork::new("http://localhost:8545", None, None, BlockNumHash::new(1, hash));

        assert_eq!(
            serde_json::to_value(fork.exact_block_id()).unwrap(),
            json!({
                "blockHash": hash,
                "requireCanonical": false,
            })
        );
    }
}
