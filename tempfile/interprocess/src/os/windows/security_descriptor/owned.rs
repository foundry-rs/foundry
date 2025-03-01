use super::*;
use crate::{AsMutPtr, AsPtr, DebugExpectExt, OrErrno, TryClone};
use std::{
	fmt::{self, Debug, Formatter},
	mem::MaybeUninit,
};
use widestring::U16CStr;
use windows_sys::Win32::{
	Security::{InitializeSecurityDescriptor, SECURITY_DESCRIPTOR, SE_SELF_RELATIVE},
	System::SystemServices::SECURITY_DESCRIPTOR_REVISION,
};

/// [Security descriptor][sd] in [absolute format][abs], stored by-value with ownership of all
/// contained ACLs and SIDs on the [local heap][lh].
///
/// Consult Microsoft Learn for [an example][ex] of how to correctly create a security descriptor.
///
/// [sd]: https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-security_descriptor
/// [abs]: https://learn.microsoft.com/en-us/windows/win32/secauthz/absolute-and-self-relative-security-descriptors
/// [ex]: https://learn.microsoft.com/en-us/windows/win32/secauthz/creating-a-security-descriptor-for-a-new-object-in-c--
/// [lh]: https://learn.microsoft.com/en-us/windows/win32/memory/global-and-local-functions
#[repr(C)]
pub struct SecurityDescriptor(SECURITY_DESCRIPTOR);
/// Interior mutability is not provided through [`SecurityDescriptor`].
unsafe impl Sync for SecurityDescriptor {}
unsafe impl Send for SecurityDescriptor {}

unsafe impl AsSecurityDescriptor for SecurityDescriptor {
	#[inline(always)]
	fn as_sd(&self) -> *const c_void {
		self.as_ptr().cast()
	}
}
unsafe impl AsSecurityDescriptorMut for SecurityDescriptor {
	#[inline(always)]
	fn as_sd_mut(&mut self) -> *mut c_void {
		self.as_sd().cast_mut()
	}
}

/// Constructors.
impl SecurityDescriptor {
	/// Creates a [default](Default) security descriptor.
	pub fn new() -> io::Result<Self> {
		let mut sd = MaybeUninit::<SECURITY_DESCRIPTOR>::uninit();
		unsafe {
			InitializeSecurityDescriptor(sd.as_mut_ptr().cast(), SECURITY_DESCRIPTOR_REVISION)
				.true_or_errno(||
					// SAFETY: InitializeSecurityDescriptor() creates a well-initialized absolute SD
					Self::from_owned(sd.assume_init()))
		}
	}

	/// Deserializes a security descriptor from the [security descriptor string format][sdsf].
	///
	/// [sdsf]: https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format
	pub fn deserialize(sdsf: &U16CStr) -> io::Result<Self> {
		let srsd = c_wrappers::deserialize(sdsf)?;
		unsafe { BorrowedSecurityDescriptor::from_ptr(srsd.as_ptr()) }.to_owned_sd()
	}

	/// Wraps the given security descriptor, assuming ownership.
	///
	/// # Safety
	/// -	The security descriptor must be [absolute][abs], not self-relative.
	/// -	The security descriptor must *own* all of its contents.
	/// -	The [safety constraints](AsSecurityDescriptor#safety-constraints) must be upheld.
	///
	/// [abs]: https://learn.microsoft.com/en-us/windows/win32/secauthz/absolute-and-self-relative-security-descriptors
	#[inline(always)]
	pub unsafe fn from_owned(mut sd: SECURITY_DESCRIPTOR) -> Self {
		debug_assert!(
			unsafe {
				c_wrappers::control_and_revision(sd.as_ptr().cast())
					.expect("failed to verify that security descriptor is not self-relative")
					.0 & SE_SELF_RELATIVE
					== 0
			},
			"self-relative security descriptor not allowed here"
		);
		unsafe {
			validate(sd.as_mut_ptr().cast());
		}
		Self(sd)
	}
}

impl Default for SecurityDescriptor {
	fn default() -> Self {
		Self::new().expect("could not default-initialize security descriptor")
	}
}

impl TryClone for SecurityDescriptor {
	#[inline]
	fn try_clone(&self) -> io::Result<Self> {
		unsafe { super::clone(self.as_sd()) }
	}
}

/// Borrowing.
impl SecurityDescriptor {
	/// Borrows immutably. The returned type is also a safe wrapper around security descriptors.
	#[inline(always)]
	pub fn borrow(&self) -> BorrowedSecurityDescriptor<'_> {
		unsafe { BorrowedSecurityDescriptor::from_ptr(self.as_ptr().cast()) }
	}
	/// Borrows mutably. The returned type is also a safe wrapper around security descriptors.
	#[inline(always)]
	pub fn borrow_mut(&mut self) -> MutBorrowedSecurityDescriptor<'_> {
		unsafe { MutBorrowedSecurityDescriptor::from_ptr(self.as_mut_ptr().cast()) }
	}
}

impl Debug for SecurityDescriptor {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_tuple("SecurityDescriptor")
			.field(&(self.as_ptr()))
			.finish()
	}
}

impl Drop for SecurityDescriptor {
	fn drop(&mut self) {
		self.free_contents()
			.debug_expect("failed to free memory owned by security descriptor");
	}
}
