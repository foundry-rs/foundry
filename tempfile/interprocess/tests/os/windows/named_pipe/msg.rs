use super::drive_server;
use crate::{
	os::windows::named_pipe::{
		pipe_mode, DuplexPipeStream, PipeListener, PipeMode, RecvPipeStream, SendPipeStream,
	},
	tests::util::*,
	SubUsizeExt,
};
use color_eyre::eyre::{ensure, WrapErr};
use recvmsg::{MsgBuf, RecvMsg, RecvResult};
use std::{
	str,
	sync::{mpsc::Sender, Arc},
};

fn msgs(server: bool) -> [Box<str>; 2] {
	[
		message(Some(format_args!("First")), server, None),
		message(Some(format_args!("Second")), server, None),
	]
}
fn futf8(m: &[u8]) -> TestResult<&str> {
	str::from_utf8(m).context("received message was not valid UTF-8")
}

fn handle_conn_duplex(
	listener: &mut PipeListener<pipe_mode::Messages, pipe_mode::Messages>,
) -> TestResult {
	let (mut recver, mut sender) = listener.accept().opname("accept")?.split();

	let [msg1, msg2] = msgs(false);
	recv(&mut recver, msg1, 0)?;
	recv(&mut recver, msg2, 1)?;

	let [msg1, msg2] = msgs(true);
	send(&mut sender, msg1, 0)?;
	send(&mut sender, msg2, 1)?;

	DuplexPipeStream::reunite(recver, sender).opname("reunite")?;
	Ok(())
}
fn handle_conn_cts(
	listener: &mut PipeListener<pipe_mode::Messages, pipe_mode::None>,
) -> TestResult {
	let mut recver = listener.accept().opname("accept")?;
	let [msg1, msg2] = msgs(false);
	recv(&mut recver, msg1, 0)?;
	recv(&mut recver, msg2, 1)
}
fn handle_conn_stc(
	listener: &mut PipeListener<pipe_mode::None, pipe_mode::Messages>,
) -> TestResult {
	let mut sender = listener.accept().opname("accept")?;
	let [msg1, msg2] = msgs(true);
	send(&mut sender, msg1, 0)?;
	send(&mut sender, msg2, 1)
}

pub fn server_duplex(id: &str, name_sender: Sender<Arc<str>>, num_clients: u32) -> TestResult {
	drive_server(
		id,
		name_sender,
		num_clients,
		|plo| {
			plo.mode(PipeMode::Messages)
				.create_duplex::<pipe_mode::Messages>()
		},
		handle_conn_duplex,
	)
}
pub fn server_cts(id: &str, name_sender: Sender<Arc<str>>, num_clients: u32) -> TestResult {
	drive_server(
		id,
		name_sender,
		num_clients,
		|plo| {
			plo.mode(PipeMode::Messages)
				.create_recv_only::<pipe_mode::Messages>()
		},
		handle_conn_cts,
	)
}
pub fn server_stc(id: &str, name_sender: Sender<Arc<str>>, num_clients: u32) -> TestResult {
	drive_server(
		id,
		name_sender,
		num_clients,
		|plo| {
			plo.mode(PipeMode::Messages)
				.create_send_only::<pipe_mode::Messages>()
		},
		handle_conn_stc,
	)
}

pub fn client_duplex(name: &str) -> TestResult {
	let (mut recver, mut sender) = DuplexPipeStream::<pipe_mode::Messages>::connect_by_path(name)
		.opname("connect")?
		.split();

	let [msg1, msg2] = msgs(false);
	send(&mut sender, msg1, 0)?;
	send(&mut sender, msg2, 1)?;

	let [msg1, msg2] = msgs(true);
	recv(&mut recver, msg1, 0)?;
	recv(&mut recver, msg2, 1)?;

	DuplexPipeStream::reunite(recver, sender).opname("reunite")?;
	Ok(())
}
pub fn client_cts(name: &str) -> TestResult {
	let mut sender =
		SendPipeStream::<pipe_mode::Messages>::connect_by_path(name).opname("connect")?;
	let [msg1, msg2] = msgs(false);
	send(&mut sender, msg1, 0)?;
	send(&mut sender, msg2, 1)
}
pub fn client_stc(name: &str) -> TestResult {
	let mut recver =
		RecvPipeStream::<pipe_mode::Messages>::connect_by_path(name).opname("connect")?;
	let [msg1, msg2] = msgs(true);
	recv(&mut recver, msg1, 0)?;
	recv(&mut recver, msg2, 1)
}

fn recv(
	conn: &mut RecvPipeStream<pipe_mode::Messages>,
	exp: impl AsRef<str>,
	nr: u8,
) -> TestResult {
	let fs = ["first", "second"][nr.to_usize()];
	let exp_ = exp.as_ref();
	let mut len = exp_.len();
	if nr == 2 {
		len -= 1; // tests spill
	}
	let mut buf = MsgBuf::from(Vec::with_capacity(len));

	let rslt = conn
		.recv_msg(&mut buf, None)
		.with_context(|| format!("{} receive failed", fs))?;

	ensure_eq!(futf8(buf.filled_part())?, exp_);
	if nr == 2 {
		ensure!(matches!(rslt, RecvResult::Spilled));
	} else {
		ensure!(matches!(rslt, RecvResult::Fit));
	}
	Ok(())
}

fn send(
	conn: &mut SendPipeStream<pipe_mode::Messages>,
	msg: impl AsRef<str>,
	nr: u8,
) -> TestResult {
	let msg_ = msg.as_ref();
	let fs = ["first", "second"][nr.to_usize()];

	let sent = conn
		.send(msg_.as_bytes())
		.wrap_err_with(|| format!("{} send failed", fs))?;

	ensure_eq!(sent, msg_.len());
	Ok(())
}
