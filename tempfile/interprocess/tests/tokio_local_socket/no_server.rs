//! Tests what happens when a client attempts to connect to a local socket that doesn't exist.

use crate::{
	local_socket::tokio::{prelude::*, Stream},
	tests::util::*,
};
use color_eyre::eyre::{bail, ensure};
use std::io;

pub async fn run_and_verify_error(path: bool) -> TestResult {
	use io::ErrorKind::*;
	let err = match client(path).await {
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
async fn client(path: bool) -> io::Result<()> {
	let nm = namegen_local_socket(make_id!(), path).next().unwrap();
	Stream::connect(nm?.borrow()).await?;
	Ok(())
}
