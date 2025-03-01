use crate::{tests::util::*, unnamed_pipe::pipe};
use std::{
	io::{BufRead, BufReader, Write},
	sync::mpsc,
	thread,
};
static MSG: &str = "Message from sender to receiver\n";

pub(super) fn main() -> TestResult {
	let (mut tx, rx) = pipe().opname("pipe creation")?;

	thread::scope(|scope| {
		let (notify, wait) = mpsc::channel();

		scope.spawn(move || {
			tx.write_all(MSG.as_bytes()).opname("send")?;
			drop(tx);
			// Test buffer retention on drop
			notify.send(()).opname("notify")?;
			TestResult::Ok(())
		});

		wait.recv().opname("wait for drop")?;
		// Sender is guaranteed to be in limbo by this point (Windows only)

		let mut buf = String::with_capacity(MSG.len());
		let mut rx = BufReader::new(rx);

		rx.read_line(&mut buf).opname("receive")?;
		ensure_eq!(buf, MSG);

		Ok(())
	})
}
