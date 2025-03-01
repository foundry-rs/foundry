use std::ffi::c_void;

/// Common interface for safe access to [security descriptors][sd].
///
/// # Safety
/// The following safety constraints must be upheld by all instances of types implementing this
/// trait (ideally by marking the appropriate constructors as unsafe):
///
/// -	The `SECURITY_DESCRIPTOR` structure includes pointer fields which Windows later
/// 	dereferences. Having those pointers point to garbage, uninitialized memory or
/// 	non-dereferencable regions constitutes undefined behavior.
/// -	The pointers contained inside must not be aliased by mutable references. They are only to be
/// 	accessed using Windows API functions such as `SetEntriesInAcl()`.
/// -	`IsValidSecurityDescriptor()` must return `true` for the given value.
///
/// Code that consumes types implementing `AsSecurityDescriptor` can rely on those things being
/// true.
///
/// [sd]: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-security_descriptor
pub unsafe trait AsSecurityDescriptor {
	/// Returns a pointer to a security descriptor as accepted by functions of the Windows API.
	///
	/// It is assumed that this pointer is not mutably aliased and cannot be used for modification.
	fn as_sd(&self) -> *const c_void;
}

/// Like [`AsSecurityDescriptor`], but allows mutation.
///
/// # Safety
/// See [`AsSecurityDescriptor`](AsSecurityDescriptor#safety).
pub unsafe trait AsSecurityDescriptorMut: AsSecurityDescriptor {
	/// Returns a pointer to a security descriptor as accepted by functions of the Windows API.
	///
	/// It is assumed that this pointer isn't aliased and can be used for modification.
	fn as_sd_mut(&mut self) -> *mut c_void;
}
