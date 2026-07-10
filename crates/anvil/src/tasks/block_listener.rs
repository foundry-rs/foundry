//! A task that listens for new blocks

use crate::shutdown::Shutdown;
use futures::{FutureExt, Stream, StreamExt};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

/// A future that executes a given `task` for the latest block available on the stream.
///
/// Available blocks are coalesced and the latest block is processed after the active task
/// completes.
pub struct BlockListener<St, F, Fut> {
    stream: St,
    task_factory: F,
    task: Option<Pin<Box<Fut>>>,
    stream_done: bool,
    on_shutdown: Shutdown,
}

impl<St, F, Fut> BlockListener<St, F, Fut>
where
    St: Stream,
    F: Fn(<St as Stream>::Item) -> Fut,
{
    pub const fn new(on_shutdown: Shutdown, block_stream: St, task_factory: F) -> Self {
        Self { stream: block_stream, task_factory, task: None, stream_done: false, on_shutdown }
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

        if let Some(mut task) = pin.task.take()
            && task.poll_unpin(cx).is_pending()
        {
            pin.task = Some(task);
            return Poll::Pending;
        }

        if pin.stream_done {
            return Poll::Ready(());
        }

        let mut block = None;
        // drain the stream
        loop {
            match pin.stream.poll_next_unpin(cx) {
                Poll::Ready(Some(next_block)) => block = Some(next_block),
                Poll::Ready(None) => {
                    pin.stream_done = true;
                    break;
                }
                Poll::Pending => break,
            }
        }

        if let Some(block) = block {
            pin.task = Some(Box::pin((pin.task_factory)(block)));
        }

        if let Some(mut task) = pin.task.take()
            && task.poll_unpin(cx).is_pending()
        {
            pin.task = Some(task);
        }

        if pin.stream_done && pin.task.is_none() { Poll::Ready(()) } else { Poll::Pending }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown;
    use futures::{
        channel::{mpsc, oneshot},
        task::noop_waker,
    };
    use parking_lot::Mutex;
    use std::sync::Arc;

    #[test]
    fn waits_for_active_task_before_processing_latest_block() {
        let (_signal, shutdown) = shutdown::signal();
        let (tx, rx) = mpsc::unbounded();
        let (release_tx, release_rx) = oneshot::channel();
        let release_rx = Arc::new(Mutex::new(Some(release_rx)));
        let processed = Arc::new(Mutex::new(Vec::new()));
        let task_release_rx = release_rx;
        let task_processed = processed.clone();
        let mut listener = Box::pin(BlockListener::new(shutdown, rx, move |block| {
            let release_rx = task_release_rx.clone();
            let processed = task_processed.clone();
            async move {
                let release_rx = (block == 1).then(|| release_rx.lock().take().unwrap());
                if let Some(release_rx) = release_rx {
                    let _ = release_rx.await;
                }
                processed.lock().push(block);
            }
        }));
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        tx.unbounded_send(1).unwrap();
        assert!(listener.as_mut().poll(&mut cx).is_pending());

        tx.unbounded_send(2).unwrap();
        tx.unbounded_send(3).unwrap();
        assert!(listener.as_mut().poll(&mut cx).is_pending());
        assert!(processed.lock().is_empty());

        drop(tx);
        release_tx.send(()).unwrap();
        assert!(listener.as_mut().poll(&mut cx).is_ready());
        assert_eq!(*processed.lock(), [1, 3]);
    }
}
