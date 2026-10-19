use super::opts::{EvmOpts, ForkContext};
use alloy_consensus::BlockHeader;
use alloy_eips::{BlockId, BlockNumHash};
use alloy_network::{AnyNetwork, AnyRpcBlock, Network};
use alloy_primitives::{B256, BlockNumber, keccak256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_client::RpcClient;
use eyre::OptionExt;
use std::{
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub mod database;

mod multi;
pub use multi::{ForkId, ForkResult, MultiFork, MultiForkHandler};

mod bal;
pub use bal::{cache_bal, validate_bal};

/// Represents a _fork_ of a remote chain whose data is available only via the `url` endpoint.
#[derive(Clone, Debug)]
pub struct CreateFork {
    /// Whether to enable rpc storage caching for this fork
    pub enable_caching: bool,
    /// The URL to a node for fetching remote state
    pub url: String,
    /// All env settings as configured by the user
    pub evm_opts: EvmOpts,
}

/// A prepared remote fork. The RPC client and block are shared by preflight and execution.
///
/// The configured selector remains separate from the observed block. Clones share the provider
/// and header; changing the requested source or rolling a fork prepares a new snapshot. The
/// optional number-based state mode retains this anchor, but state reads may follow a replacement
/// block after a reorganization.
#[derive(Clone)]
pub struct Fork {
    source_id: B256,
    client: RpcClient,
    selector: Option<BlockNumber>,
    pub(crate) block: Arc<AnyRpcBlock>,
    context: ForkContext,
    /// Whether state reads use the RPC block number instead of the exact hash.
    pub(crate) state_by_number: bool,
}

impl Fork {
    pub(crate) fn new(
        opts: &EvmOpts,
        client: RpcClient,
        block: AnyRpcBlock,
        mut context: ForkContext,
    ) -> Self {
        debug_assert_eq!(block.header.number(), context.block_number);
        context.network_profile = context.network_profile.canonical_execution_profile();
        Self {
            source_id: source_id(
                opts.fork_url.as_deref().unwrap_or_default(),
                opts.fork_source_headers(),
                opts.rpc_jwt.as_deref(),
            ),
            client,
            selector: opts.fork_block_number,
            block: Arc::new(block),
            context,
            state_by_number: opts.fork_state_by_number,
        }
    }

    /// Whether this backend's fork was created from the requested source, selector, and state mode.
    pub fn matches_request(&self, opts: &EvmOpts) -> bool {
        opts.fork_url.as_deref().is_some_and(|url| {
            self.selector == opts.fork_block_number
                && self.state_by_number == opts.fork_state_by_number
                && self.matches_source(url, opts.fork_source_headers(), opts.rpc_jwt.as_deref())
        })
    }

    pub(crate) fn matches_source(
        &self,
        url: &str,
        headers: Option<&[String]>,
        jwt: Option<&str>,
    ) -> bool {
        self.source_id == source_id(url, headers, jwt)
    }

    /// Returns the RPC block number, which may differ from the EVM-visible number on L2s.
    pub fn number(&self) -> BlockNumber {
        self.block.header.number()
    }

    /// Returns the fork block hash.
    pub fn hash(&self) -> B256 {
        self.block.header.hash
    }

    /// Returns the endpoint and execution context observed when the fork was prepared.
    pub const fn context(&self) -> ForkContext {
        self.context
    }

    /// Returns an exact state selector that remains valid for a retained noncanonical block.
    pub fn exact_block_id(&self) -> BlockId {
        BlockId::from((self.hash(), Some(false)))
    }

    /// Returns the state selector, honoring the RPC compatibility opt-in.
    pub(crate) fn state_block_id(&self) -> BlockId {
        if self.state_by_number { BlockId::number(self.number()) } else { self.exact_block_id() }
    }

    pub(crate) fn block(&self) -> BlockNumHash {
        BlockNumHash::new(self.number(), self.hash())
    }

    /// Reuses the configured RPC client with the response types required by the caller.
    pub(crate) fn provider<N: Network>(&self) -> RootProvider<N> {
        RootProvider::new(self.client.clone())
    }

    pub(crate) async fn at_block(&self, block: BlockNumHash) -> eyre::Result<Self> {
        let provider = self.provider::<AnyNetwork>();
        let response = provider
            .get_block_by_hash(block.hash)
            .await?
            .ok_or_eyre("exact fork block is unavailable")?;
        eyre::ensure!(
            response.header.number() == block.number && response.header.hash == block.hash,
            "exact fork block does not match the requested number and hash"
        );
        let mut fork = self.clone();
        fork.selector = Some(block.number);
        fork.block = Arc::new(response);
        fork.state_by_number = false;
        fork.context.block_number = block.number;
        Ok(fork)
    }

    pub(crate) const fn source_id(&self) -> B256 {
        self.source_id
    }

    /// Returns the stable, redacted identity used by persisted execution caches.
    pub fn fingerprint(&self) -> B256 {
        let mut encoded = serde_json::to_vec(&(
            "foundry-resolved-fork-v1",
            self.source_id,
            self.block(),
            self.context,
        ))
        .expect("fork identity is serializable");
        // Distinguish execution policies without invalidating existing hash-based disk caches.
        if self.state_by_number {
            encoded.extend_from_slice(b"state-by-number");
        }
        keccak256(encoded)
    }
}

impl PartialEq for Fork {
    fn eq(&self, other: &Self) -> bool {
        (self.source_id, self.selector, self.block(), self.context, self.state_by_number)
            == (
                other.source_id,
                other.selector,
                other.block(),
                other.context,
                other.state_by_number,
            )
    }
}

impl Eq for Fork {}

impl Hash for Fork {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.source_id, self.selector, self.block(), self.context, self.state_by_number)
            .hash(state);
    }
}

