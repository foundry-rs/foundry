//! Unix-specific named pipe functionality.

use super::{c_wrappers, FdOps};
use crate::{
	os::unix::unixprelude::*,
	unnamed_pipe::{Recver as PubRecver, Sender as PubSender},
	Sealed,
};
use std::{
	fmt::{self, Debug, Formatter},
	io,
	os::fd::OwnedFd,
};

#[cfg(feature = "tokio")]
pub(crate) mod tokio;

/// Unix-specific extensions to synchronous named pipe senders and receivers.
#[allow(private_bounds)]
pub trait UnnamedPipeExt: AsFd + Sealed {
	/// Sets whether the nonblocking mode for the pipe half is enabled. By default, it is
	/// disabled.
	///
	/// In nonblocking mode, attempts to receive from a [`Recver`](PubRecver) when there is no
	/// data available, much like attempts to send data via a [`Sender`](PubSender) when the send
	/// buffer has filled up because the receiving side hasn't received enough bytes in time,
	/// never block like they normally do. Instead, a [`WouldBlock`](io::ErrorKind::WouldBlock)
	/// error is immediately returned, allowing the thread to perform useful actions in the
	/// meantime.
	#[inline]
	fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
		c_wrappers::set_nonblocking(self.as_fd(), nonblocking)
	}
}
#[cfg_attr(feature = "doc_cfg", doc(cfg(unix)))]
impl UnnamedPipeExt for PubRecver {}
#[cfg_attr(feature = "doc_cfg", doc(cfg(unix)))]
impl UnnamedPipeExt for PubSender {}

/// Like [platform-general `pipe()`](crate::unnamed_pipe::pipe), but allows pipe pairs to be
/// immediately created in nonblocking mode on Linux, eliding a `fcntl()`.
///
/// ## System calls
/// - `pipe2` (Linux)
/// - `pipe` (not Linux)
/// - `fcntl` (not Linux, only if `nonblocking` is `true`)
pub fn pipe(nonblocking: bool) -> io::Result<(PubSender, PubRecver)> {
	let (success, fds) = unsafe {
		let mut fds: [c_int; 2] = [0; 2];
		let result;
		#[cfg(any(target_os = "linux", target_os = "android"))]
		{
			result = libc::pipe2(
				fds.as_mut_ptr(),
				if nonblocking { libc::O_NONBLOCK } else { 0 },
			);
		}
		#[cfg(not(any(target_os = "linux", target_os = "android")))]
		{
			result = libc::pipe(fds.as_mut_ptr());
		}
		(result == 0, fds)
	};
	if success {
		let (w, r) = unsafe {
			// SAFETY: we just created both of those file descriptors, which means that neither of
			// them can be in use elsewhere.
			let w = OwnedFd::from_raw_fd(fds[1]);
			let r = OwnedFd::from_raw_fd(fds[0]);
			(w, r)
		};
		let w = PubSender(Sender(FdOps(w)));
		let r = PubRecver(Recver(FdOps(r)));
		#[cfg(not(any(target_os = "linux", target_os = "android")))]
		{
			if nonblocking {
				w.set_nonblocking(true)?;
				r.set_nonblocking(true)?;
			}
		}
		Ok((w, r))
	} else {
		Err(io::Error::last_os_error())
	}
}

// This is imported by a macro, hence the confusing name.
#[inline]
pub(crate) fn pipe_impl() -> io::Result<(PubSender, PubRecver)> {
	pipe(false)
}

pub(crate) struct Recver(FdOps);
impl Sealed for Recver {}
impl Debug for Recver {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Recver")
			.field("fd", &self.0 .0.as_raw_fd())
			.finish()
	}
}
multimacro! {
	Recver,
	forward_rbv(FdOps, &),
	forward_sync_ref_read,
	forward_try_clone,
	forward_handle,
	derive_sync_mut_read,
}

pub(crate) struct Sender(FdOps);
impl Sealed for Sender {}
impl Debug for Sender {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Sender")
			.field("fd", &self.0 .0.as_raw_fd())
			.finish()
	}
}

multimacro! {
	Sender,
	forward_rbv(FdOps, &),
	forward_sync_ref_write,
	forward_try_clone,
	forward_handle,
	derive_sync_mut_write,
}
