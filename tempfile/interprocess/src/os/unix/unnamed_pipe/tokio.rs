use super::UnnamedPipeExt;
use crate::{
	os::unix::{unixprelude::*, FdOps},
	unnamed_pipe::{
		tokio::{Recver as PubRecver, Sender as PubSender},
		Recver as SyncRecver, Sender as SyncSender,
	},
};
use std::{
	io,
	pin::Pin,
	task::{ready, Context, Poll},
};
use tokio::io::{unix::AsyncFd, AsyncRead, AsyncWrite, Interest, ReadBuf, Ready};

type RecverImpl = AsyncFd<FdOps>;
type SenderImpl = AsyncFd<FdOps>;

pub(crate) fn pipe_impl() -> io::Result<(PubSender, PubRecver)> {
	let (tx, rx) = super::pipe(true)?;
	Ok((
		PubSender(Sender::try_from_nb(tx)?),
		PubRecver(Recver::try_from_nb(rx)?),
	))
}

#[derive(Debug)]
pub(crate) struct Recver(RecverImpl);
impl Recver {
	fn try_from_nb(rx: SyncRecver) -> io::Result<Self> {
		Ok(Self(RecverImpl::with_interest(
			FdOps(rx.into()),
			Interest::READABLE,
		)?))
	}
}

impl AsyncRead for Recver {
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<io::Result<()>> {
		let slf = self.get_mut();
		loop {
			let fd = slf.0.get_ref().as_raw_fd();
			let mut readiness = ready!(slf.0.poll_read_ready_mut(cx))?;
			unsafe {
				// SAFETY(unfilled_mut): what the fuck does "de-initialize" mean
				// SAFETY(borrow_raw): we're getting it from an OwnedFd that we don't drop
				match FdOps::read_uninit(BorrowedFd::borrow_raw(fd), buf.unfilled_mut()) {
					Ok(bytes_read) => {
						buf.assume_init(bytes_read);
						buf.advance(bytes_read);
						break Poll::Ready(Ok(()));
					}
					Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
						readiness.clear_ready_matching(Ready::READABLE);
					}
					Err(e) => break Poll::Ready(Err(e)),
				}
			}
		}
	}
}

impl TryFrom<SyncRecver> for Recver {
	type Error = io::Error;
	fn try_from(rx: SyncRecver) -> io::Result<Self> {
		rx.set_nonblocking(true)?;
		Self::try_from_nb(rx)
	}
}
impl TryFrom<Recver> for OwnedFd {
	type Error = io::Error;
	fn try_from(rx: Recver) -> io::Result<Self> {
		Ok(rx.0.into_inner().into())
	}
}
impl TryFrom<OwnedFd> for Recver {
	type Error = io::Error;
	fn try_from(rx: OwnedFd) -> io::Result<Self> {
		SyncRecver::from(rx).try_into()
	}
}
forward_as_handle!(Recver);

#[derive(Debug)]
pub(crate) struct Sender(SenderImpl);
impl Sender {
	fn try_from_nb(tx: SyncSender) -> io::Result<Self> {
		Ok(Self(SenderImpl::with_interest(
			FdOps(tx.into()),
			Interest::WRITABLE,
		)?))
	}
}

impl AsyncWrite for Sender {
	fn poll_write(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<io::Result<usize>> {
		let slf = self.get_mut();
		loop {
			let fd = slf.0.get_ref().as_raw_fd();
			let mut readiness = ready!(slf.0.poll_write_ready_mut(cx))?;
			unsafe {
				// SAFETY(borrow_raw): we're getting it from an OwnedFd that we don't drop
				match FdOps::write(BorrowedFd::borrow_raw(fd), buf) {
					Ok(bytes_read) => break Poll::Ready(Ok(bytes_read)),
					Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
						readiness.clear_ready_matching(Ready::WRITABLE);
					}
					Err(e) => break Poll::Ready(Err(e)),
				}
			}
		}
	}
	#[inline]
	fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
	#[inline]
	fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
}

impl TryFrom<SyncSender> for Sender {
	type Error = io::Error;
	fn try_from(tx: SyncSender) -> io::Result<Self> {
		tx.set_nonblocking(true)?;
		Self::try_from_nb(tx)
	}
}
impl TryFrom<Sender> for OwnedFd {
	type Error = io::Error;
	fn try_from(rx: Sender) -> io::Result<Self> {
		Ok(rx.0.into_inner().into())
	}
}
impl TryFrom<OwnedFd> for Sender {
	type Error = io::Error;
	fn try_from(tx: OwnedFd) -> io::Result<Self> {
		SyncSender::from(tx).try_into()
	}
}
forward_as_handle!(Sender);
