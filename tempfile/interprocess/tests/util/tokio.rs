use super::{TestResult, WrapErrExt, NUM_CLIENTS, NUM_CONCURRENT_CLIENTS};
use color_eyre::eyre::{bail, Context};
use std::{future::Future, sync::Arc};
use tokio::{
	sync::{
		oneshot::{channel, Sender},
		Semaphore,
	},
	task, try_join,
};

pub fn test_wrapper(f: impl Future<Output = TestResult> + Send + 'static) -> TestResult {
	super::test_wrapper(|| {
		let rt = tokio::runtime::Builder::new_current_thread()
			.enable_io()
			.build()
			.opname("Tokio runtime spawn")?;
		rt.block_on(f)
	})
}

/// Waits for the leader closure to reach a point where it sends a message for the follower closure,
/// then runs the follower. Captures Eyre errors on both sides and panics if any occur, reporting
/// which side produced the error.
pub async fn drive_pair<T, Ld, Ldf, Fl, Flf>(
	leader: Ld,
	leader_name: &str,
	follower: Fl,
	follower_name: &str,
) -> TestResult
where
	Ld: FnOnce(Sender<T>) -> Ldf,
	Ldf: Future<Output = TestResult>,
	Fl: FnOnce(T) -> Flf,
	Flf: Future<Output = TestResult>,
{
	let (sender, receiver) = channel();

	let leading_task = async {
		leader(sender)
			.await
			.with_context(|| format!("{leader_name} exited early with error"))
	};
	let following_task = async {
		let msg = receiver.await?;
		follower(msg)
			.await
			.with_context(|| format!("{follower_name} exited early with error"))
	};
	try_join!(leading_task, following_task).map(|((), ())| ())
}

pub async fn drive_server_and_multiple_clients<T, Srv, Srvf, Clt, Cltf>(
	server: Srv,
	client: Clt,
) -> TestResult
where
	T: Send + Sync + ?Sized + 'static,
	Srv: FnOnce(Sender<Arc<T>>, u32) -> Srvf + Send + 'static,
	Srvf: Future<Output = TestResult>,
	Clt: Fn(Arc<T>) -> Cltf + Send + Sync + 'static,
	Cltf: Future<Output = TestResult> + Send,
{
	let client_wrapper = |msg| async move {
		let client = Arc::new(client);
		let choke = Arc::new(Semaphore::new(NUM_CONCURRENT_CLIENTS.try_into().unwrap()));

		let mut client_tasks = Vec::with_capacity(NUM_CLIENTS.try_into().unwrap());
		for _ in 0..NUM_CLIENTS {
			let permit = Arc::clone(&choke).acquire_owned().await.unwrap();
			let clientc = Arc::clone(&client);
			let msgc = Arc::clone(&msg);
			let jhndl = task::spawn(async move {
				let _prm = permit; // Send to other thread to drop when client finishes
				clientc(msgc).await
			});
			client_tasks.push(jhndl);
		}
		for client in client_tasks {
			let Ok(rslt) = client.await else {
				bail!("client task panicked");
			};
			rslt?; // Early-return the first error; context not necessary as drive_pair does it
		}
		Ok::<(), color_eyre::eyre::Error>(())
	};
	let server_wrapper = move |sender: Sender<Arc<T>>| server(sender, NUM_CLIENTS);

	drive_pair(server_wrapper, "server", client_wrapper, "client").await
}
