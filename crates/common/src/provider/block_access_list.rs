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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::ErrorPayload;
    use alloy_provider::ProviderBuilder;
    use alloy_transport::mock::Asserter;
    use serde_json::json;

    #[tokio::test]
    async fn retrieves_historical_block_access_list() {
        let response = json!([{
            "address": "0x0000000000000000000000000000000000000001",
            "storageChanges": [{"key": "0x1", "changes": [{"index": "0x1", "value": "0x2"}]}],
            "storageReads": ["0x3"],
            "balanceChanges": [{"index": "0x1", "value": "0x4"}],
            "nonceChanges": [{"index": "0x1", "value": "0x5"}],
            "codeChanges": [{"index": "0x1", "code": "0x6000"}]
        }]);
        let expected = serde_json::from_value::<BlockAccessList>(response.clone()).unwrap();
        let asserter = Asserter::new();
        asserter.push_success(&response);
        let provider = ProviderBuilder::new().connect_mocked_client(asserter);

        // Historical mainnet blocks must be probed even though Amsterdam was not active.
        let outcome =
            fetch_block_access_list(&provider, BlockId::number(20_000_000)).await.unwrap();
        assert_eq!(outcome, BlockAccessListOutcome::Available(expected));
    }

    #[tokio::test]
    async fn missing_block_access_list_does_not_disable_later_requests() {
        let asserter = Asserter::new();
        asserter.push_success(&serde_json::Value::Null);
        asserter.push_success(&json!([]));
        let provider = ProviderBuilder::new().connect_mocked_client(asserter);

        assert_eq!(
            fetch_block_access_list(&provider, BlockId::number(20_000_000)).await.unwrap(),
            BlockAccessListOutcome::Unavailable
        );
        assert_eq!(
            fetch_block_access_list(&provider, BlockId::number(20_000_001)).await.unwrap(),
            BlockAccessListOutcome::Available(vec![])
        );
    }

    #[tokio::test]
    async fn distinguishes_rpc_errors_without_disabling_later_requests() {
        for code in [-32601, -32001, -32602, -32603, -32000, -32005] {
            let asserter = Asserter::new();
            // Classification must use the code, not a provider-specific message.
            asserter.push_failure(ErrorPayload {
                code,
                message: "block access list unavailable".into(),
                data: None,
            });
            asserter.push_success(&json!([]));
            let provider = ProviderBuilder::new().connect_mocked_client(asserter);

            let result = fetch_block_access_list(&provider, BlockId::number(20_000_000)).await;
            match code {
                -32601 => assert_eq!(result.unwrap(), BlockAccessListOutcome::Unsupported),
                -32001 => assert_eq!(result.unwrap(), BlockAccessListOutcome::Unavailable),
                _ => {
                    let BlockAccessListError::Request(error) = result.unwrap_err() else {
                        panic!("expected a request error for code {code}");
                    };
                    assert_eq!(error.as_error_resp().unwrap().code, code);
                }
            }
            assert_eq!(
                fetch_block_access_list(&provider, BlockId::number(20_000_001)).await.unwrap(),
                BlockAccessListOutcome::Available(vec![])
            );
        }
    }

    #[tokio::test]
    async fn rejects_malformed_responses_without_disabling_later_requests() {
        let account = json!({
            "address": "0x0000000000000000000000000000000000000001",
            "storageChanges": [],
            "storageReads": [],
            "balanceChanges": [],
            "nonceChanges": [],
            "codeChanges": []
        });
        let mut invalid_address = account.clone();
        invalid_address["address"] = json!("0x01");
        let mut invalid_index = account.clone();
        invalid_index["balanceChanges"] = json!([{"index": "0x10000000000000000", "value": "0x1"}]);
        let mut invalid_balance = account.clone();
        invalid_balance["balanceChanges"] = json!([{"index": "0x1", "value": "0xinvalid"}]);
        let mut invalid_storage = account.clone();
        invalid_storage["storageChanges"] = json!([{"key": "0x1", "changes": [{}]}]);
        let mut invalid_code = account;
        invalid_code["codeChanges"] = json!([{"index": "0x1", "code": "0xgg"}]);

        for response in [
            json!({"blockAccessList": []}),
            json!("0x"),
            json!([{"address": "0x0000000000000000000000000000000000000001"}]),
            json!([invalid_address]),
            json!([invalid_index]),
            json!([invalid_balance]),
            json!([invalid_storage]),
            json!([invalid_code]),
        ] {
            let asserter = Asserter::new();
            asserter.push_success(&response);
            asserter.push_success(&json!([]));
            let provider = ProviderBuilder::new().connect_mocked_client(asserter);

            let error =
                fetch_block_access_list(&provider, BlockId::number(20_000_000)).await.unwrap_err();
            assert!(matches!(error, BlockAccessListError::InvalidResponse(_)), "{error:?}");
            assert_eq!(
                fetch_block_access_list(&provider, BlockId::number(20_000_001)).await.unwrap(),
                BlockAccessListOutcome::Available(vec![])
            );
        }
    }
}
