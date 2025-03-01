use super::{c_wrappers, AsSecurityDescriptor, AsSecurityDescriptorMut, SecurityDescriptor};
use std::{ffi::c_void, io};
use widestring::U16CStr;
use windows_sys::Win32::Security::{
	GetSecurityDescriptorDacl, GetSecurityDescriptorGroup, GetSecurityDescriptorOwner,
	GetSecurityDescriptorSacl, SetSecurityDescriptorDacl, SetSecurityDescriptorGroup,
	SetSecurityDescriptorOwner, SetSecurityDescriptorSacl, ACL, SECURITY_ATTRIBUTES,
	SECURITY_DESCRIPTOR_CONTROL,
};

#[rustfmt::skip] macro_rules! indirect_methods {
	(@ $doc:literal $nm:ident acl $wfn:ident) => {
		#[doc = concat!("\
Returns a raw pointer to the contained ", $doc, " access control list, as well as the value of its
corresponding \"defaulted\" boolean flag used to denote automatically generated ACLs.")]
		#[inline] fn $nm(&self) -> io::Result<Option<(*const ACL, bool)>> {
			unsafe { c_wrappers::acl(self.as_sd(), $wfn) }
		}
	};
	(@ $doc:literal $nm:ident sid $wfn:ident) => {
		#[doc = concat!("\
Returns a raw pointer to the contained ", $doc, " SID, as well as the value of its corresponding
\"defaulted\" boolean flag used to denote SIDs that were chosen automatically.")]
		#[inline] fn $nm(&self) -> io::Result<(*const c_void, bool)> {
			unsafe { c_wrappers::sid(self.as_sd(), $wfn) }
		}
	};

	(@ $doc:literal $nm:ident unset_acl $wfn:ident) => {
		#[doc = concat!("\
Marks the security descriptor as not containing the ", $doc, " access control list. If one was
previously present, its memory is not reclaimed.")]
		#[inline] fn $nm(&mut self) -> io::Result<()> {
			unsafe { c_wrappers::unset_acl(self.as_sd(), $wfn) }
		}
	};
	(@ $doc:literal $nm:ident unset_sid $wfn:ident) => {
		#[doc = concat!("\
Marks the security descriptor as not containing the ", $doc, " SID. If one was previously present,
its memory is not reclaimed.")]
		#[inline] fn $nm(&mut self) -> io::Result<()> {
			unsafe { c_wrappers::unset_sid(self.as_sd(), $wfn) }
		}
	};

	(@ $doc:literal $nm:ident set_acl $wfn:ident) => {
		#[doc = concat!("\
Sets the ", $doc, " access control list to the specified value, assuming ownership on the
[local heap][lh].

If `defaulted` is `true`, the ", $doc, " access control list is marked as having been produced by
some default mechanism. This is only used for internal program logic and is not checked by Windows.

Note that, for DACLs, a null ACL (`ptr::null_mut()`) is not the same as an unset/absent ACL: it
actually provides ***full access*** for every security principal.

# Safety
The pointer, *if not null*:
- must point to a well-initialized ACL;
- must not be owned elsewhere;
- must be valid for deallocation with `LocalFree()`.

[lh]: https://learn.microsoft.com/en-us/windows/win32/memory/global-and-local-functions
")]
		#[doc(hidden)]
		#[inline] unsafe fn $nm(&mut self, acl: *mut ACL, defaulted: bool) -> io::Result<()> {
			unsafe { c_wrappers::set_acl(self.as_sd(), Some(acl), defaulted, $wfn) }
		}
	};

	(@ $doc:literal $nm:ident set_sid $wfn:ident) => {
		#[doc = concat!("\
Sets the ", $doc, " SID to the specified value, assuming ownership on the [local heap][lh].

A null pointer is not accepted as a sentinel for the lack of a ", $doc, " SID. See the
corresponding unsetter in [`AsSecurityDescriptorMutExt`].

If `defaulted` is `true`, the ", $doc, " SID is marked as having been produced by some default
mechanism. This is only used for internal program logic and is not checked by Windows.

# Safety
The pointer:
- must point to a well-initialized SID;
- must not be owned elsewhere;
- must be valid for deallocation with `LocalFree()`.

[lh]: https://learn.microsoft.com/en-us/windows/win32/memory/global-and-local-functions
")]
		#[inline] unsafe fn $nm(&mut self, sid: *mut c_void, defaulted: bool) -> io::Result<()> {
			if sid.is_null() {
				return Err(
					io::Error::new(
						io::ErrorKind::InvalidInput,
						concat!(
"set_", $doc, " is the wrong function to use for unsetting the SID – use unset_", $doc, " instead"
						),
					)
				)
			}
			unsafe { c_wrappers::set_sid(self.as_sd(), sid, defaulted, $wfn) }
		}
	};

	(@ $doc:literal $nm:ident remove_acl [$unset:ident] $get:ident) => {
		#[doc = concat!("\
Deallocates the ", $doc, " access control list and marks it as not present in the security
descriptor. The security descriptor remains valid after this operation. If the ", $doc, " access
control list is not present, succeeds with no effect.

# Errors
Same as [`.free_contents()`](Self::free_contents).")]
		#[inline] fn $nm(&mut self) -> io::Result<()> {
			let val = self.$get()?;
			self.$unset()?;
			if let Some((val, _)) = val {
				unsafe { c_wrappers::free_acl(val.cast_mut())? };
			}
			Ok(())
		}
	};

	(@ $doc:literal $nm:ident remove_sid [$unset:ident] $get:ident) => {
		#[doc = concat!("\
Deallocates the ", $doc, " SID and marks it as not present in the security descriptor. The security
descriptor remains valid after this operation. If the ", $doc, " SID is not present, succeeds with
no effect.

# Errors
Same as [`.free_contents()`](Self::free_contents).")]
		#[inline] fn $nm(&mut self) -> io::Result<()> {
			let val = self.$get()?;
			self.$unset()?;
			unsafe { c_wrappers::free_sid(val.0.cast_mut())? };
			Ok(())
		}
	};

	($($doc:literal $nm:ident $cat:ident $([$unset:ident])? $wfn:ident)+) => {$(
		indirect_methods!(@ $doc $nm $cat $([$unset])? $wfn);
	)+};
}

