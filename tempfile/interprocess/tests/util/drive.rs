use super::{choke::*, TestResult, NUM_CLIENTS, NUM_CONCURRENT_CLIENTS};
use color_eyre::eyre::{bail, Context};
use std::{
	borrow::Borrow,
	io,
	sync::mpsc::{channel, /*Receiver,*/ Sender},
	thread,
};

/// Waits for the leader closure to reach a point where it sends a message for the follower closure,
/// then runs the follower. Captures Eyre errors on both sides and bubbles them up if they occur,
/// reporting which side produced the error.
pub fn drive_pair<T, Ld, Fl>(
	leader: Ld,
	leader_name: &str,
	follower: Fl,
	follower_name: &str,
) -> TestResult
where
	T: Send,
	Ld: FnOnce(Sender<T>) -> TestResult + Send,
	Fl: FnOnce(T) -> TestResult,
{
	thread::scope(|scope| {
		let (sender, receiver) = channel();

		let ltname = leader_name.to_lowercase();
		let leading_thread = thread::Builder::new()
			.name(ltname)
			.spawn_scoped(scope, move || leader(sender))
			.with_context(|| format!("{leader_name} thread launch failed"))?;

		if let Ok(msg) = receiver.recv() {
			// If the leader reached the send point, proceed with the follower code
			let rslt = follower(msg);
			exclude_deadconn(rslt)
				.with_context(|| format!("{follower_name} exited early with error"))?;
		}
		let Ok(rslt) = leading_thread.join() else {
			bail!("{leader_name} panicked");
		};
		exclude_deadconn(rslt).with_context(|| format!("{leader_name} exited early with error"))
	})
}

/// Filters errors that have to do with the other side returning an error and not bubbling it up in
/// time.
#[rustfmt::skip] // oh FUCK OFF
fn exclude_deadconn(r: TestResult) -> TestResult {
	use io::ErrorKind::*;
	let Err(e) = r else {
		return r;
	};
	let Some(ioe) = e.root_cause().downcast_ref::<io::Error>() else {
		return Err(e);
	};
	match ioe.kind() {
		ConnectionRefused
		| ConnectionReset
		| ConnectionAborted
		| NotConnected
		| BrokenPipe
		| WriteZero
		| UnexpectedEof => Ok(()),
		_ => Err(e),
	}
}

pub fn drive_server_and_multiple_clients<T, B, Srv, Clt>(server: Srv, client: Clt) -> TestResult
where
	T: Send + Borrow<B>,
	B: Send + Sync + ?Sized,
	Srv: FnOnce(Sender<T>, u32) -> TestResult + Send,
	Clt: Fn(&B) -> TestResult + Send + Sync,
{
	let choke = Choke::new(NUM_CONCURRENT_CLIENTS);

	let client_wrapper = |msg: T| {
		thread::scope(|scope| {
			let mut client_threads = Vec::with_capacity(usize::try_from(NUM_CLIENTS).unwrap());
			for n in 1..=NUM_CLIENTS {
				let tname = format!("client {n}");

				let choke_guard = choke.take();
				let (bclient, bmsg) = (&client, msg.borrow());

				let jhndl = thread::Builder::new()
					.name(tname.clone())
					.spawn_scoped(scope, move || {
						// Has to use move to send to other thread to drop when client finishes
						let _cg = choke_guard;
						bclient(bmsg)
					})
					.with_context(|| format!("{tname} thread launch failed"))?;
				client_threads.push(jhndl);
			}
			for client in client_threads {
				let Ok(rslt) = client.join() else {
					bail!("client thread panicked");
				};
				rslt?; // Early-return the first error; context not necessary as drive_pair does it
			}
			Ok(())
		})
	};
	let server_wrapper = move |sender: Sender<T>| server(sender, NUM_CLIENTS);

	drive_pair(server_wrapper, "server", client_wrapper, "client")
}
