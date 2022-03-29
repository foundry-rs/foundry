//! background service

use crate::eth::{miner::MiningMode, pool::Pool};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// The type that drives the blockchain's state
pub struct NodeService {
    pool: Arc<Pool>,
    mining_mod: MiningMode,
}

impl Future for NodeService {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
