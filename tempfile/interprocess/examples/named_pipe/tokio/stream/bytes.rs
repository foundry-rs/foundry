//{
#[cfg(not(all(windows, feature = "tokio")))]
fn main() {}
#[cfg(all(windows, feature = "tokio"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	//}
	use interprocess::os::windows::named_pipe::{pipe_mode, tokio::*};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		try_join,
	};

	// Await this here since we can't do a whole lot without a connection.
	let conn = DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path(r"\\.\pipe\Example").await?;

	// This consumes our connection and splits it into two owned halves, so that we could
	// concurrently act on both. Take care not to use the .split() method from the futures crate's
	// AsyncReadExt.
	let (mut recver, mut sender) = conn.split();

	// Preemptively allocate a sizeable buffer for receiving. This size should be enough and
	// should be easy to find for the allocator.
	let mut buffer = String::with_capacity(128);

	// Describe the send operation as sending our whole string, waiting for that to complete, and
	// then shutting down the send half, which sends an EOF to the other end to help it determine
	// where the message ends.
	let send = async {
		sender.write_all(b"Hello from client!").await?;
		sender.shutdown().await?;
		Ok(())
	};

	// Describe the receive operation as receiving until EOF into our big buffer.
	let recv = recver.read_to_string(&mut buffer);

	// Concurrently perform both operations: send-and-invoke-EOF and receive.
	try_join!(send, recv)?;

	// Get rid of those here to close the receive half too.
	drop((recver, sender));

	// Display the results when we're done!
	println!("Server answered: {}", buffer.trim());
	//{
	Ok(())
} //}
