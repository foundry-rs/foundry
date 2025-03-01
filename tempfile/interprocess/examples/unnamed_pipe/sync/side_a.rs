//{
use std::{io, os, sync::mpsc};
#[cfg(windows)]
type Handle = os::windows::io::OwnedHandle;
#[cfg(unix)]
type Handle = os::unix::io::OwnedFd;
pub(crate) fn emain(handle_sender: mpsc::SyncSender<Handle>) -> io::Result<()> {
	//}
	use interprocess::unnamed_pipe::pipe;
	use std::io::{prelude::*, BufReader};

	// Create the unnamed pipe, yielding a sender and a receiver.
	let (tx, rx) = pipe()?;

	// Let's extract the raw handle or file descriptor of the sender. Note that `OwnedHandle` and
	// `OwnedFd` both implement `From<unnamed_pipe::Sender>`.
	let txh = tx.into();
	// Now deliver `txh` to the child process. This may be done by starting it here with a
	// command-line argument or via stdin. This works because child processes inherit handles and
	// file descriptors to unnamed pipes. You can also use a different platform-specific way of
	// transferring handles or file descriptors across a process boundary.
	//{
	handle_sender.send(txh).unwrap();
	//}

	let mut buf = String::with_capacity(128);
	// We'd like to receive a line, so buffer our input.
	let mut rx = BufReader::new(rx);
	// Receive the line from the other process.
	rx.read_line(&mut buf)?;

	assert_eq!(buf.trim(), "Hello from side B!");
	//{
	Ok(())
}
#[allow(dead_code)]
fn main() {} //}
