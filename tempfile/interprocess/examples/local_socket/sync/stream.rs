//{
fn main() -> std::io::Result<()> {
	//}
	use interprocess::local_socket::{prelude::*, GenericFilePath, GenericNamespaced, Stream};
	use std::io::{prelude::*, BufReader};

	// Pick a name.
	let name = if GenericNamespaced::is_supported() {
		"example.sock".to_ns_name::<GenericNamespaced>()?
	} else {
		"/tmp/example.sock".to_fs_name::<GenericFilePath>()?
	};

	// Preemptively allocate a sizeable buffer for receiving. This size should be enough and
	// should be easy to find for the allocator.
	let mut buffer = String::with_capacity(128);

	// Create our connection. This will block until the server accepts our connection, but will
	// fail immediately if the server hasn't even started yet; somewhat similar to how happens
	// with TCP, where connecting to a port that's not bound to any server will send a "connection
	// refused" response, but that will take twice the ping, the roundtrip time, to reach the
	// client.
	let conn = Stream::connect(name)?;
	// Wrap it into a buffered reader right away so that we could receive a single line out of it.
	let mut conn = BufReader::new(conn);

	// Send our message into the stream. This will finish either when the whole message has been
	// sent or if a send operation returns an error. (`.get_mut()` is to get the sender,
	// `BufReader` doesn't implement pass-through `Write`.)
	conn.get_mut().write_all(b"Hello from client!\n")?;

	// We now employ the buffer we allocated prior and receive a single line, interpreting a
	// newline character as an end-of-file (because local sockets cannot be portably shut down),
	// verifying validity of UTF-8 on the fly.
	conn.read_line(&mut buffer)?;

	// Print out the result, getting the newline for free!
	print!("Server answered: {buffer}");
	//{
	Ok(())
} //}
