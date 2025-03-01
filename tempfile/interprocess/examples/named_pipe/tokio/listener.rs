//{
#[cfg(not(all(windows, feature = "tokio")))]
fn main() {}
#[cfg(all(windows, feature = "tokio"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	//}
	use interprocess::os::windows::named_pipe::{pipe_mode, tokio::*, PipeListenerOptions};
	use std::{io, path::Path};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		try_join,
	};

	// Describe the things we do when we've got a connection ready.
	async fn handle_conn(conn: DuplexPipeStream<pipe_mode::Bytes>) -> io::Result<()> {
		// Split the connection into two halves to process received and sent data concurrently.
		let (mut recver, mut sender) = conn.split();

		// Allocate a sizeable buffer for receiving. This size should be enough and should be easy
		// to find for the allocator.
		let mut buffer = String::with_capacity(128);

		// Describe the send operation as first sending our whole message, and then shutting down
		// the send half to send an EOF to help the other side determine the end of the
		// transmission.
		let send = async {
			sender.write_all(b"Hello from server!").await?;
			sender.shutdown().await?;
			Ok(())
		};

		// Describe the receive operation as receiving into our big buffer.
		let recv = recver.read_to_string(&mut buffer);

		// Run both the send-and-invoke-EOF operation and the receive operation concurrently.
		try_join!(recv, send)?;

		// Dispose of our connection right now and not a moment later because I want to!
		drop((recver, sender));

		// Produce our output!
		println!("Client answered: {}", buffer.trim());
		Ok(())
	}

	static PIPE_NAME: &str = "Example";

	// Create our listener.
	let listener = PipeListenerOptions::new()
		.path(Path::new(PIPE_NAME))
		.create_tokio_duplex::<pipe_mode::Bytes>()?;

	// The syncronization between the server and client, if any is used, goes here.
	eprintln!(r"Server running at \\.\pipe\{PIPE_NAME}");

	// Set up our loop boilerplate that processes our incoming connections.
	loop {
		// Sort out situations when establishing an incoming connection caused an error.
		let conn = match listener.accept().await {
			Ok(c) => c,
			Err(e) => {
				eprintln!("There was an error with an incoming connection: {e}");
				continue;
			}
		};

		// Spawn new parallel asynchronous tasks onto the Tokio runtime and hand the connection
		// over to them so that multiple clients could be processed simultaneously in a
		// lightweight fashion.
		tokio::spawn(async move {
			// The outer match processes errors that happen when we're connecting to something.
			// The inner if-let processes errors that happen during the connection.
			if let Err(e) = handle_conn(conn).await {
				eprintln!("error while handling connection: {e}");
			}
		});
	}
} //
