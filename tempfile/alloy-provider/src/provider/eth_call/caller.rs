use super::{EthCallManyParams, EthCallParams};
use crate::ProviderCall;
use alloy_json_rpc::RpcRecv;
use alloy_network::Network;
use alloy_rpc_client::WeakClient;
use alloy_transport::{TransportErrorKind, TransportResult};

/// Trait that helpes convert `EthCall` into a `ProviderCall`.
pub trait Caller<N, Resp>: Send + Sync
where
    N: Network,
    Resp: RpcRecv,
{
    /// Method that needs to be implemented to convert to a `ProviderCall`.
    ///
    /// This method sends the request to relevant data source and returns a `ProviderCall`.
    fn call(
        &self,
        params: EthCallParams<'_, N>,
    ) -> TransportResult<ProviderCall<EthCallParams<'static, N>, Resp>>;

    /// Method that needs to be implemented for estimating gas using "eth_estimateGas" for the
    /// transaction.
    fn estimate_gas(
        &self,
        params: EthCallParams<'_, N>,
    ) -> TransportResult<ProviderCall<EthCallParams<'static, N>, Resp>>;

    /// Method that needs to be implemented for `"eth_callMany"` RPC requests.
    fn call_many(
        &self,
        params: EthCallManyParams<'_>,
    ) -> TransportResult<ProviderCall<EthCallManyParams<'static>, Resp>>;
}

impl<N, Resp> Caller<N, Resp> for WeakClient
where
    N: Network,
    Resp: RpcRecv,
{
    fn call(
        &self,
        params: EthCallParams<'_, N>,
    ) -> TransportResult<ProviderCall<EthCallParams<'static, N>, Resp>> {
        provider_rpc_call(self, "eth_call", params)
    }

    fn estimate_gas(
        &self,
        params: EthCallParams<'_, N>,
    ) -> TransportResult<ProviderCall<EthCallParams<'static, N>, Resp>> {
        provider_rpc_call(self, "eth_estimateGas", params)
    }

    fn call_many(
        &self,
        params: EthCallManyParams<'_>,
    ) -> TransportResult<ProviderCall<EthCallManyParams<'static>, Resp>> {
        let client = self.upgrade().ok_or_else(TransportErrorKind::backend_gone)?;

        let rpc_call = client.request("eth_callMany", params.into_owned());

        Ok(ProviderCall::RpcCall(rpc_call))
    }
}

fn provider_rpc_call<N: Network, Resp: RpcRecv>(
    client: &WeakClient,
    method: &'static str,
    params: EthCallParams<'_, N>,
) -> TransportResult<ProviderCall<EthCallParams<'static, N>, Resp>> {
    let client = client.upgrade().ok_or_else(TransportErrorKind::backend_gone)?;

    let rpc_call = client.request(method, params.into_owned());

    Ok(ProviderCall::RpcCall(rpc_call))
}
