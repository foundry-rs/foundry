//! Windows-specific functionality for various interprocess communication primitives, as well as
//! Windows-specific ones.

pub mod local_socket;
pub mod named_pipe;
pub mod security_descriptor;
pub mod unnamed_pipe;
//pub mod mailslot;

mod impersonation_guard;
mod path_conversion;
mod share_handle;

pub use {impersonation_guard::*, path_conversion::*, share_handle::*};

mod file_handle;
mod limbo_pool;
pub(crate) mod misc;
mod needs_flush;

#[cfg(feature = "tokio")]
mod tokio_flusher;

mod limbo {
	pub(super) mod sync;
	#[cfg(feature = "tokio")]
	pub(super) mod tokio;

	pub(crate) static LIMBO_ERR: &str =
		"attempt to perform operation on pipe stream which has been sent off to limbo";
	pub(crate) static REBURY_ERR: &str = "attempt to bury same pipe stream twice";
}

pub(crate) use {file_handle::*, misc::*, needs_flush::*};

mod c_wrappers;
