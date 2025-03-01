#![cfg(windows)]

mod bytes;
mod msg;

use crate::{os::windows::named_pipe::PipeListenerOptions, tests::util::*};
use std::{
	fmt::Debug,
	io,
	path::Path,
	sync::{mpsc::Sender, Arc},
};

macro_rules! matrix {
	(@dir_s duplex) => {server_duplex}; (@dir_s stc) => {server_stc}; (@dir_s cts) => {server_cts};
	(@dir_c duplex) => {client_duplex}; (@dir_c stc) => {client_stc}; (@dir_c cts) => {client_cts};
	($($mod:ident $ty:ident $nm:ident)+) => {$(
		#[test]
		fn $nm() -> TestResult {
			use $mod::*;
			test_wrapper(|| {
				let server = matrix!(@dir_s $ty);
				drive_server_and_multiple_clients(
					|ns, nc| server(make_id!(), ns, nc),
					matrix!(@dir_c $ty),
				)
			})
		}
	)+};
}

matrix! {
	bytes	duplex	bytes_bidir
	bytes	cts		bytes_unidir_client_to_server
	bytes	stc		bytes_unidir_server_to_client
	msg		duplex	msg_bidir
	msg		cts		msg_unidir_client_to_server
	msg		stc		msg_unidir_server_to_client
}

fn drive_server<L: Debug>(
	id: &str,
	name_sender: Sender<Arc<str>>,
	num_clients: u32,
	mut createfn: impl (FnMut(PipeListenerOptions<'_>) -> io::Result<L>),
	mut acceptfn: impl FnMut(&mut L) -> TestResult,
) -> TestResult {
	let (name, mut listener) = listen_and_pick_name(&mut namegen_named_pipe(id), |nm| {
		createfn(PipeListenerOptions::new().path(Path::new(nm)))
	})?;

	let _ = name_sender.send(name);

	for _ in 0..num_clients {
		acceptfn(&mut listener)?;
	}

	Ok(())
}
