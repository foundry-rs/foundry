//{
// TODO(2.3.0)..?
#[cfg(not(all(windows, feature = "tokio")))]
fn main() {}
#[cfg(all(windows, feature = "tokio"))]
fn main() -> std::io::Result<()> {
	//}
	//{
	Ok(())
} //}
