use crate::{
	local_socket::{prelude::*, ListenerOptions, Name, Stream},
	tests::util::*,
	BoolExt, SubUsizeExt,
};
use color_eyre::eyre::WrapErr;
use std::{
	io::{BufRead, BufReader, Write},
	str,
	sync::{mpsc::Sender, Arc},
};

fn msg(server: bool, nts: bool) -> Box<str> {
	message(None, server, Some(['\n', '\0'][nts.to_usize()]))
}

pub fn server(
	id: &str,
	handle_client: fn(Stream) -> TestResult,
	name_sender: Sender<Arc<Name<'static>>>,
	num_clients: u32,
	path: bool,
) -> TestResult {
	let (name, listener) = listen_and_pick_name(&mut namegen_local_socket(id, path), |nm| {
		ListenerOptions::new().name(nm.borrow()).create_sync()
	})?;
	let _ = name_sender.send(name);
	listener
		.incoming()
		.take(num_clients.try_into().unwrap())
		.try_for_each(|conn| handle_client(conn.opname("accept")?))
}

pub fn handle_client(conn: Stream) -> TestResult {
	let mut conn = BufReader::new(conn);
	recv(&mut conn, &msg(false, false), 0)?;
	send(conn.get_mut(), &msg(true, false), 0)?;
	recv(&mut conn, &msg(false, true), 1)?;
	send(conn.get_mut(), &msg(true, true), 1)
}

pub fn client(name: &Name<'_>) -> TestResult {
	let mut conn = Stream::connect(name.borrow())
		.opname("connect")
		.map(BufReader::new)?;
	send(conn.get_mut(), &msg(false, false), 0)?;
	recv(&mut conn, &msg(true, false), 0)?;
	send(conn.get_mut(), &msg(false, true), 1)?;
	recv(&mut conn, &msg(true, true), 1)
}

fn recv(conn: &mut dyn BufRead, exp: &str, nr: u8) -> TestResult {
	let term = *exp.as_bytes().last().unwrap();
	let fs = ["first", "second"][nr.to_usize()];

	let mut buffer = Vec::with_capacity(exp.len());
	conn.read_until(term, &mut buffer)
		.wrap_err_with(|| format!("{} receive failed", fs))?;
	ensure_eq!(
		str::from_utf8(&buffer).with_context(|| format!("{} receive wasn't valid UTF-8", fs))?,
		exp,
	);
	Ok(())
}
fn send(conn: &mut dyn Write, msg: &str, nr: u8) -> TestResult {
	let fs = ["first", "second"][nr.to_usize()];
	conn.write_all(msg.as_bytes())
		.with_context(|| format!("{} socket send failed", fs))
}
