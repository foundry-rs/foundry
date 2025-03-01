use alloy_eips::BlockId;
use alloy_json_rpc::{RpcRecv, RpcSend};
use alloy_primitives::B256;
use alloy_rpc_client::RpcCall;
use alloy_transport::TransportResult;
use std::future::IntoFuture;

use crate::ProviderCall;

/// Helper struct that houses the params along with the BlockId.
#[derive(Debug, Clone)]
pub struct ParamsWithBlock<Params: RpcSend> {
    /// The params to be sent to the RPC call.
    pub params: Params,
    /// The block id to be used for the RPC call.
    pub block_id: BlockId,
}

impl<Params: RpcSend> serde::Serialize for ParamsWithBlock<Params> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize params to a Value first
        let mut ser = serde_json::to_value(&self.params).map_err(serde::ser::Error::custom)?;

        // serialize the block id
        let block_id = serde_json::to_value(self.block_id).map_err(serde::ser::Error::custom)?;

        if let serde_json::Value::Array(ref mut arr) = ser {
            arr.push(block_id);
        } else if ser.is_null() {
            ser = serde_json::Value::Array(vec![block_id]);
        } else {
            ser = serde_json::Value::Array(vec![ser, block_id]);
        }

        ser.serialize(serializer)
    }
}

type ProviderCallProducer<Params, Resp, Output, Map> =
    Box<dyn Fn(BlockId) -> ProviderCall<ParamsWithBlock<Params>, Resp, Output, Map> + Send>;

/// Container for varous types of calls dependent on a block id.
enum WithBlockInner<Params, Resp, Output = Resp, Map = fn(Resp) -> Output>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    /// [RpcCall] which params are getting wrapped into [ParamsWithBlock] once the block id is set.
    RpcCall(RpcCall<Params, Resp, Output, Map>),
    /// Closure that produces a [ProviderCall] once the block id is set.
    ProviderCall(ProviderCallProducer<Params, Resp, Output, Map>),
}

impl<Params, Resp, Output, Map> core::fmt::Debug for WithBlockInner<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RpcCall(call) => f.debug_tuple("RpcCall").field(call).finish(),
            Self::ProviderCall(_) => f.debug_struct("ProviderCall").finish(),
        }
    }
}

/// A struct that takes an optional [`BlockId`] parameter.
///
/// This resolves to a [`ProviderCall`] that will execute the call on the specified block.
///
/// By default this will use "latest".
#[pin_project::pin_project]
#[derive(Debug)]
pub struct RpcWithBlock<Params, Resp, Output = Resp, Map = fn(Resp) -> Output>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output + Clone,
{
    inner: WithBlockInner<Params, Resp, Output, Map>,
    block_id: BlockId,
}

impl<Params, Resp, Output, Map> RpcWithBlock<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output + Clone,
{
    /// Create a new [`RpcWithBlock`] from a [`RpcCall`].
    pub fn new_rpc(inner: RpcCall<Params, Resp, Output, Map>) -> Self {
        Self { inner: WithBlockInner::RpcCall(inner), block_id: Default::default() }
    }

    /// Create a new [`RpcWithBlock`] from a closure producing a [`ProviderCall`].
    pub fn new_provider<F>(get_call: F) -> Self
    where
        F: Fn(BlockId) -> ProviderCall<ParamsWithBlock<Params>, Resp, Output, Map> + Send + 'static,
    {
        let get_call = Box::new(get_call);
        Self { inner: WithBlockInner::ProviderCall(get_call), block_id: Default::default() }
    }
}

impl<Params, Resp, Output, Map> From<RpcCall<Params, Resp, Output, Map>>
    for RpcWithBlock<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output + Clone,
{
    fn from(inner: RpcCall<Params, Resp, Output, Map>) -> Self {
        Self::new_rpc(inner)
    }
}

impl<F, Params, Resp, Output, Map> From<F> for RpcWithBlock<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output + Clone,
    F: Fn(BlockId) -> ProviderCall<ParamsWithBlock<Params>, Resp, Output, Map> + Send + 'static,
{
    fn from(inner: F) -> Self {
        Self::new_provider(inner)
    }
}

impl<Params, Resp, Output, Map> RpcWithBlock<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Map: Fn(Resp) -> Output + Clone,
{
    /// Set the block id.
    pub const fn block_id(mut self, block_id: BlockId) -> Self {
        self.block_id = block_id;
        self
    }

    /// Set the block id to "pending".
    pub const fn pending(self) -> Self {
        self.block_id(BlockId::pending())
    }

    /// Set the block id to "latest".
    pub const fn latest(self) -> Self {
        self.block_id(BlockId::latest())
    }

    /// Set the block id to "earliest".
    pub const fn earliest(self) -> Self {
        self.block_id(BlockId::earliest())
    }

    /// Set the block id to "finalized".
    pub const fn finalized(self) -> Self {
        self.block_id(BlockId::finalized())
    }

    /// Set the block id to "safe".
    pub const fn safe(self) -> Self {
        self.block_id(BlockId::safe())
    }

    /// Set the block id to a specific height.
    pub const fn number(self, number: u64) -> Self {
        self.block_id(BlockId::number(number))
    }

    /// Set the block id to a specific hash, without requiring the hash be part
    /// of the canonical chain.
    pub const fn hash(self, hash: B256) -> Self {
        self.block_id(BlockId::hash(hash))
    }

    /// Set the block id to a specific hash and require the hash be part of the
    /// canonical chain.
    pub const fn hash_canonical(self, hash: B256) -> Self {
        self.block_id(BlockId::hash_canonical(hash))
    }
}

impl<Params, Resp, Output, Map> IntoFuture for RpcWithBlock<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Output: 'static,
    Map: Fn(Resp) -> Output + Clone,
{
    type Output = TransportResult<Output>;

    type IntoFuture = ProviderCall<ParamsWithBlock<Params>, Resp, Output, Map>;

    fn into_future(self) -> Self::IntoFuture {
        match self.inner {
            WithBlockInner::RpcCall(rpc_call) => {
                let block_id = self.block_id;
                let rpc_call = rpc_call.map_params(|params| ParamsWithBlock { params, block_id });
                ProviderCall::RpcCall(rpc_call)
            }
            WithBlockInner::ProviderCall(get_call) => get_call(self.block_id),
        }
    }
}
