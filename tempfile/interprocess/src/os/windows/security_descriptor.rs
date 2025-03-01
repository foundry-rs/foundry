//! Lightweight safety layer for working with security descriptors.
//!
//! Constructing Windows security descriptors can get complicated, and is non-trivial on a
//! conceptual level, making it largely outside the scope of Interprocess. To help you roll your own
//! security descriptor handling or get help from a different crate, this module provides security
//! descriptor primitives that have an emphasis on composability. The most complicated facility is
//! perhaps the implementation of [`TryClone`](crate::TryClone) for [`SecurityDescriptor`], and even
//! that is mostly boilerplate written in accordance to official Windows documentation.

mod as_security_descriptor;
mod borrowed;
mod c_wrappers;
mod ext;
mod owned;
mod try_clone;

#[allow(unused_imports)] // this is literally a false positive
pub(crate) use try_clone::LocalBox;

use crate::BoolExt;

pub use {as_security_descriptor::*, borrowed::*, ext::*, owned::*};

use try_clone::clone;

use std::{ffi::c_void, io};
use windows_sys::Win32::Security::{IsValidSecurityDescriptor, SECURITY_ATTRIBUTES};

unsafe fn validate(ptr: *mut c_void) {
	unsafe {
		debug_assert!(
			IsValidSecurityDescriptor(ptr) == 1,
			"invalid security descriptor: {}",
			io::Error::last_os_error(),
		);
	}
}

pub(super) fn create_security_attributes(
	sd: Option<BorrowedSecurityDescriptor<'_>>,
	inheritable: bool,
) -> SECURITY_ATTRIBUTES {
	let mut attrs = unsafe { std::mem::zeroed::<SECURITY_ATTRIBUTES>() };
	if let Some(sd) = sd {
		sd.write_to_security_attributes(&mut attrs);
	}
	attrs.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>()
		.try_into()
		.unwrap();
	attrs.bInheritHandle = inheritable.to_i32();
	attrs
}
