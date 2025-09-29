use futures::stream;
use polkadot_sdk::sc_service::RpcHandlers;
use serde_json::Value;
use subxt::{
    backend::rpc::{RawRpcFuture, RawRpcSubscription, RawValue, RpcClientT},
    ext::{jsonrpsee::core::traits::ToRpcParams, subxt_rpcs::Error as SubxtRpcError},
};

pub struct InMemoryRpcClient(pub RpcHandlers);

pub struct Params(Option<Box<RawValue>>);

impl ToRpcParams for Params {
    fn to_rpc_params(self) -> std::result::Result<Option<Box<RawValue>>, serde_json::Error> {
        Ok(self.0)
    }
}

impl RpcClientT for InMemoryRpcClient {
    fn request_raw<'a>(
        &'a self,
        method: &'a str,
        params: Option<Box<RawValue>>,
    ) -> RawRpcFuture<'a, Box<RawValue>> {
        Box::pin(async move {
            self.0
                .handle()
                .call(method, Params(params))
                .await
                .map_err(|err| SubxtRpcError::Client(Box::new(err)))
        })
    }

    fn subscribe_raw<'a>(
        &'a self,
        sub: &'a str,
        params: Option<Box<RawValue>>,
        _unsub: &'a str,
    ) -> RawRpcFuture<'a, RawRpcSubscription> {
        Box::pin(async move {
            let subscription = self
                .0
                .handle()
                .subscribe_unbounded(sub, Params(params))
                .await
                .map_err(|err| SubxtRpcError::Client(Box::new(err)))?;
            let id = Value::from(subscription.subscription_id().to_owned())
                .as_str()
                .map(|s| s.to_string());
            let raw_stream = stream::unfold(subscription, |mut sub| async move {
                match sub.next::<Box<RawValue>>().await {
                    Some(Ok((notification, _sub_id))) => Some((Ok(notification), sub)),
                    Some(Err(e)) => Some((Err(SubxtRpcError::Client(Box::new(e))), sub)),
                    None => None, // Subscription ended, Do something here? :-??
                }
            });
            Ok(RawRpcSubscription { stream: Box::pin(raw_stream), id })
        })
    }
}
