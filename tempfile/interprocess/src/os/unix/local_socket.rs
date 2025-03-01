//! Unix-specific local socket features.

pub(crate) mod dispatch_sync;
#[cfg(feature = "tokio")]
pub(crate) mod dispatch_tokio;
pub(crate) mod name_type;

pub use name_type::*;

use crate::{local_socket::ListenerOptions, Sealed};

/// Unix-specific [listener options](ListenerOptions).
#[allow(private_bounds)]
pub trait ListenerOptionsExt: Sized + Sealed {
	/// Sets the file mode (Unix permissions) to be applied to the socket file.
	///
	/// # Implementation notes
	/// An opportunistic `fchmod()` is performed on the socket. If the system responds with a
	/// `EINVAL`, Interprocess concludes that `fchmod()` on sockets is not supported on the
	/// platform, remembers this fact in an atomic global variable and falls back to a temporary
	/// `umask` change.
	///
	/// Linux is known to support `fchmod()` on Unix domain sockets, while FreeBSD is known not to.
	///
	/// Note that the fallback behavior **inherently racy:** if you specify this mode as, say,
	/// 666₈ and have another thread create a file during the critical section between the first
	/// `umask()` call and the one performed just before returning from `.create_*()`, that file
	/// will have mode 666₈. There is nothing Interprocess can do about this, as POSIX prescribes
	/// the `umask` to be shared across threads.
	#[must_use = builder_must_use!()]
	fn mode(self, mode: libc::mode_t) -> Self;
}

impl ListenerOptionsExt for ListenerOptions<'_> {
	#[inline(always)]
	fn mode(mut self, mode: libc::mode_t) -> Self {
		self.mode = Some(mode);
		self
	}
}