/// Methods derived from the interface of [`AsSecurityDescriptor`].
pub trait AsSecurityDescriptorExt: AsSecurityDescriptor {
	indirect_methods! {
		"DACL"	dacl	acl	GetSecurityDescriptorDacl
		"SACL"	sacl	acl	GetSecurityDescriptorSacl
		"owner"	owner	sid	GetSecurityDescriptorOwner
		"group"	group	sid	GetSecurityDescriptorGroup
	}

	/// Returns the [control bits][cb] of the security descriptor and its revision number.
	///
	/// [cb]: https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-control
	fn control_and_revision(&self) -> io::Result<(SECURITY_DESCRIPTOR_CONTROL, u32)> {
		unsafe { c_wrappers::control_and_revision(self.as_sd()) }
	}

	/// Clones the security descriptor, producing a new [owned one](SecurityDescriptor).
	///
	/// This is aliased to [`TryClone`](crate::TryClone) on [`SecurityDescriptor`] itself.
	fn to_owned_sd(&self) -> io::Result<SecurityDescriptor> {
		unsafe { super::clone(self.as_sd()) }
	}

	/// Sets the security descriptor pointer of the given `SECURITY_ATTRIBUTES` structure to the
	/// security descriptor borrow of `self`.
	fn write_to_security_attributes(&self, attributes: &mut SECURITY_ATTRIBUTES) {
		attributes.lpSecurityDescriptor = self.as_sd().cast_mut();
	}

