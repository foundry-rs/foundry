use crate::AsPtr;

use super::{validate, AsSecurityDescriptor, AsSecurityDescriptorMut};
use std::{ffi::c_void, marker::PhantomData, ptr::NonNull};
use windows_sys::Win32::Security::SECURITY_DESCRIPTOR;

/// Pointer to a [security descriptor][sd] with reference-like guarantees which doesn't allow
/// mutation.
///
/// The pointee is known to be valid (and not mutably aliased) for the duration of the given
/// lifetime, just like with regular Rust references.
///
/// Unlike [`MutBorrowedSecurityDescriptor`] and the owned
/// [`SecurityDescriptor`](super::SecurityDescriptor), this type does not require the security
/// descriptor to be absolute.
///
/// [sd]: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-security_descriptor
#[repr(transparent)]
#[derive(Copy, Clone, Debug)]
pub struct BorrowedSecurityDescriptor<'a>(NonNull<c_void>, PhantomData<&'a SECURITY_DESCRIPTOR>);
/// Mutability is not provided through [`BorrowedSecurityDescriptor`].
unsafe impl Sync for BorrowedSecurityDescriptor<'_> {}
unsafe impl Send for BorrowedSecurityDescriptor<'_> {}

unsafe impl AsSecurityDescriptor for BorrowedSecurityDescriptor<'_> {
	#[inline(always)]
	fn as_sd(&self) -> *const c_void {
		self.0.as_ptr().cast_const()
	}
}

/// Constructors.
impl<'a> BorrowedSecurityDescriptor<'a> {
	/// Borrows the given security descriptor.
	///
	/// # Safety
	/// The [safety constraints](AsSecurityDescriptor#safety-constraints) must be upheld.
	#[inline]
	pub unsafe fn from_ref(r: &'a SECURITY_DESCRIPTOR) -> Self {
		unsafe { Self::from_ptr(r.as_ptr().cast()) }
	}

	/// Wraps the given raw pointer to a security descriptor.
	///
	/// # Safety
	/// -	The pointer must be non-null, well-aligned and dereferencable.
	/// -	The [safety constraints](AsSecurityDescriptor#safety-constraints) must be upheld.
	#[inline]
	pub unsafe fn from_ptr(p: *const c_void) -> Self {
		let p = p.cast_mut();
		unsafe {
			debug_assert!(!p.is_null(), "null pointer to security descriptor");
			validate(p);
			Self(NonNull::new(p).unwrap_unchecked(), PhantomData)
		}
	}
}

/// Pointer to a [security descriptor][sd] with reference-like guarantees which allows mutation.
///
/// The pointee is known to be valid (and not mutably aliased) for the duration of the given
/// lifetime, just like with regular Rust references.
///
/// [sd]: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-security_descriptor
#[repr(transparent)]
#[derive(Debug)]
pub struct MutBorrowedSecurityDescriptor<'a>(
	NonNull<SECURITY_DESCRIPTOR>,
	PhantomData<&'a mut SECURITY_DESCRIPTOR>,
);
/// Interior mutability is not provided through [`BorrowedSecurityDescriptor`].
unsafe impl Sync for MutBorrowedSecurityDescriptor<'_> {}
unsafe impl Send for MutBorrowedSecurityDescriptor<'_> {}

unsafe impl AsSecurityDescriptor for MutBorrowedSecurityDescriptor<'_> {
	#[inline(always)]
	fn as_sd(&self) -> *const c_void {
		self.0.as_ptr().cast()
	}
}
unsafe impl AsSecurityDescriptorMut for MutBorrowedSecurityDescriptor<'_> {
	#[inline(always)]
	fn as_sd_mut(&mut self) -> *mut c_void {
		self.as_sd().cast_mut()
	}
}

/// Constructors.
impl<'a> MutBorrowedSecurityDescriptor<'a> {
	/// Borrows the given security descriptor.
	///
	/// # Safety
	/// The [safety constraints](AsSecurityDescriptor#safety-constraints) must be upheld.
	#[inline]
	pub unsafe fn from_ref(r: &'a mut SECURITY_DESCRIPTOR) -> Self {
		unsafe { Self::from_ptr(r) }
	}

	/// Wraps the given raw pointer to a security descriptor.
	///
	/// # Safety
	/// -	The pointer must be non-null, well-aligned and dereferencable.
	/// -	The [safety constraints](AsSecurityDescriptor#safety-constraints) must be upheld.
	#[inline]
	pub unsafe fn from_ptr(p: *mut SECURITY_DESCRIPTOR) -> Self {
		unsafe {
			debug_assert!(!p.is_null(), "null pointer to security descriptor");
			validate(p.cast());
			Self(NonNull::new(p).unwrap_unchecked(), PhantomData)
		}
	}
}
