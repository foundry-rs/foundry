#[cfg(feature = "tokio")]
mod side_a;
#[cfg(feature = "tokio")]
mod side_b;

#[cfg(feature = "tokio")]
#[tokio::main]
async fn main() -> std::io::Result<()> {
	use tokio::{sync::oneshot, task};

	let (htx, hrx) = oneshot::channel();
	let jh = task::spawn(side_a::emain(htx));
	let handle = hrx.await.unwrap();

	side_b::emain(handle).await?;
	jh.await.unwrap()
}
#[cfg(not(feature = "tokio"))]
fn main() {}
