//! Tests what happens when a client attempts to connect to a local socket that doesn't exist.

use crate::{
	local_socket::{prelude::*, Stream},
	tests::util::*,
};
use color_eyre::eyre::{bail, ensure};
use std::io;

pub fn run_and_verify_error(id: &str, path: bool) -> TestResult {
	use io::ErrorKind::*;
	let err = match client(id, path) {
		Err(e) => e,
		Ok(()) => bail!("client successfully connected to nonexistent server"),
	};
	ensure!(
		matches!(err.kind(), NotFound | ConnectionRefused),
		"expected error to be 'not found' or 'connection refused', received '{}'",
		err
	);
	Ok(())
}
fn client(id: &str, path: bool) -> io::Result<()> {
	let nm = namegen_local_socket(id, path).next().unwrap();
	Stream::connect(nm?.borrow())?;
	Ok(())
}
