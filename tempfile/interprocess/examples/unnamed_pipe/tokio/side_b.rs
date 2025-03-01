//{
use std::{io, os};
#[cfg(windows)]
type Handle = os::windows::io::OwnedHandle;
#[cfg(unix)]
type Handle = os::unix::io::OwnedFd;
pub(crate) async fn emain(handle: Handle) -> io::Result<()> {
	//}
	use interprocess::unnamed_pipe;
	use tokio::io::AsyncWriteExt;

	// `handle` here is an `OwnedHandle` or an `OwnedFd` from the standard library. Those
	// implement `FromRawHandle` and `FromRawFd` respectively. The actual value can be transferred
	// via a command-line parameter since it's numerically equal to the value obtained in the
	// parent process via `OwnedHandle::try_from()`/`OwnedFd::try_from()` thanks to handle
	// inheritance.
	let mut tx = unnamed_pipe::tokio::Sender::try_from(handle)?;

	// Send our message to the other side.
	tx.write_all(b"Hello from side B!\n").await?;
	//{
	Ok(())
}
#[allow(dead_code)]
fn main() {} //}
