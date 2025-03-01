use super::super::name_to_addr;
use crate::{
	error::ReuniteError,
	local_socket::{traits::tokio as traits, Name},
	Sealed,
};
use std::{
	io::{self, ErrorKind::WouldBlock},
	os::{
		fd::{AsFd, OwnedFd},
		unix::{
			net::{SocketAddr, UnixStream as SyncUnixStream},
			prelude::BorrowedFd,
		},
	},
	pin::Pin,
	task::{ready, Context, Poll},
};
use tokio::{
	io::{AsyncRead, AsyncWrite, ReadBuf},
	net::{
		unix::{OwnedReadHalf as RecvHalfImpl, OwnedWriteHalf as SendHalfImpl},
		UnixStream,
	},
};

#[derive(Debug)]
pub struct Stream(pub(super) UnixStream);
impl Sealed for Stream {}

impl Stream {
	#[allow(clippy::unwrap_used)]
	async fn _connect(addr: SocketAddr) -> io::Result<UnixStream> {
		#[cfg(any(target_os = "linux", target_os = "android"))]
		{
			#[cfg(target_os = "android")]
			use std::os::android::net::SocketAddrExt;
			#[cfg(target_os = "linux")]
			use std::os::linux::net::SocketAddrExt;
			if addr.as_abstract_name().is_some() {
				return tokio::task::spawn_blocking(move || {
					let stream = SyncUnixStream::connect_addr(&addr)?;
					stream.set_nonblocking(true)?;
					Ok::<_, io::Error>(stream)
				})
				.await??
				.try_into();
			}
		}
		UnixStream::connect(addr.as_pathname().unwrap()).await
	}
}

impl traits::Stream for Stream {
	type RecvHalf = RecvHalf;
	type SendHalf = SendHalf;

	async fn connect(name: Name<'_>) -> io::Result<Self> {
		Self::_connect(name_to_addr(name, false)?)
			.await
			.map(Self::from)
	}
	fn split(self) -> (RecvHalf, SendHalf) {
		let (r, w) = self.0.into_split();
		(RecvHalf(r), SendHalf(w))
	}
	#[inline]
	fn reunite(rh: RecvHalf, sh: SendHalf) -> Result<Self, ReuniteError<RecvHalf, SendHalf>> {
		rh.0.reunite(sh.0)
			.map(Self::from)
			.map_err(|tokio::net::unix::ReuniteError(rh, sh)| ReuniteError {
				rh: RecvHalf(rh),
				sh: SendHalf(sh),
			})
	}
}

fn ioloop(
	mut try_io: impl FnMut() -> io::Result<usize>,
	mut poll_read_ready: impl FnMut() -> Poll<io::Result<()>>,
) -> Poll<io::Result<usize>> {
	loop {
		match try_io() {
			Err(e) if e.kind() == WouldBlock => ready!(poll_read_ready()?),
			els => return Poll::Ready(els),
		};
	}
}

multimacro! {
	Stream,
	pinproj_for_unpin(UnixStream),
	forward_rbv(UnixStream, &),
	forward_tokio_rw,
	forward_as_handle(unix),
	derive_trivial_conv(UnixStream),
}
impl AsyncRead for &Stream {
	#[inline]
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<io::Result<()>> {
		ioloop(|| self.0.try_read_buf(buf), || self.0.poll_read_ready(cx)).map(|e| e.map(|_| ()))
	}
}
impl AsyncWrite for &Stream {
	#[inline]
	fn poll_write(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<io::Result<usize>> {
		ioloop(|| self.0.try_write(buf), || self.0.poll_write_ready(cx))
	}
	#[inline]
	fn poll_write_vectored(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		bufs: &[io::IoSlice<'_>],
	) -> Poll<io::Result<usize>> {
		ioloop(
			|| self.0.try_write_vectored(bufs),
			|| self.0.poll_write_ready(cx),
		)
	}
	#[inline]
	fn is_write_vectored(&self) -> bool {
		self.0.is_write_vectored()
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
impl TryFrom<Stream> for OwnedFd {
	type Error = io::Error;
	#[inline]
	fn try_from(slf: Stream) -> io::Result<Self> {
		Ok(slf.0.into_std()?.into())
	}
}
impl TryFrom<OwnedFd> for Stream {
	type Error = io::Error;
	#[inline]
	fn try_from(fd: OwnedFd) -> io::Result<Self> {
		Ok(UnixStream::from_std(SyncUnixStream::from(fd))?.into())
	}
}

pub struct RecvHalf(RecvHalfImpl);
impl Sealed for RecvHalf {}
impl traits::RecvHalf for RecvHalf {
	type Stream = Stream;
}
multimacro! {
	RecvHalf,
	pinproj_for_unpin(RecvHalfImpl),
	forward_debug("local_socket::RecvHalf"),
	forward_tokio_read,
}
impl AsyncRead for &RecvHalf {
	#[inline]
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<io::Result<()>> {
		ioloop(
			|| self.0.try_read_buf(buf),
			|| self.0.as_ref().poll_read_ready(cx),
		)
		.map(|e| e.map(|_| ()))
	}
}
impl AsFd for RecvHalf {
	#[inline]
	fn as_fd(&self) -> BorrowedFd<'_> {
		self.0.as_ref().as_fd()
	}
}

pub struct SendHalf(SendHalfImpl);
impl Sealed for SendHalf {}
impl traits::SendHalf for SendHalf {
	type Stream = Stream;
}
multimacro! {
	SendHalf,
	pinproj_for_unpin(SendHalfImpl),
	forward_rbv(SendHalfImpl, &),
	forward_debug("local_socket::SendHalf"),
	forward_tokio_write,
}
impl AsyncWrite for &SendHalf {
	#[inline]
	fn poll_write(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<io::Result<usize>> {
		ioloop(
			|| self.0.try_write(buf),
			|| self.0.as_ref().poll_write_ready(cx),
		)
	}
	#[inline]
	fn poll_write_vectored(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		bufs: &[io::IoSlice<'_>],
	) -> Poll<io::Result<usize>> {
		ioloop(
			|| self.0.try_write_vectored(bufs),
			|| self.0.as_ref().poll_write_ready(cx),
		)
	}
	#[inline]
	fn is_write_vectored(&self) -> bool {
		self.0.is_write_vectored()
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
impl AsFd for SendHalf {
	#[inline]
	fn as_fd(&self) -> BorrowedFd<'_> {
		self.0.as_ref().as_fd()
	}
}
