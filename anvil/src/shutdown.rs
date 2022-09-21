//! Helper for shutdown signals

use futures::{
    channel::oneshot,
    future::{FusedFuture, Shared},
    FutureExt,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Future that resolves when the shutdown event has fired
#[derive(Clone)]
pub struct Shutdown(Shared<oneshot::Receiver<()>>);

impl Future for Shutdown {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        if pin.0.is_terminated() || pin.0.poll_unpin(cx).is_ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// Shutdown signal that fires either manually or on drop by closing the channel
pub struct Signal(oneshot::Sender<()>);

impl Signal {
    /// Fire the signal manually.
    pub fn fire(self) -> Result<(), ()> {
        self.0.send(())
    }
}

/// Create a channel pair that's used to propagate shutdown event
pub fn signal() -> (Signal, Shutdown) {
    let (sender, receiver) = oneshot::channel();
    (Signal(sender), Shutdown(receiver.shared()))
}
