use super::*;
use crate::{AsMutPtr, OrErrno};
use std::{
	marker::PhantomData,
	mem::{size_of, size_of_val, zeroed, ManuallyDrop},
	ptr::{self, NonNull},
};
use windows_sys::Win32::{
	Foundation::LocalFree,
	Security::{
		AclRevisionInformation, AclSizeInformation, AddAce, CopySid, GetAce, GetAclInformation,
		GetSidLengthRequired, GetSidSubAuthorityCount, InitializeAcl, IsValidSid, ACE_HEADER,
		ACL, ACL_INFORMATION_CLASS, ACL_REVISION_INFORMATION, ACL_SIZE_INFORMATION,
		SECURITY_DESCRIPTOR_CONTROL, SE_DACL_PROTECTED, SE_SACL_PROTECTED,
	},
	System::{
		Memory::{LocalAlloc, LMEM_FIXED},
		SystemServices::MAXDWORD,
	},
};

pub(super) unsafe fn clone(sd: *const c_void) -> io::Result<SecurityDescriptor> {
	// Those are the only ones that can be set with SetSecurityDescriptorControl().
	const CONTROL_MASK: SECURITY_DESCRIPTOR_CONTROL = SE_DACL_PROTECTED | SE_SACL_PROTECTED;

	let mut new_sd = SecurityDescriptor::new()?;
	let old_sd = unsafe {
		// SAFETY: as per contract
		BorrowedSecurityDescriptor::from_ptr(sd)
	};

	let clnsid = |(sid, dfl)| {
		io::Result::<(Option<LocalBox<c_void>>, bool)>::Ok((unsafe { clone_sid(sid)? }, dfl))
	};
	let dacl = old_sd.dacl()?;
	let sacl = old_sd.sacl()?;
	let owner = clnsid(old_sd.owner()?)?;
	let group = clnsid(old_sd.group()?)?;

	if let Some((acl, dfl)) = dacl {
		if acl.is_null() {
			unsafe { new_sd.set_dacl(ptr::null_mut(), dfl)? };
		} else {
			let mut acl = ManuallyDrop::new(unsafe { clone_acl(acl)? });
			unsafe { new_sd.set_dacl((*acl).as_mut_ptr(), dfl)? };
		}
	}
	if let Some((acl, dfl)) = sacl {
		if acl.is_null() {
			unsafe { new_sd.set_sacl(ptr::null_mut(), dfl)? };
		} else {
			let mut acl = ManuallyDrop::new(unsafe { clone_acl(acl)? });
			unsafe { new_sd.set_sacl((*acl).as_mut_ptr(), dfl)? };
		}
	}

	let assid = |sid: &mut LocalBox<c_void>| sid.as_mut_ptr();
	let (mut owner, odfl) = (ManuallyDrop::new(owner.0), owner.1);
	if let Some(owner) = owner.as_mut().map(assid) {
		unsafe { new_sd.set_owner(owner, odfl)? };
	}

	let (mut group, gdfl) = (ManuallyDrop::new(group.0), group.1);
	if let Some(group) = group.as_mut().map(assid) {
		unsafe { new_sd.set_owner(group, gdfl)? };
	}

	let control = old_sd.control_and_revision()?.0;
	new_sd.set_control(CONTROL_MASK, control & CONTROL_MASK)?;

	Ok(new_sd)
}

pub(crate) struct LocalBox<T>(NonNull<T>, PhantomData<T>);
impl<T> LocalBox<T> {
	#[allow(clippy::unwrap_used, clippy::unwrap_in_result)]
	pub fn allocate(sz: u32) -> io::Result<Self> {
		// Unwrap note: this code isn't supposed to compile on Win16.
		let allocation = unsafe { LocalAlloc(LMEM_FIXED, sz.try_into().unwrap()) };
		(allocation.is_null()).false_or_errno(|| unsafe {
			Self(NonNull::new_unchecked(allocation.cast()), PhantomData)
		})
	}
	#[inline]
	pub fn as_ptr(&self) -> *const T {
		self.0.as_ptr().cast_const()
	}
	#[inline]
	pub fn as_mut_ptr(&mut self) -> *mut T {
		self.0.as_ptr()
	}
	#[inline]
	pub unsafe fn from_raw(raw: *mut T) -> Self {
		unsafe { Self(NonNull::new_unchecked(raw), PhantomData) }
	}
}
impl<T> Drop for LocalBox<T> {
	fn drop(&mut self) {
		unsafe { LocalFree(self.as_mut_ptr().cast()) }
			.is_null()
			.true_val_or_errno(())
			.expect("LocalFree() failed")
	}
}

