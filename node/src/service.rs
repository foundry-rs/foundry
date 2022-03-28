//! background service

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Gets spawned, and drives the chain's state
pub struct NodeHandler {}

impl Future for NodeHandler {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}
