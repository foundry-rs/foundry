//{
#[cfg(not(feature = "tokio"))]
fn main() {}
#[cfg(feature = "tokio")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	//}
	use interprocess::local_socket::{
		tokio::{prelude::*, Stream},
		GenericFilePath, GenericNamespaced,
	};
	use tokio::{
		io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
		try_join,
	};

	// Pick a name.
	let name = if GenericNamespaced::is_supported() {
		"example.sock".to_ns_name::<GenericNamespaced>()?
	} else {
		"/tmp/example.sock".to_fs_name::<GenericFilePath>()?
	};

	// Await this here since we can't do a whole lot without a connection.
	let conn = Stream::connect(name).await?;

	// This consumes our connection and splits it into two halves, so that we can concurrently use
	// both.
	let (recver, mut sender) = conn.split();
	let mut recver = BufReader::new(recver);

	// Allocate a sizeable buffer for receiving. This size should be enough and should be easy to
	// find for the allocator.
	let mut buffer = String::with_capacity(128);

	// Describe the send operation as writing our whole string.
	let send = sender.write_all(b"Hello from client!\n");
	// Describe the receive operation as receiving until a newline into our buffer.
	let recv = recver.read_line(&mut buffer);

	// Concurrently perform both operations.
	try_join!(send, recv)?;

	// Close the connection a bit earlier than you'd think we would. Nice practice!
	drop((recver, sender));

	// Display the results when we're done!
	println!("Server answered: {}", buffer.trim());
	//{
	Ok(())
} //}
