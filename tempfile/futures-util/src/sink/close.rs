use core::marker::PhantomData;
use core::pin::Pin;
use futures_core::future::Future;
use futures_core::task::{Context, Poll};
use futures_sink::Sink;

/// Future for the [`close`](super::SinkExt::close) method.
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Close<'a, Si: ?Sized, Item> {
    sink: &'a mut Si,
    _phantom: PhantomData<fn(Item)>,
}

impl<Si: Unpin + ?Sized, Item> Unpin for Close<'_, Si, Item> {}

/// A future that completes when the sink has finished closing.
///
/// The sink itself is returned after closing is complete.
impl<'a, Si: Sink<Item> + Unpin + ?Sized, Item> Close<'a, Si, Item> {
    pub(super) fn new(sink: &'a mut Si) -> Self {
        Self { sink, _phantom: PhantomData }
    }
}

impl<Si: Sink<Item> + Unpin + ?Sized, Item> Future for Close<'_, Si, Item> {
    type Output = Result<(), Si::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.sink).poll_close(cx)
    }
}
