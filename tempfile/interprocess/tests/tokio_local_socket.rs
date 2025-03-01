// TODO(2.3.0) test various error conditions

mod no_server;
mod stream;

use crate::{
	local_socket::{tokio::Stream, Name},
	tests::util::{self, tokio::test_wrapper, TestResult},
};
use std::{future::Future, pin::Pin, sync::Arc};

#[allow(clippy::type_complexity)]
async fn test_stream(id: &'static str, split: bool, path: bool) -> TestResult {
	use stream::*;
	type Fut = Pin<Box<dyn Future<Output = TestResult> + Send + 'static>>;
	type F<T> = Box<dyn Fn(T) -> Fut + Send + Sync>;
	let hcl: F<Stream> = if split {
		Box::new(|conn| Box::pin(handle_client_split(conn)))
	} else {
		Box::new(|conn| Box::pin(handle_client_nosplit(conn)))
	};
	let client: F<Arc<Name<'static>>> = if split {
		Box::new(|conn| Box::pin(client_split(conn)))
	} else {
		Box::new(|conn| Box::pin(client_nosplit(conn)))
	};
	util::tokio::drive_server_and_multiple_clients(
		move |s, n| server(id, hcl, s, n, path),
		client,
	)
	.await
}

macro_rules! matrix {
	(@body $split:ident $path:ident) => {
		test_wrapper(test_stream(make_id!(), $split, $path))
	};
	($nm:ident false $path:ident) => {
		#[test]
		fn $nm() -> TestResult { matrix!(@body false $path) }
	};
	($nm:ident true $path:ident) => {
		#[test]
		#[cfg(not(windows))]
		fn $nm() -> TestResult { matrix!(@body true $path) }
	};
	($($nm:ident $split:ident $path:ident)+) => { $(matrix!($nm $split $path);)+ };
}

matrix! {
	stream_file_nosplit			false	true
	stream_file_split			true	true
	stream_namespaced_nosplit	false	false
	stream_namespaced_split		true	false
}

#[test]
fn no_server_file() -> TestResult {
	test_wrapper(no_server::run_and_verify_error(true))
}
#[test]
fn no_server_namespaced() -> TestResult {
	test_wrapper(no_server::run_and_verify_error(false))
}
