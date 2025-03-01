//{
#[cfg(not(feature = "tokio"))]
fn main() {}
#[cfg(feature = "tokio")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	//}
	use interprocess::local_socket::{
		tokio::{prelude::*, Stream},
		GenericNamespaced, ListenerOptions,
	};
	use std::io;
	use tokio::{
		io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
		try_join,
	};

	// Describe the things we do when we've got a connection ready.
	async fn handle_conn(conn: Stream) -> io::Result<()> {
		let mut recver = BufReader::new(&conn);
		let mut sender = &conn;

		// Allocate a sizeable buffer for receiving. This size should be big enough and easy to
		// find for the allocator.
		let mut buffer = String::with_capacity(128);

		// Describe the send operation as sending our whole message.
		let send = sender.write_all(b"Hello from server!\n");
		// Describe the receive operation as receiving a line into our big buffer.
		let recv = recver.read_line(&mut buffer);

		// Run both operations concurrently.
		try_join!(recv, send)?;

		// Produce our output!
		println!("Client answered: {}", buffer.trim());
		Ok(())
	}

	// Pick a name.
	let printname = "example.sock";
	let name = printname.to_ns_name::<GenericNamespaced>()?;

	// Configure our listener...
	let opts = ListenerOptions::new().name(name);

	// ...and create it.
	let listener = match opts.create_tokio() {
		Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
			// When a program that uses a file-type socket name terminates its socket server
			// without deleting the file, a "corpse socket" remains, which can neither be
			// connected to nor reused by a new listener. Normally, Interprocess takes care of
			// this on affected platforms by deleting the socket file when the listener is
			// dropped. (This is vulnerable to all sorts of races and thus can be disabled.)
			//
			// There are multiple ways this error can be handled, if it occurs, but when the
			// listener only comes from Interprocess, it can be assumed that its previous instance
			// either has crashed or simply hasn't exited yet. In this example, we leave cleanup
			// up to the user, but in a real application, you usually don't want to do that.
			eprintln!(
				"
Error: could not start server because the socket file is occupied. Please check if {printname}
is in use by another process and try again."
			);
			return Err(e.into());
		}
		x => x?,
	};

	// The syncronization between the server and client, if any is used, goes here.
	eprintln!("Server running at {printname}");

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
				eprintln!("Error while handling connection: {e}");
			}
		});
	}
} //
