use super::drive_server;
use crate::{
	os::windows::named_pipe::{
		pipe_mode,
		tokio::{
			DuplexPipeStream, PipeListener, PipeListenerOptionsExt, RecvPipeStream,
			SendPipeStream,
		},
	},
	tests::util::{message, TestResult, WrapErrExt},
};
use std::sync::Arc;
use tokio::{
	io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
	sync::oneshot::Sender,
	try_join,
};

fn msg(server: bool) -> Box<str> {
	message(None, server, Some('\n'))
}

pub async fn server_duplex(
	id: &str,
	name_sender: Sender<Arc<str>>,
	num_clients: u32,
) -> TestResult {
	drive_server(
		id,
		name_sender,
		num_clients,
		|plo| plo.create_tokio_duplex::<pipe_mode::Bytes>(),
		handle_conn_duplex,
	)
	.await
}
pub async fn server_cts(id: &str, name_sender: Sender<Arc<str>>, num_clients: u32) -> TestResult {
	drive_server(
		id,
		name_sender,
		num_clients,
		|plo| plo.create_tokio_recv_only::<pipe_mode::Bytes>(),
		handle_conn_cts,
	)
	.await
}
pub async fn server_stc(id: &str, name_sender: Sender<Arc<str>>, num_clients: u32) -> TestResult {
	drive_server(
		id,
		name_sender,
		num_clients,
		|plo| plo.create_tokio_send_only::<pipe_mode::Bytes>(),
		handle_conn_stc,
	)
	.await
}

async fn handle_conn_duplex(
	listener: Arc<PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>>,
) -> TestResult {
	let (mut recver, mut sender) = listener.accept().await.opname("accept")?.split();
	try_join!(recv(&mut recver, msg(false)), send(&mut sender, msg(true)))?;
	DuplexPipeStream::reunite(recver, sender).opname("reunite")?;
	Ok(())
}
async fn handle_conn_cts(
	listener: Arc<PipeListener<pipe_mode::Bytes, pipe_mode::None>>,
) -> TestResult {
	let mut recver = listener.accept().await.opname("accept")?;
	recv(&mut recver, msg(false)).await
}
async fn handle_conn_stc(
	listener: Arc<PipeListener<pipe_mode::None, pipe_mode::Bytes>>,
) -> TestResult {
	let mut sender = listener.accept().await.opname("accept")?;
	send(&mut sender, msg(true)).await
}

pub async fn client_duplex(name: Arc<str>) -> TestResult {
	let (mut recver, mut sender) = DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path(&*name)
		.await
		.opname("connect")?
		.split();
	try_join!(recv(&mut recver, msg(true)), send(&mut sender, msg(false)))?;
	DuplexPipeStream::reunite(recver, sender).opname("reunite")?;
	Ok(())
}
pub async fn client_cts(name: Arc<str>) -> TestResult {
	let mut sender = SendPipeStream::<pipe_mode::Bytes>::connect_by_path(&*name)
		.await
		.opname("connect")?;
	send(&mut sender, msg(false)).await
}
pub async fn client_stc(name: Arc<str>) -> TestResult {
	let mut recver = RecvPipeStream::<pipe_mode::Bytes>::connect_by_path(&*name)
		.await
		.opname("connect")?;
	recv(&mut recver, msg(true)).await
}

async fn recv(recver: &mut RecvPipeStream<pipe_mode::Bytes>, exp: impl AsRef<str>) -> TestResult {
	let mut buffer = String::with_capacity(128);
	let mut recver = BufReader::new(recver);
	recver.read_line(&mut buffer).await.opname("receive")?;
	ensure_eq!(buffer, exp.as_ref());
	Ok(())
}
async fn send(sender: &mut SendPipeStream<pipe_mode::Bytes>, snd: impl AsRef<str>) -> TestResult {
	sender
		.write_all(snd.as_ref().as_bytes())
		.await
		.opname("send")
}
