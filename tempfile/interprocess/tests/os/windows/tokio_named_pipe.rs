#![cfg(all(windows, feature = "tokio"))]

mod bytes;

use crate::{
	os::windows::named_pipe::PipeListenerOptions,
	tests::util::{
		listen_and_pick_name,
		tokio::{drive_server_and_multiple_clients, test_wrapper},
		TestResult,
	},
};
use color_eyre::eyre::WrapErr;
use std::{fmt::Debug, future::Future, io, path::Path, sync::Arc};
use tokio::{sync::oneshot::Sender, task};

use crate::tests::util::namegen_named_pipe;

macro_rules! matrix {
	(@dir_s duplex) => {server_duplex}; (@dir_s stc) => {server_stc}; (@dir_s cts) => {server_cts};
	(@dir_c duplex) => {client_duplex}; (@dir_c stc) => {client_stc}; (@dir_c cts) => {client_cts};
	($($mod:ident $ty:ident $nm:ident)+) => {$(
		#[test]
		fn $nm() -> TestResult {
			use $mod::*;
			test_wrapper(async {
				let server = matrix!(@dir_s $ty);
				drive_server_and_multiple_clients(
					move |ns, nc| server(make_id!(), ns, nc),
					matrix!(@dir_c $ty),
				).await
			})
		}
	)+};
}

matrix! {
	bytes duplex bytes_bidir
	bytes cts	bytes_unidir_client_to_server
	bytes stc	bytes_unidir_server_to_client
}

async fn drive_server<L: Debug, T: Future<Output = TestResult> + Send + 'static>(
	id: &str,
	name_sender: Sender<Arc<str>>,
	num_clients: u32,
	mut createfn: impl (FnMut(PipeListenerOptions<'_>) -> io::Result<L>),
	mut acceptfut: impl FnMut(Arc<L>) -> T,
) -> TestResult {
	let (name, listener) = listen_and_pick_name(&mut namegen_named_pipe(id), |nm| {
		createfn(PipeListenerOptions::new().path(Path::new(nm))).map(Arc::new)
	})?;

	let _ = name_sender.send(name);

	let mut tasks = Vec::with_capacity(num_clients.try_into().unwrap());

	for _ in 0..num_clients {
		tasks.push(task::spawn(acceptfut(Arc::clone(&listener))));
	}
	for task in tasks {
		task.await
			.wrap_err("server task panicked")?
			.wrap_err("server task returned early with error")?;
	}

	Ok(())
}
