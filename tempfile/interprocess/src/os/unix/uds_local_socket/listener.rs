use super::{name_to_addr, ReclaimGuard, Stream};
use crate::{
	local_socket::{
		traits::{self, Stream as _},
		ListenerNonblockingMode, ListenerOptions,
	},
	os::unix::c_wrappers,
};
use std::{
	io,
	iter::FusedIterator,
	os::{
		fd::{AsFd, BorrowedFd, OwnedFd},
		unix::net::UnixListener,
	},
	sync::atomic::{AtomicBool, Ordering::SeqCst},
};

/// Wrapper around [`UnixListener`] that implements
/// [`Listener`](crate::local_socket::traits::Listener).
#[derive(Debug)]
pub struct Listener {
	pub(super) listener: UnixListener,
	pub(super) reclaim: ReclaimGuard,
	pub(super) nonblocking_streams: AtomicBool,
}
impl Listener {
	fn decode_listen_error(error: io::Error) -> io::Error {
		io::Error::from(match error.kind() {
			io::ErrorKind::AlreadyExists => io::ErrorKind::AddrInUse,
			_ => return error,
		})
	}
}
impl crate::Sealed for Listener {}
impl traits::Listener for Listener {
	type Stream = Stream;

	fn from_options(options: ListenerOptions<'_>) -> io::Result<Self> {
		let nonblocking = options.nonblocking.accept_nonblocking();

		let listener = c_wrappers::bind_and_listen_with_mode(
			libc::SOCK_STREAM,
			&name_to_addr(options.name.borrow(), true)?,
			nonblocking,
			options.mode,
		)
		.map(UnixListener::from)
		.map_err(Self::decode_listen_error)?;

		if !c_wrappers::CAN_CREATE_NONBLOCKING && nonblocking {
			listener.set_nonblocking(true)?;
		}

		Ok(Self {
			listener,
			reclaim: options
				.reclaim_name
				.then(|| options.name.into_owned())
				.map(ReclaimGuard::new)
				.unwrap_or_default(),
			nonblocking_streams: AtomicBool::new(options.nonblocking.stream_nonblocking()),
		})
	}
	#[inline]
	fn accept(&self) -> io::Result<Stream> {
		// TODO(2.3.0) make use of the second return value in some shape or form
		let stream = self.listener.accept().map(|(s, _)| Stream::from(s))?;
		if self.nonblocking_streams.load(SeqCst) {
			stream.set_nonblocking(true)?;
		}
		Ok(stream)
	}
	#[inline]
	fn set_nonblocking(&self, nonblocking: ListenerNonblockingMode) -> io::Result<()> {
		use ListenerNonblockingMode::*;
		self.listener
			.set_nonblocking(matches!(nonblocking, Accept | Both))?;
		self.nonblocking_streams
			.store(matches!(nonblocking, Stream | Both), SeqCst);
		Ok(())
	}
	fn do_not_reclaim_name_on_drop(&mut self) {
		self.reclaim.forget();
	}
}
impl Iterator for Listener {
	type Item = io::Result<Stream>;
	#[inline(always)]
	fn next(&mut self) -> Option<Self::Item> {
		Some(traits::Listener::accept(self))
	}
}
impl FusedIterator for Listener {}

impl From<Listener> for UnixListener {
	fn from(mut l: Listener) -> Self {
		l.reclaim.forget();
		l.listener
	}
}

impl AsFd for Listener {
	#[inline]
	fn as_fd(&self) -> BorrowedFd<'_> {
		self.listener.as_fd()
	}
}
impl From<Listener> for OwnedFd {
	#[inline]
	fn from(l: Listener) -> Self {
		UnixListener::from(l).into()
	}
}
impl From<OwnedFd> for Listener {
	fn from(fd: OwnedFd) -> Self {
		Listener {
			listener: fd.into(),
			reclaim: ReclaimGuard::default(),
			nonblocking_streams: AtomicBool::new(false),
		}
	}
}
