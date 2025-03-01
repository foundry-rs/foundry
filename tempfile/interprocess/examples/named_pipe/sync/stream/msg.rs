//{
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() -> std::io::Result<()> {
	//}
	use interprocess::os::windows::named_pipe::*;
	use recvmsg::prelude::*;
	// Preemptively allocate a sizeable buffer for receiving. Keep in mind that this will depend
	// on the specifics of the protocol you're using.
	let mut buffer = MsgBuf::from(Vec::with_capacity(128));

	// Create our connection. This will block until the server accepts our connection, but will
	// fail immediately if the server hasn't even started yet; somewhat similar to how happens
	// with TCP, where connecting to a port that's not bound to any server will send a "connection
	// refused" response, but that will take twice the ping, the roundtrip time, to reach the
	// client.
	let mut conn = DuplexPipeStream::<pipe_mode::Messages>::connect_by_path(r"\\.\pipe\Example")?;

	// Here's our message so that we could check its length later.
	static MESSAGE: &[u8] = b"Hello from client!";
	// Send the message, getting the amount of bytes that was actually sent in return.
	let sent = conn.send(MESSAGE)?;
	assert_eq!(sent, MESSAGE.len()); // If it doesn't match, something's seriously wrong.

	// Use the reliable message receive API, which gets us a `RecvResult` from the
	// `reliable_recv_msg` module.
	conn.recv_msg(&mut buffer, None)?;

	// Convert the data that's been received into a string. This checks for UTF-8 validity, and if
	// invalid characters are found, a new buffer is allocated to house a modified version of the
	// received data, where decoding errors are replaced with those diamond-shaped question mark
	// U+FFFD REPLACEMENT CHARACTER thingies: �.
	let received_string = String::from_utf8_lossy(buffer.filled_part());

	// Print out the result!
	println!("Server answered: {received_string}");
	//{
	Ok(())
} //}
