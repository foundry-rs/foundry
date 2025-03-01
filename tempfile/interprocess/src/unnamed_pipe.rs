//! Creation and usage of unnamed pipes.
//!
//! Unlike named pipes, unnamed pipes are only accessible through their handles – once an endpoint
//! is closed, its corresponding end of the pipe is no longer accessible. Unnamed pipes typically
//! work best when communicating with child processes.
//!
//! The handles and file descriptors are inheritable by default. The `AsRawHandle` and `AsRawFd`
//! traits can be used to get a numeric handle value which can then be communicated to a child
//! process using a command-line argument, environment variable or some other program startup IPC
//! method. The numeric value can then be reconstructed into an I/O object using
//! `FromRawHandle`/`FromRawFd`. Interprocess does not concern itself with how this is done.
//!
//! Note
//! [the standard library's support for piping `stdin`, `stdout` and `stderr`](std::process::Stdio),
//! which can be used in simple cases instead of unnamed pipes. Making use of that feature is
//! advisable if the program of the child process can be modified to communicate with its parent
//! via standard I/O streams.
//!
//! # Examples
//! See [`pipe()`].

#[cfg(feature = "tokio")]
#[cfg_attr(feature = "doc_cfg", doc(cfg(feature = "tokio")))]
pub mod tokio;

impmod! {unnamed_pipe,
	Recver as RecverImpl,
	Sender as SenderImpl,
	pipe_impl,
}
use crate::Sealed;
use std::io;

/// Creates a new pipe with the default creation settings and returns the handles to its sending end
/// and receiving end.
///
/// The platform-specific builders in the `os` module of the crate might be more helpful if extra
/// configuration for the pipe is needed.
///
/// # Examples
/// ## Basic communication
/// In a parent process:
/// ```no_run
#[doc = doctest_file::include_doctest!("examples/unnamed_pipe/sync/side_a.rs")]
/// ```
/// In a child process:
/// ```no_run
#[doc = doctest_file::include_doctest!("examples/unnamed_pipe/sync/side_b.rs")]
/// ```
#[inline]
pub fn pipe() -> io::Result<(Sender, Recver)> {
	pipe_impl()
}

/// Handle to the receiving end of an unnamed pipe, created by the [`pipe()`] function together
/// with the [sending end](Sender).
///
/// The core functionality is exposed via the [`Read`](io::Read) trait. The type is convertible to
/// and from handles/file descriptors and allows its internal handle/FD to be borrowed. On
/// Windows, the `ShareHandle` trait is also implemented.
///
/// The handle/file descriptor is inheritable. See [module-level documentation](self) for more on
/// how this can be used.
// field is pub(crate) to allow platform builders to create the public-facing pipe types
pub struct Recver(pub(crate) RecverImpl);
impl Sealed for Recver {}
multimacro! {
	Recver,
	forward_sync_read,
	forward_handle,
	forward_debug,
	derive_raw,
}

/// Handle to the sending end of an unnamed pipe, created by the [`pipe()`] function together with
/// the [receiving end](Recver).
///
/// The core functionality is exposed via the [`Write`](io::Write) trait. The type is convertible
/// to and from handles/file descriptors and allows its internal handle/FD to be borrowed. On
/// Windows, the `ShareHandle` trait is also implemented.
///
/// The handle/file descriptor is inheritable. See [module-level documentation](self) for more on
/// how this can be used.
///
/// # Limbo
/// On Windows, much like named pipes, unnamed pipes are subject to limbo, meaning that dropping
/// an unnamed pipe does not immediately discard the contents of the send buffer. See the
/// documentation on `named_pipe::PipeStream` for more.
///
/// [ARH]: https://doc.rust-lang.org/std/os/windows/io/trait.AsRawHandle.html
/// [IRH]: https://doc.rust-lang.org/std/os/windows/io/trait.IntoRawHandle.html
/// [`FromRawHandle`]: https://doc.rust-lang.org/std/os/windows/io/trait.FromRawHandle.html
/// [ARF]: https://doc.rust-lang.org/std/os/unix/io/trait.AsRawFd.html
/// [IRF]: https://doc.rust-lang.org/std/os/unix/io/trait.IntoRawFd.html
/// [`FromRawFd`]: https://doc.rust-lang.org/std/os/unix/io/trait.FromRawFd.html
pub struct Sender(pub(crate) SenderImpl);
impl Sealed for Sender {}
multimacro! {
	Sender,
	forward_sync_write,
	forward_handle,
	forward_debug,
	derive_raw,
}
