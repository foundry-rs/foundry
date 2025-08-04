//! A task that listens for new blocks

use crate::shutdown::Shutdown;
use futures::{FutureExt, Stream, StreamExt};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

/// A Future that will execute a given `task` for each new block that arrives on the stream.
pub struct BlockListener<St, F, Fut> {
    stream: St,
    task_factory: F,
    task: Option<Pin<Box<Fut>>>,
    on_shutdown: Shutdown,
}

impl<St, F, Fut> BlockListener<St, F, Fut>
where
    St: Stream,
    F: Fn(<St as Stream>::Item) -> Fut,
{
    pub fn new(on_shutdown: Shutdown, block_stream: St, task_factory: F) -> Self {
        Self { stream: block_stream, task_factory, task: None, on_shutdown }
    }
}

impl<St, F, Fut> Future for BlockListener<St, F, Fut>
where
    St: Stream + Unpin,
    F: Fn(<St as Stream>::Item) -> Fut + Unpin + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        if pin.on_shutdown.poll_unpin(cx).is_ready() {
            return Poll::Ready(());
        }

        let mut block = None;
        // drain the stream
        while let Poll::Ready(maybe_block) = pin.stream.poll_next_unpin(cx) {
            if maybe_block.is_none() {
                // stream complete
                return Poll::Ready(());
            }
            block = maybe_block;
        }

        if let Some(block) = block {
            pin.task = Some(Box::pin((pin.task_factory)(block)));
        }

        if let Some(mut task) = pin.task.take()
            && task.poll_unpin(cx).is_pending()
        {
            pin.task = Some(task);
        }
        Poll::Pending
    }
}
