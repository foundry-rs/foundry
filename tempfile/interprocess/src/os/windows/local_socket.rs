//! Windows-specific local socket functionality.

pub(crate) mod dispatch_sync;
#[cfg(feature = "tokio")]
pub(crate) mod dispatch_tokio;
pub(crate) mod name_type;

pub use name_type::*;

use super::security_descriptor::SecurityDescriptor;
use crate::{local_socket::ListenerOptions, Sealed};

/// Windows-specific [listener options](ListenerOptions).
#[allow(private_bounds)]
pub trait ListenerOptionsExt: Sized + Sealed {
	/// Sets the security descriptor that will control access to the underlying named pipe.
	#[must_use = builder_must_use!()]
	fn security_descriptor(self, sd: SecurityDescriptor) -> Self;
}

impl ListenerOptionsExt for ListenerOptions<'_> {
	#[inline(always)]
	fn security_descriptor(mut self, sd: SecurityDescriptor) -> Self {
		self.security_descriptor = Some(sd);
		self
	}
}