	/// Serializes the security descriptor into [the security descriptor string format][sdsf] for
	/// debug printing, textual storage and safe interchange.
	///
	/// The [`selector` bitflags][secinfo] determine the information that is serialized. This is
	/// primarily useful for deserialization, since not all of the information that can be stored
	/// in the string representation can be set without special permissions.
	///
	/// The result is returned by passing it by-reference to the specified closure. This is because
	/// the slice is written to a `LocalAlloc()`-allocated buffer. Because there isn't a `LocalBox`
	/// in the public API of Interprocess for the sake of simplicity, returning the buffer raw would
	/// be prone to memory leaks.
	///
	/// [sdsf]: https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format
	/// [secinfo]: https://learn.microsoft.com/en-us/windows/win32/secauthz/security-information
	fn serialize<R>(&self, selector: u32, f: impl FnOnce(&U16CStr) -> R) -> io::Result<R> {
		let (s, bsz) = unsafe { c_wrappers::serialize(self.as_sd(), selector)? };
		let slice =
			unsafe { U16CStr::from_ptr_truncate(s.as_ptr(), bsz) }.map_err(io::Error::other)?;
		Ok(f(slice))
	}
}
impl<T: AsSecurityDescriptor + ?Sized> AsSecurityDescriptorExt for T {}

/// Methods derived from the interface of [`AsSecurityDescriptorMut`].
pub trait AsSecurityDescriptorMutExt: AsSecurityDescriptorMut {
	indirect_methods! {
		"DACL"	unset_dacl		unset_acl	SetSecurityDescriptorDacl
		"SACL"	unset_sacl		unset_acl	SetSecurityDescriptorSacl
		"owner"	unset_owner		unset_sid	SetSecurityDescriptorOwner
		"group"	unset_group		unset_sid	SetSecurityDescriptorGroup
		"DACL"	set_dacl		set_acl		SetSecurityDescriptorDacl
		"SACL"	set_sacl		set_acl		SetSecurityDescriptorSacl
		"owner"	set_owner		set_sid		SetSecurityDescriptorOwner
		"group"	set_group		set_sid		SetSecurityDescriptorGroup
		"DACL"	remove_dacl		remove_acl	[unset_dacl]	dacl
		"SACL"	remove_sacl		remove_acl	[unset_sacl]	sacl
		"owner"	remove_owner	remove_sid	[unset_owner]	owner
		"group"	remove_group	remove_sid	[unset_group]	group
	}

	/// Modifies the [control bits][cb] of the security descriptor. The bits set to 1 in the mask
	/// are overridden with values from the corresponding places of the `value` argument.
	///
	/// [cb]: https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-control
	fn set_control(
		&mut self,
		mask: SECURITY_DESCRIPTOR_CONTROL,
		value: SECURITY_DESCRIPTOR_CONTROL,
	) -> io::Result<()> {
		unsafe { c_wrappers::set_control(self.as_sd(), mask, value) }
	}

	/// Deallocates the DACL and the SACL, marking them as not present in the security descriptor.
	/// The security descriptor remains valid after this operation.
	///
	/// # Errors
	/// Same as [`.free_contents()`](Self::free_contents).
	fn remove_acls(&mut self) -> io::Result<()> {
		self.remove_dacl()?;
		self.remove_sacl()
	}

	/// Deallocates the owner SID and the group SID, marking them as not present in the security
	/// descriptor. The security descriptor remains valid after this operation.
	///
	/// # Errors
	/// Same as [`.free_contents()`](Self::free_contents).
	fn remove_sids(&mut self) -> io::Result<()> {
		self.remove_owner()?;
		self.remove_group()
	}

	/// Frees the ACLs and SIDs pointed to by the security descriptor. The security descriptor
	/// remains valid after this operation.
	///
	/// # Errors
	/// If an error was returned, memory has not been reclaimed. This indicates a likely bug in the
	/// program. Since the error is non-critical, it might make sense to log it instead of panicking
	/// right away.
	fn free_contents(&mut self) -> io::Result<()> {
		self.remove_acls()?;
		self.remove_sids()
	}
}
impl<T: AsSecurityDescriptorMut + ?Sized> AsSecurityDescriptorMutExt for T {}
