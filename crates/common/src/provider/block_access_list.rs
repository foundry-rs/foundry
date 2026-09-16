//! Block access list retrieval and provider capability probing.
//!
//! Retrieval also probes the endpoint, including for blocks before Amsterdam. These helpers do not
//! apply state changes or establish that a returned list can safely reconstruct transaction state.

use alloy_eips::{BlockId, eip7928::BlockAccessList};
use alloy_network::Network;
use alloy_provider::Provider;
use std::time::Duration;
use tokio::time::timeout;

/// Fetches a block access list using Alloy's [`Provider::get_block_access_list`].
///
/// Returns `None` if the list is unavailable, the method is unsupported, the response is invalid,
/// or the request fails or exceeds 500 milliseconds (including any provider retries).
///
/// The request itself probes provider support, without checking hardfork activation. Providers may
/// serve lists for historical blocks before Amsterdam. This function keeps no capability cache:
/// an unavailable block, invalid response, or request failure does not suppress subsequent probes.
/// It adds no retries beyond those configured on the supplied provider.
///
/// For transaction-specific forks, pass the mined transaction's block hash to pin the request to
/// that block across reorganizations. Resolving the transaction and validating/applying the list
/// are the caller's responsibility. A returned list does not establish that it can safely replace
/// replay. Hash selectors requiring canonicality return `None` because Alloy's hash endpoint does
/// not enforce `requireCanonical`.
pub async fn fetch_block_access_list<P, N>(provider: &P, block: BlockId) -> Option<BlockAccessList>
where
    P: Provider<N> + ?Sized,
    N: Network,
{
    if let BlockId::Hash(hash) = block
        && hash.require_canonical == Some(true)
    {
        return None;
    }

    timeout(Duration::from_millis(500), provider.get_block_access_list(block)).await.ok()?.ok()?
}
