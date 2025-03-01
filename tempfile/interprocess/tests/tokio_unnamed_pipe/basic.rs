use crate::{
	tests::util::{TestResult, WrapErrExt},
	unnamed_pipe::tokio::pipe,
};
use tokio::{
	io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
	sync::mpsc,
	task,
};

static MSG: &str = "Message from sender to receiver\n";

pub(super) async fn main() -> TestResult {
	let (mut tx, rx) = pipe().opname("pipe creation")?;

	let (notify, mut wait) = mpsc::channel(1);
	let jh = task::spawn(async move {
		tx.write_all(MSG.as_bytes()).await.opname("send")?;
		drop(tx);
		// Test buffer retention on drop
		notify.send(()).await.opname("notify")?;
		TestResult::Ok(())
	});

	wait.recv().await.unwrap();
	// Sender is guaranteed to be in limbo by this point (Windows only)

	let mut buf = String::with_capacity(MSG.len());
	let mut rx = BufReader::new(rx);

	rx.read_line(&mut buf).await.opname("receive")?;
	ensure_eq!(buf, MSG);

	jh.await??;
	Ok(())
}
