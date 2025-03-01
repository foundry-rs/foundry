/// Fallible OS object cloning.
///
/// The `DuplicateHandle`/`dup` system calls can fail for a variety of reasons, most of them being
/// related to system resource exhaustion. This trait is implemented by types in Interprocess which
/// wrap OS objects (which is to say, the majority of types here) to enable handle/file descriptor
/// duplication functionality on them.
pub trait TryClone: Sized {
	/// Clones `self`, possibly returning an error.
	fn try_clone(&self) -> std::io::Result<Self>;
}
impl<T: Clone> TryClone for T {
	fn try_clone(&self) -> std::io::Result<Self> {
		Ok(self.clone())
	}
}
