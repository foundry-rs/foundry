use super::Stream;
use crate::{
	local_socket::{
		prelude::*, traits::tokio as traits, ListenerNonblockingMode, ListenerOptions,
	},
	os::unix::uds_local_socket::{listener::Listener as SyncListener, ReclaimGuard},
	Sealed,
};
use std::{
	fmt::{self, Debug, Formatter},
	io,
	os::unix::prelude::*,
};
use tokio::net::UnixListener;

pub struct Listener {
	listener: UnixListener,
	reclaim: ReclaimGuard,
}
impl Sealed for Listener {}
impl traits::Listener for Listener {
	type Stream = Stream;

	fn from_options(options: ListenerOptions<'_>) -> io::Result<Self> {
		options
			.nonblocking(ListenerNonblockingMode::Both)
			.create_sync_as::<SyncListener>()
			.and_then(|mut sync| {
				let reclaim = sync.reclaim.take();
				Ok(Self {
					listener: UnixListener::from_std(sync.into())?,
					reclaim,
				})
			})
	}
	async fn accept(&self) -> io::Result<Stream> {
		let inner = self.listener.accept().await?.0;
		Ok(Stream::from(inner))
	}

	fn do_not_reclaim_name_on_drop(&mut self) {
		self.reclaim.forget();
	}
}

/// Does not assume that the sync `Listener` is in nonblocking mode, setting it to
/// `ListenerNonblockingMode::Both` automatically.
// TODO(3.0.0) remove handholding and assume nonblocking
impl TryFrom<SyncListener> for Listener {
	type Error = io::Error;
	fn try_from(mut sync: SyncListener) -> io::Result<Self> {
		sync.set_nonblocking(ListenerNonblockingMode::Both)?;
		let reclaim = sync.reclaim.take();
		Ok(Self {
			listener: UnixListener::from_std(sync.into())?,
			reclaim,
		})
	}
}

impl Debug for Listener {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Listener")
			.field("fd", &self.listener.as_raw_fd())
			.field("reclaim", &self.reclaim)
			.finish()
	}
}
impl AsFd for Listener {
	#[inline]
	fn as_fd(&self) -> BorrowedFd<'_> {
		self.listener.as_fd()
	}
}
impl TryFrom<Listener> for OwnedFd {
	type Error = io::Error;
	fn try_from(mut slf: Listener) -> io::Result<Self> {
		slf.listener.into_std().map(|s| {
			slf.reclaim.forget();
			s.into()
		})
	}
}
/// Does not assume that the listener is in nonblocking mode, setting it to
/// `ListenerNonblockingMode::Both` automatically.
impl TryFrom<OwnedFd> for Listener {
	type Error = io::Error;
	fn try_from(fd: OwnedFd) -> io::Result<Self> {
		Self::try_from(SyncListener::from(fd))
	}
}
