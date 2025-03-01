//! Creation of FIFO files.
//!
//! On Windows, named pipes can be compared to Unix domain sockets: they can have multiple duplex
//! connections on a single path, and the data can be chosen to either preserve or erase the message
//! boundaries, resulting in a reliable performant alternative to TCP and UDP working in the bounds
//! of a single machine. Those Unix domain sockets are employed by `interprocess` for local sockets
//! via [an implementation provided by the standard library](std::os::unix::net).
//!
//! On Unix, named pipes, referred to as "FIFO files" in this crate, are just files which can have
//! a sender and a receiver communicating with each other in one direction without message
//! boundaries. If further receivers try to open the file, they will simply receive nothing at all;
//! if further senders are connected, the data mixes in an unpredictable way, making it unusable.
//! Therefore, FIFOs are to be used specifically to conveniently connect two applications through a
//! known path which works like a pipe and nothing else.
//!
//! ## Usage
//! The [`create_fifo()`] function serves for a FIFO file creation. Opening FIFO files works via the
//! standard [`File`](std::fs::File)s, opened either only for sending or only for receiving.
//! Deletion works the same way as with any regular file, via
//! [`remove_file()`](std::fs::remove_file).

use super::unixprelude::*;
use crate::OrErrno;
use std::{ffi::CString, io, path::Path};

/// Creates a FIFO file at the specified path with the specified permissions.
///
/// Since the `mode` parameter is masked with the [`umask`], it's best to leave it at `0o777` unless
/// a different value is desired.
///
/// ## System calls
/// -	[`mkfifo`]
///
/// [`mkfifo`]: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/mkfifo.html
/// [`umask`]: https://en.wikipedia.org/wiki/Umask
pub fn create_fifo<P: AsRef<Path>>(path: P, mode: mode_t) -> io::Result<()> {
	_create_fifo(path.as_ref(), mode)
}
fn _create_fifo(path: &Path, mode: mode_t) -> io::Result<()> {
	let path = CString::new(path.as_os_str().as_bytes())?;
	unsafe { libc::mkfifo(path.as_bytes_with_nul().as_ptr().cast(), mode) != -1 }
		.true_val_or_errno(())
}
