use std::{cmp, io};

use bytes::{Buf, Bytes};
use hyper::rt::{Read, ReadBufCursor, Write};

use std::{
    pin::Pin,
    task::{self, Poll},
};

/// Combine a buffer with an IO, rewinding reads to use the buffer.
#[derive(Debug)]
pub(crate) struct Rewind<T> {
    pub(crate) pre: Option<Bytes>,
    pub(crate) inner: T,
}

impl<T> Rewind<T> {
    #[cfg(all(feature = "server", any(feature = "http1", feature = "http2")))]
    pub(crate) fn new_buffered(io: T, buf: Bytes) -> Self {
        Rewind {
            pre: Some(buf),
            inner: io,
        }
    }
}

impl<T> Read for Rewind<T>
where
    T: Read + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        mut buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(mut prefix) = self.pre.take() {
            // If there are no remaining bytes, let the bytes get dropped.
            if !prefix.is_empty() {
                let copy_len = cmp::min(prefix.len(), remaining(&mut buf));
                // TODO: There should be a way to do following two lines cleaner...
                put_slice(&mut buf, &prefix[..copy_len]);
                prefix.advance(copy_len);
                // Put back what's left
                if !prefix.is_empty() {
                    self.pre = Some(prefix);
                }

                return Poll::Ready(Ok(()));
            }
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

fn remaining(cursor: &mut ReadBufCursor<'_>) -> usize {
    // SAFETY:
    // We do not uninitialize any set bytes.
    unsafe { cursor.as_mut().len() }
}

// Copied from `ReadBufCursor::put_slice`.
// If that becomes public, we could ditch this.
fn put_slice(cursor: &mut ReadBufCursor<'_>, slice: &[u8]) {
    assert!(
        remaining(cursor) >= slice.len(),
        "buf.len() must fit in remaining()"
    );

    let amt = slice.len();

    // SAFETY:
    // the length is asserted above
    unsafe {
        cursor.as_mut()[..amt]
            .as_mut_ptr()
            .cast::<u8>()
            .copy_from_nonoverlapping(slice.as_ptr(), amt);
        cursor.advance(amt);
    }
}

impl<T> Write for Rewind<T>
where
    T: Write + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

/*
#[cfg(test)]
mod tests {
    use super::Rewind;
    use bytes::Bytes;
    use tokio::io::AsyncReadExt;

    #[cfg(not(miri))]
    #[tokio::test]
    async fn partial_rewind() {
        let underlying = [104, 101, 108, 108, 111];

        let mock = tokio_test::io::Builder::new().read(&underlying).build();

        let mut stream = Rewind::new(mock);

        // Read off some bytes, ensure we filled o1
        let mut buf = [0; 2];
        stream.read_exact(&mut buf).await.expect("read1");

        // Rewind the stream so that it is as if we never read in the first place.
        stream.rewind(Bytes::copy_from_slice(&buf[..]));

        let mut buf = [0; 5];
        stream.read_exact(&mut buf).await.expect("read1");

        // At this point we should have read everything that was in the MockStream
        assert_eq!(&buf, &underlying);
    }

    #[cfg(not(miri))]
    #[tokio::test]
    async fn full_rewind() {
        let underlying = [104, 101, 108, 108, 111];

        let mock = tokio_test::io::Builder::new().read(&underlying).build();

        let mut stream = Rewind::new(mock);

        let mut buf = [0; 5];
        stream.read_exact(&mut buf).await.expect("read1");

        // Rewind the stream so that it is as if we never read in the first place.
        stream.rewind(Bytes::copy_from_slice(&buf[..]));

        let mut buf = [0; 5];
        stream.read_exact(&mut buf).await.expect("read1");
    }
}
*/