impl fmt::Debug for Fork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Fork");
        debug.field("source", &"<redacted>");
        if let Some(number) = self.selector {
            debug.field("selector", &number);
        } else {
            debug.field("selector", &"latest");
        }
        debug.field("block", &self.block()).finish()
    }
}

fn source_id(url: &str, headers: Option<&[String]>, jwt: Option<&str>) -> B256 {
    let mut encoded = Vec::from(b"foundry-resolved-fork-source-v1".as_slice());
    encode_source_part(&mut encoded, url.as_bytes());
    let headers = headers.unwrap_or_default();
    encoded.extend_from_slice(
        &u64::try_from(headers.len()).expect("fork header count exceeds u64").to_be_bytes(),
    );
    for header in headers {
        encode_source_part(&mut encoded, header.as_bytes());
    }
    if let Some(jwt) = jwt {
        encoded.push(1);
        encode_source_part(&mut encoded, jwt.as_bytes());
    } else {
        encoded.push(0);
    }
    keccak256(encoded)
}

fn encode_source_part(encoded: &mut Vec<u8>, part: &[u8]) {
    encoded.extend_from_slice(
        &u64::try_from(part.len()).expect("source identity part length exceeds u64").to_be_bytes(),
    );
    encoded.extend_from_slice(part);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::{AnyHeader, AnyRpcHeader};
    use alloy_rpc_types::Block;
    use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
    use serde_json::json;
    use std::collections::HashSet;

    impl Fork {
        pub(crate) fn test(
            url: &str,
            headers: Option<&[String]>,
            jwt: Option<&str>,
            selector: Option<BlockNumber>,
            block: BlockNumHash,
            context: ForkContext,
        ) -> Self {
            let opts = EvmOpts {
                fork_url: Some(url.to_string()),
                fork_headers: headers.map(<[String]>::to_vec),
                rpc_jwt: jwt.map(str::to_string),
                fork_block_number: selector,
                ..Default::default()
            };
            let header = AnyHeader { number: block.number, ..Default::default() };
            let block = AnyRpcBlock::new(
                Block::new(AnyRpcHeader::from_sealed(header.seal(block.hash)), Default::default())
                    .into(),
            );
            Self::new(&opts, opts.fork_rpc_client(url).unwrap(), block, context)
        }
    }

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
        let fork = Fork::test(
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
    fn fork_state_by_number_preserves_anchor_and_separates_identity() {
        let block = BlockNumHash::new(42, B256::with_last_byte(1));
        let exact = Fork::test("http://localhost:8545", None, None, None, block, context(42));
        let mut numbered = exact.clone();
        numbered.state_by_number = true;
        assert_eq!(serde_json::to_value(numbered.state_block_id()).unwrap(), json!("0x2a"));
        assert_eq!(exact.state_block_id(), exact.exact_block_id());
        assert_eq!(numbered.exact_block_id(), exact.exact_block_id());
        assert_ne!(numbered, exact);
        assert_eq!(numbered.source_id(), exact.source_id());
        assert_ne!(numbered.fingerprint(), exact.fingerprint());
    }

    #[test]
    fn endpoint_identity_participates_in_equality_and_hashing() {
        let block = BlockNumHash::new(1, B256::with_last_byte(1));
        let first = Fork::test("http://localhost:8545", None, None, None, block, context(1));
        for changed_context in [
            ForkContext { instance_id: Some(B256::with_last_byte(2)), ..context(1) },
            ForkContext { network_profile: NetworkConfigs::with_celo(), ..context(1) },
            ForkContext {
                network: NetworkVariant::Tempo,
                network_profile: NetworkConfigs::with_tempo(),
                ..context(1)
            },
        ] {
            let second =
                Fork::test("http://localhost:8545", None, None, None, block, changed_context);

            assert_ne!(first, second);
            assert_ne!(first.fingerprint(), second.fingerprint());
            assert_eq!(HashSet::from([first.clone(), second]).len(), 2);
        }
    }

    #[test]
    fn configured_source_identity_is_unambiguous() {
        let block = BlockNumHash::new(1, B256::with_last_byte(1));
        let context = context(1);
        let plain = Fork::test("http://localhost:8545", None, None, None, block, context);
        let header = Fork::test(
            "http://localhost:8545",
            Some(&["secret".to_string()]),
            None,
            None,
            block,
            context,
        );
        let jwt = Fork::test("http://localhost:8545", None, Some("secret"), None, block, context);

        assert_ne!(plain.source_id, header.source_id);
        assert_ne!(plain.source_id, jwt.source_id);
        assert_ne!(header.source_id, jwt.source_id);
        assert_ne!(plain.fingerprint(), header.fingerprint());
        assert_ne!(plain.fingerprint(), jwt.fingerprint());
    }

    #[test]
    fn resolved_fork_canonicalizes_equivalent_ethereum_profiles() {
        let url = "http://localhost:8545";
        let block = BlockNumHash::new(1, B256::with_last_byte(1));
        let implicit = Fork::test(url, None, None, Some(1), block, context(1));
        let explicit = Fork::test(
            url,
            None,
            None,
            Some(1),
            block,
            ForkContext { network_profile: NetworkConfigs::with_ethereum(), ..context(1) },
        );

        assert_eq!(implicit.context(), explicit.context());
        assert_eq!(implicit.fingerprint(), explicit.fingerprint());
        assert_eq!(HashSet::from([implicit, explicit]).len(), 1);
    }
}
