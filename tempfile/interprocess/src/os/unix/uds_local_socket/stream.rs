use super::name_to_addr;
use crate::{
	error::ReuniteError,
	local_socket::{
		traits::{self, ReuniteResult},
		ConcurrencyDetector, LocalSocketSite, Name,
	},
	Sealed, TryClone,
};
use std::{
	io::{self, prelude::*, IoSlice, IoSliceMut},
	os::{fd::OwnedFd, unix::net::UnixStream},
	sync::Arc,
};

/// Wrapper around [`UnixStream`] that implements
/// [`Stream`](crate::local_socket::traits::Stream).
#[derive(Debug)]
pub struct Stream(pub(super) UnixStream, ConcurrencyDetector<LocalSocketSite>);
impl Sealed for Stream {}
impl traits::Stream for Stream {
	type RecvHalf = RecvHalf;
	type SendHalf = SendHalf;

	fn connect(name: Name<'_>) -> io::Result<Self> {
		UnixStream::connect_addr(&name_to_addr(name, false)?).map(Self::from)
	}
	#[inline]
	fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
		self.0.set_nonblocking(nonblocking)
	}
	#[inline]
	fn split(self) -> (RecvHalf, SendHalf) {
		let arc = Arc::new(self);
		(RecvHalf(Arc::clone(&arc)), SendHalf(arc))
	}
	#[inline]
	#[allow(clippy::unwrap_in_result)]
	fn reunite(rh: RecvHalf, sh: SendHalf) -> ReuniteResult<Self> {
		if !Arc::ptr_eq(&rh.0, &sh.0) {
			return Err(ReuniteError { rh, sh });
		}
		drop(rh);
		let inner = Arc::into_inner(sh.0).expect("stream half inexplicably copied");
		Ok(inner)
	}
}

impl Read for &Stream {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let _guard = self.1.lock();
		(&mut &self.0).read(buf)
	}
	fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
		let _guard = self.1.lock();
		(&mut &self.0).read_vectored(bufs)
	}
	// FUTURE is_read_vectored
}
impl Write for &Stream {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let _guard = self.1.lock();
		(&mut &self.0).write(buf)
	}
	fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
		let _guard = self.1.lock();
		(&mut &self.0).write_vectored(bufs)
	}
	#[inline]
	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
	// FUTURE is_write_vectored
}

impl From<UnixStream> for Stream {
	fn from(s: UnixStream) -> Self {
		Self(s, ConcurrencyDetector::new())
	}
}

impl From<OwnedFd> for Stream {
	fn from(fd: OwnedFd) -> Self {
		UnixStream::from(fd).into()
	}
}

impl TryClone for Stream {
	#[inline]
	fn try_clone(&self) -> std::io::Result<Self> {
		self.0.try_clone().map(Self::from)
	}
}

multimacro! {
	Stream,
	forward_asinto_handle(unix),
	derive_sync_mut_rw,
}

/// [`Stream`]'s receive half, implemented using [`Arc`].
#[derive(Debug)]
pub struct RecvHalf(pub(super) Arc<Stream>);
impl Sealed for RecvHalf {}
impl traits::RecvHalf for RecvHalf {
	type Stream = Stream;
}
multimacro! {
	RecvHalf,
	forward_rbv(Stream, *),
	forward_sync_ref_read,
	forward_as_handle,
	derive_sync_mut_read,
}

/// [`Stream`]'s send half, implemented using [`Arc`].
#[derive(Debug)]
pub struct SendHalf(pub(super) Arc<Stream>);
impl Sealed for SendHalf {}
impl traits::SendHalf for SendHalf {
	type Stream = Stream;
}
multimacro! {
	SendHalf,
	forward_rbv(Stream, *),
	forward_sync_ref_write,
	forward_as_handle,
	derive_sync_mut_write,
}