/// Wraps `GetAclInformation()`.
///
/// # Safety
/// -	`zeroed::<T>()` must be POD, i.e. all bit patterns of `T`'s size must constitute
/// 	well-initialized instances of `T`.
/// -	`T` must be the correct size for `information_class`.
unsafe fn get_acl_info<T>(
	acl: *const ACL,
	information_class: ACL_INFORMATION_CLASS,
) -> io::Result<T> {
	let mut info = unsafe { zeroed::<T>() };
	unsafe {
		GetAclInformation(
			acl.cast_mut(),
			info.as_mut_ptr().cast(),
			size_of_val(&info).try_into().unwrap(),
			information_class,
		)
		.true_val_or_errno(info)
	}
}

#[allow(clippy::unwrap_used, clippy::as_conversions)]
fn create_acl(sz: u32, rev: u32) -> io::Result<LocalBox<ACL>> {
	const ALIGN: u32 = size_of::<u32>() as u32; // 100₂
	const ALIGN_MASK: u32 = ALIGN - 1; // 011₂
	let sz = if sz & ALIGN_MASK != 0 {
		// It's not possible for the allocated size of an ACL to exceed DWORD::MAX, and it's also
		// not possible for the upward-aligned bytes-in-use figure to exceed the allocated size.
		sz.checked_add(1).unwrap()
	} else {
		sz
	};

	let mut acl = LocalBox::allocate(sz)?;
	unsafe { InitializeAcl(acl.as_mut_ptr(), sz, rev) }.true_val_or_errno(acl)
}

unsafe fn clone_acl(acl: *const ACL) -> io::Result<LocalBox<ACL>> {
	let (sz_info, rev) = unsafe {
		let sz_info = get_acl_info::<ACL_SIZE_INFORMATION>(acl, AclSizeInformation)?;
		let rev =
			get_acl_info::<ACL_REVISION_INFORMATION>(acl, AclRevisionInformation)?.AclRevision;
		(sz_info, rev)
	};
	let mut new_acl = create_acl(sz_info.AclBytesInUse, rev)?;

	unsafe {
		let mut ace = ptr::null_mut();
		for i in 0..sz_info.AceCount {
			GetAce(acl, i, &mut ace).true_val_or_errno(())?;
			AddAce(
				new_acl.as_mut_ptr(),
				rev,
				MAXDWORD,
				ace.cast_const(),
				(*ace.cast_const().cast::<ACE_HEADER>()).AceSize.into(),
			)
			.true_val_or_errno(())?;
		}
	}
	Ok(new_acl)
}

unsafe fn clone_sid(sid: *const c_void) -> io::Result<Option<LocalBox<c_void>>> {
	if sid.is_null() {
		// Unlike with ACLs, a null PSID is a sentinel for the lack of a SID. By analogy with
		// `None.clone() == None`, we return the same value.
		return Ok(None);
	}
	let sid = sid.cast_mut();
	unsafe { IsValidSid(sid) }.true_val_or_errno(())?;

	let num_subauths = unsafe { *GetSidSubAuthorityCount(sid) };
	let sz = unsafe { GetSidLengthRequired(num_subauths) };

	let mut new_sid = LocalBox::allocate(sz)?;
	unsafe { CopySid(sz, new_sid.as_mut_ptr(), sid) }.true_val_or_errno(Some(new_sid))
}
