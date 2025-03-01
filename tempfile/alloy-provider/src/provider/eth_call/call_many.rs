use std::{marker::PhantomData, sync::Arc, task::Poll};

use alloy_eips::BlockId;
use alloy_json_rpc::RpcRecv;
use alloy_network::Network;
use alloy_rpc_types_eth::{state::StateOverride, Bundle, StateContext, TransactionIndex};
use alloy_transport::TransportResult;
use futures::{future, FutureExt};

use crate::ProviderCall;

use super::{Caller, EthCallManyParams};

/// A builder for an `"eth_callMany"` RPC request.
#[derive(Clone)]
pub struct EthCallMany<'req, N, Resp: RpcRecv, Output = Resp, Map = fn(Resp) -> Output>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    caller: Arc<dyn Caller<N, Resp>>,
    params: EthCallManyParams<'req>,
    map: Map,
    _pd: PhantomData<fn() -> (Resp, Output)>,
}

impl<N, Resp, Output, Map> std::fmt::Debug for EthCallMany<'_, N, Resp, Output, Map>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthCallMany")
            .field("params", &self.params)
            .field("method", &"eth_callMany")
            .finish()
    }
}

impl<'req, N, Resp> EthCallMany<'req, N, Resp>
where
    N: Network,
    Resp: RpcRecv,
{
    /// Instantiates a new `EthCallMany` with the given parameters.
    pub fn new(caller: impl Caller<N, Resp> + 'static, bundles: &'req Vec<Bundle>) -> Self {
        Self {
            caller: Arc::new(caller),
            params: EthCallManyParams::new(bundles),
            map: std::convert::identity,
            _pd: PhantomData,
        }
    }
}

impl<'req, N, Resp, Output, Map> EthCallMany<'req, N, Resp, Output, Map>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    /// Set a mapping function to transform the response.
    pub fn map<NewOutput, NewMap>(
        self,
        map: NewMap,
    ) -> EthCallMany<'req, N, Resp, NewOutput, NewMap>
    where
        NewMap: Fn(Resp) -> NewOutput,
    {
        EthCallMany { caller: self.caller, params: self.params, map, _pd: PhantomData }
    }

    /// Set the [`BlockId`] in the [`StateContext`].
    pub fn block(mut self, block: BlockId) -> Self {
        self.params = self.params.with_block(block);
        self
    }

    /// Set the [`TransactionIndex`] in the [`StateContext`].
    pub fn transaction_index(mut self, tx_index: TransactionIndex) -> Self {
        self.params = self.params.with_transaction_index(tx_index);
        self
    }

    /// Set the [`StateContext`] for the call.
    pub fn context(mut self, context: &'req StateContext) -> Self {
        self.params = self.params.with_context(*context);
        self
    }

    /// Set the [`StateOverride`] for the call.
    pub fn overrides(mut self, overrides: &'req StateOverride) -> Self {
        self.params = self.params.with_overrides(overrides);
        self
    }

    /// Extend the bundles for the call.
    pub fn extend_bundles(mut self, bundles: &'req [Bundle]) -> Self {
        self.params.bundles_mut().extend_from_slice(bundles);
        self
    }
}

impl<'req, N, Resp, Output, Map> std::future::IntoFuture for EthCallMany<'req, N, Resp, Output, Map>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    type Output = TransportResult<Output>;

    type IntoFuture = CallManyFut<'req, N, Resp, Output, Map>;

    fn into_future(self) -> Self::IntoFuture {
        CallManyFut {
            inner: CallManyInnerFut::Preparing {
                caller: self.caller,
                params: self.params,
                map: self.map,
            },
        }
    }
}

/// Intermediate future for `"eth_callMany"` requests.
#[derive(Debug)]
#[doc(hidden)] // Not public API.
#[allow(unnameable_types)]
#[pin_project::pin_project]
pub struct CallManyFut<'req, N: Network, Resp: RpcRecv, Output, Map: Fn(Resp) -> Output> {
    inner: CallManyInnerFut<'req, N, Resp, Output, Map>,
}

impl<N, Resp, Output, Map> CallManyFut<'_, N, Resp, Output, Map>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    const fn is_preparing(&self) -> bool {
        matches!(self.inner, CallManyInnerFut::Preparing { .. })
    }

    const fn is_running(&self) -> bool {
        matches!(self.inner, CallManyInnerFut::Running { .. })
    }

    fn poll_preparing(&mut self, cx: &mut std::task::Context<'_>) -> Poll<TransportResult<Output>> {
        let CallManyInnerFut::Preparing { caller, params, map } =
            std::mem::replace(&mut self.inner, CallManyInnerFut::Polling)
        else {
            unreachable!("bad state");
        };

        let fut = caller.call_many(params)?;
        self.inner = CallManyInnerFut::Running { fut, map };
        self.poll_running(cx)
    }

    fn poll_running(&mut self, cx: &mut std::task::Context<'_>) -> Poll<TransportResult<Output>> {
        let CallManyInnerFut::Running { ref mut fut, ref map } = self.inner else {
            unreachable!("bad state");
        };

        fut.poll_unpin(cx).map(|res| res.map(map))
    }
}

impl<N, Resp, Output, Map> future::Future for CallManyFut<'_, N, Resp, Output, Map>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    type Output = TransportResult<Output>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.is_preparing() {
            this.poll_preparing(cx)
        } else if this.is_running() {
            this.poll_running(cx)
        } else {
            panic!("bad state");
        }
    }
}

enum CallManyInnerFut<'req, N: Network, Resp: RpcRecv, Output, Map: Fn(Resp) -> Output> {
    Preparing { caller: Arc<dyn Caller<N, Resp>>, params: EthCallManyParams<'req>, map: Map },
    Running { fut: ProviderCall<EthCallManyParams<'static>, Resp>, map: Map },
    Polling,
}

impl<N, Resp, Output, Map> std::fmt::Debug for CallManyInnerFut<'_, N, Resp, Output, Map>
where
    N: Network,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallManyInnerFut::Preparing { params, .. } => {
                f.debug_tuple("Preparing").field(&params).finish()
            }
            CallManyInnerFut::Running { .. } => f.debug_tuple("Running").finish(),
            CallManyInnerFut::Polling => f.debug_tuple("Polling").finish(),
        }
    }
}
