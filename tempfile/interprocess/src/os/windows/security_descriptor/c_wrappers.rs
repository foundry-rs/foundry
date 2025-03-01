use super::LocalBox;
use crate::{BoolExt, OrErrno, SubUsizeExt};
use std::{ffi::c_void, io, ptr};
use widestring::U16CStr;
use windows_sys::Win32::{
	Foundation::{LocalFree, BOOL, PSID},
	Security::{
		Authorization::{
			ConvertSecurityDescriptorToStringSecurityDescriptorW,
			ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
		},
		FreeSid, GetSecurityDescriptorControl, SetSecurityDescriptorControl, ACL,
		SECURITY_DESCRIPTOR_CONTROL,
	},
};

pub(super) unsafe fn control_and_revision(
	sd: *const c_void,
) -> io::Result<(SECURITY_DESCRIPTOR_CONTROL, u32)> {
	let mut control = SECURITY_DESCRIPTOR_CONTROL::default();
	let mut revision = 0;

	unsafe { GetSecurityDescriptorControl(sd.cast_mut(), &mut control, &mut revision) }
		.true_val_or_errno((control, revision))
}

pub(super) unsafe fn acl(
	sd: *const c_void,
	f: unsafe extern "system" fn(*mut c_void, *mut BOOL, *mut *mut ACL, *mut BOOL) -> BOOL,
) -> io::Result<Option<(*const ACL, bool)>> {
	let mut exists = 0;
	let mut pacl = ptr::null_mut();
	let mut defaulted = 0;
	unsafe { f(sd.cast_mut(), &mut exists, &mut pacl, &mut defaulted) }.true_or_errno(|| {
		if exists != 0 {
			Some((pacl.cast_const(), defaulted != 0))
		} else {
			None
		}
	})
}
pub(super) unsafe fn sid(
	sd: *const c_void,
	f: unsafe extern "system" fn(*mut c_void, *mut PSID, *mut BOOL) -> BOOL,
) -> io::Result<(*const c_void, bool)> {
	let mut psid = ptr::null_mut();
	let mut defaulted = 1;
	unsafe { f(sd.cast_mut(), &mut psid, &mut defaulted) }
		.true_or_errno(|| (psid.cast_const(), defaulted != 0))
}

pub(super) unsafe fn set_acl(
	sd: *const c_void,
	acl: Option<*mut ACL>,
	defaulted: bool,
	f: unsafe extern "system" fn(*mut c_void, BOOL, *const ACL, BOOL) -> BOOL,
) -> io::Result<()> {
	let has_acl = acl.is_some().to_i32();
	// Note that the null ACL is a valid value that does not represent the lack of an ACL. The null
	// pointer this defaults to will be ignored by Windows because has_acl == false.
	let acl = acl.unwrap_or(ptr::null_mut());
	unsafe { f(sd.cast_mut(), has_acl, acl, defaulted.to_i32()) }.true_val_or_errno(())
}
pub(super) unsafe fn set_sid(
	sd: *const c_void,
	sid: *mut c_void,
	defaulted: bool,
	f: unsafe extern "system" fn(*mut c_void, PSID, BOOL) -> BOOL,
) -> io::Result<()> {
	unsafe { f(sd.cast_mut(), sid, defaulted.to_i32()) }.true_val_or_errno(())
}

pub(super) unsafe fn set_control(
	sd: *const c_void,
	mask: SECURITY_DESCRIPTOR_CONTROL,
	value: SECURITY_DESCRIPTOR_CONTROL,
) -> io::Result<()> {
	unsafe { SetSecurityDescriptorControl(sd.cast_mut(), mask, value) }.true_val_or_errno(())
}

pub(super) unsafe fn unset_acl(
	sd: *const c_void,
	f: unsafe extern "system" fn(*mut c_void, BOOL, *const ACL, BOOL) -> BOOL,
) -> io::Result<()> {
	unsafe { set_acl(sd, None, false, f) }
}
pub(super) unsafe fn unset_sid(
	sd: *const c_void,
	f: unsafe extern "system" fn(*mut c_void, PSID, BOOL) -> BOOL,
) -> io::Result<()> {
	unsafe { set_sid(sd, ptr::null_mut(), false, f) }
}

pub(super) unsafe fn free_acl(acl: *mut ACL) -> io::Result<()> {
	unsafe { LocalFree(acl.cast()) }
		.is_null()
		.true_val_or_errno(())
}
pub(super) unsafe fn free_sid(sid: *mut c_void) -> io::Result<()> {
	if sid.is_null() {
		return Ok(());
	}
	if unsafe { FreeSid(sid) }.is_null() {
		Ok(())
	} else {
		Err(io::Error::new(
			io::ErrorKind::Other,
			"failed to deallocate SID",
		))
	}
}

pub(super) unsafe fn serialize(
	sd: *const c_void,
	selector: u32,
) -> io::Result<(LocalBox<u16>, usize)> {
	let mut localboxed_string = ptr::null_mut();
	let mut buflen = 0;
	unsafe {
		ConvertSecurityDescriptorToStringSecurityDescriptorW(
			sd.cast_mut(),
			SDDL_REVISION_1,
			selector,
			&mut localboxed_string,
			&mut buflen,
		)
	}
	.true_val_or_errno(())?;
	Ok((
		unsafe { LocalBox::from_raw(localboxed_string.cast()) },
		buflen.to_usize(),
	))
}

pub(super) fn deserialize(sdsf: &U16CStr) -> io::Result<LocalBox<c_void>> {
	let mut srsd = ptr::null_mut();
	let mut buflen = 0;
	unsafe {
		ConvertStringSecurityDescriptorToSecurityDescriptorW(
			sdsf.as_ptr(),
			SDDL_REVISION_1,
			&mut srsd,
			&mut buflen,
		)
	}
	.true_val_or_errno(())?;
	Ok(unsafe { LocalBox::from_raw(srsd) })
}
