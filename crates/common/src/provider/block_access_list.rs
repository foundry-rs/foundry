//! Block access list retrieval and provider capability probing.
//!
//! Retrieval also probes the endpoint, including for blocks before Amsterdam. These helpers do not
//! apply state changes or establish that a returned list can safely reconstruct transaction state.

use super::{is_rpc_method_not_found, rpc_error_code};
use alloy_eips::{BlockId, eip7928::BlockAccessList};
use alloy_network::Network;
use alloy_provider::Provider;
use alloy_rpc_types::error::EthRpcErrorCode;
use alloy_transport::TransportError;

/// The outcome of requesting a block access list.
#[derive(Debug, PartialEq, Eq)]
pub enum BlockAccessListOutcome {
    /// The provider returned a typed block access list, which may be empty.
    ///
    /// This only checks the response schema. Callers must validate the list against the target
    /// block and transaction before using it for state reconstruction.
    Available(BlockAccessList),
    /// The response was `null` or reported resource-not-found (`-32001`) for this block.
    ///
    /// This does not imply that other blocks are unavailable or that the method is unsupported.
    Unavailable,
    /// The endpoint reports that `eth_getBlockAccessList` is not supported.
    Unsupported,
}

/// A failure to retrieve a block access list, without evidence that the method is unsupported.
#[derive(Debug, thiserror::Error)]
pub enum BlockAccessListError {
    /// The response could not be decoded as a block access list.
    #[error("invalid block access list response: {0}")]
    InvalidResponse(#[source] TransportError),
    /// The request failed, for example due to rate limiting, authentication, or a transport error.
    #[error("block access list request failed: {0}")]
    Request(#[source] TransportError),
}

/// Fetches a block access list using `eth_getBlockAccessList`.
///
/// The request itself probes provider support, without checking hardfork activation. Providers may
/// serve lists for historical blocks before Amsterdam. This function keeps no capability cache:
/// an unavailable block, invalid response, or request failure does not suppress subsequent probes.
/// It adds no retries beyond those configured on the supplied provider.
///
/// For transaction-specific forks, pass the mined transaction's block hash to pin the request to
/// that block across reorganizations. Resolving the transaction and validating/applying the list
/// are the caller's responsibility. In particular, [`BlockAccessListOutcome::Available`] alone does
/// not establish that the list can safely replace replay.
pub async fn fetch_block_access_list<P, N>(
    provider: &P,
    block: BlockId,
) -> Result<BlockAccessListOutcome, BlockAccessListError>
where
    P: Provider<N> + ?Sized,
    N: Network,
{
    // Alloy's get_block_access_list dispatches to separate ByBlockHash/ByBlockNumber endpoints.
    let result = provider
        .client()
        .request::<_, Option<BlockAccessList>>("eth_getBlockAccessList", (block,))
        .await;
    match result {
        Ok(Some(access_list)) => Ok(BlockAccessListOutcome::Available(access_list)),
        Ok(None) => Ok(BlockAccessListOutcome::Unavailable),
        Err(error) if is_rpc_method_not_found(&error) => Ok(BlockAccessListOutcome::Unsupported),
        Err(error)
            if rpc_error_code(&error) == Some(EthRpcErrorCode::ResourceNotFound.code().into()) =>
        {
            Ok(BlockAccessListOutcome::Unavailable)
        }
        Err(error) if error.is_deser_error() => Err(BlockAccessListError::InvalidResponse(error)),
        Err(error) => Err(BlockAccessListError::Request(error)),
    }
}
