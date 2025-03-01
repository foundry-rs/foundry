//! Unix-specific functionality for various interprocess communication primitives, as well as
//! Unix-specific ones.
//!
//! ## FIFO files
//! This type of interprocess communication similar to unnamed pipes in that they are unidirectional
//! byte channels which behave like files. The difference is that FIFO files are actual
//! (pseudo)files on the filesystem and thus can be accessed by unrelated applications (one doesn't
//! need to be spawned by another).
//!
//! FIFO files are available on all supported systems.

pub(crate) mod imports;

mod c_wrappers;
mod fdops;
// Exported into child modules specifically, not this file.
use fdops::*;

pub mod fifo_file;
pub mod local_socket;
pub mod uds_local_socket;
pub mod unnamed_pipe;

mod unixprelude {
	#[allow(unused_imports)]
	pub use libc::{c_char, c_int, c_short, gid_t, mode_t, pid_t, size_t, uid_t};
	pub use std::os::unix::prelude::*;
}
