//! Tests what happens when a server attempts to listen for clients that never come.

use crate::{
	local_socket::{prelude::*, ListenerNonblockingMode, ListenerOptions, Stream},
	tests::util::*,
};
use color_eyre::eyre::{bail, ensure};
use std::io;

pub fn run_and_verify_error(id: &str, path: bool) -> TestResult {
	use io::ErrorKind::*;
	let err = match server(id, path) {
		Err(e) => e,
		Ok(c) => bail!("server successfully listened for a nonexistent client: {c:?}"),
	};
	ensure!(
		matches!(err.kind(), WouldBlock),
		"expected error to be 'would block', received '{}'",
		err
	);
	Ok(())
}
fn server(id: &str, path: bool) -> io::Result<Stream> {
	let listener = listen_and_pick_name(&mut namegen_local_socket(id, path), |nm| {
		ListenerOptions::new()
			.name(nm.borrow())
			.nonblocking(ListenerNonblockingMode::Accept)
			.create_sync()
	})
	.map_err(|e| {
		e.downcast::<io::Error>()
			.unwrap_or_else(|e| io::Error::new(io::ErrorKind::Other, e))
	})?
	.1;
	listener.accept()
}
