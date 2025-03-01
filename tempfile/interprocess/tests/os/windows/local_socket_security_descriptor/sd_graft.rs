#![cfg(not(ci))]

use crate::{
	local_socket::{prelude::*, Listener, ListenerOptions, Stream},
	os::windows::{
		local_socket::ListenerOptionsExt,
		security_descriptor::{
			AsSecurityDescriptorExt, BorrowedSecurityDescriptor, LocalBox, SecurityDescriptor,
		},
		AsRawHandleExt as _,
	},
	tests::util::*,
	OrErrno, SubUsizeExt, TryClone,
};
use std::{
	ffi::OsString, fs::File, io, mem::MaybeUninit, os::windows::prelude::*, ptr, sync::Arc,
};
use widestring::{U16CStr, U16Str};
use windows_sys::Win32::{
	Foundation::{MAX_PATH, STATUS_SUCCESS},
	Security::{
		Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT, SE_OBJECT_TYPE},
		DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
	},
	System::LibraryLoader::GetModuleFileNameW,
};

const SECINFO: u32 =
	DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION;

fn get_sd(handle: BorrowedHandle<'_>, ot: SE_OBJECT_TYPE) -> TestResult<SecurityDescriptor> {
	let mut sdptr = ptr::null_mut();
	let errno = unsafe {
		GetSecurityInfo(
			handle.as_int_handle(),
			ot,
			SECINFO,
			ptr::null_mut(),
			ptr::null_mut(),
			ptr::null_mut(),
			ptr::null_mut(),
			&mut sdptr,
		)
	};
	let errno = {
		#[allow(clippy::as_conversions)]
		{
			errno as i32
		}
	};
	(errno == STATUS_SUCCESS)
		.then_some(())
		.ok_or_else(|| io::Error::from_raw_os_error(errno))
		.opname("GetSecurityInfo")?;

	let sdbx = unsafe { LocalBox::from_raw(sdptr) };
	unsafe { BorrowedSecurityDescriptor::from_ptr(sdbx.as_ptr()) }
		.to_owned_sd()
		.opname("security descriptor clone")
}

fn count_opening_parentheses(s: &U16Str) -> u32 {
	let mut cpa = 0;
	for c in s.as_slice().iter().copied() {
		if c == b'('.into() {
			cpa += 1;
		}
	}
	cpa
}

fn ensure_equal_number_of_opening_parentheses(a: &U16Str, b: &U16Str) -> TestResult {
	ensure_eq!(count_opening_parentheses(a), count_opening_parentheses(b));
	Ok(())
}

fn ensure_equal_non_acl_part(a: &U16CStr, b: &U16CStr) -> TestResult<usize> {
	let mut idx = 0;
	for (i, (ca, cb)) in a
		.as_slice()
		.iter()
		.copied()
		.zip(b.as_slice().iter().copied())
		.enumerate()
	{
		idx = i;
		if ca == b'D'.into() {
			break;
		}
		ensure_eq!(ca, cb);
	}
	Ok(idx)
}

fn get_self_exe(obuf: &mut [MaybeUninit<u16>]) -> io::Result<&U16CStr> {
	if obuf.is_empty() {
		return Ok(Default::default());
	}
	let base = obuf.as_mut_ptr().cast();
	let cap = obuf.len().try_into().unwrap_or(u32::MAX);
	unsafe { GetModuleFileNameW(0, base, cap) != 0 }
		.true_val_or_errno(())
		.and_then(|()| unsafe {
			U16CStr::from_ptr_truncate(base.cast_const(), cap.to_usize())
				.map_err(io::Error::other)
		})
}

#[allow(clippy::as_conversions)]
pub(super) fn test_main() -> TestResult {
	let sd = {
		let mut pathbuf = [MaybeUninit::uninit(); MAX_PATH as _];
		let path: OsString = get_self_exe(&mut pathbuf)
			.opname("query of path to own executable")?
			.into();
		let file = File::open(path).opname("own executable open")?;
		get_sd(file.as_handle(), SE_KERNEL_OBJECT)
			.opname("query of own executable's security descriptor")?
	};
	sd.serialize(SECINFO, |s| {
		eprintln!("SDDL of the running executable: {}", s.display());
	})
	.opname("serialize")?;

	let (name, listener) =
		listen_and_pick_name(&mut namegen_local_socket(make_id!(), false), |nm| {
			ListenerOptions::new()
				.name(nm.borrow())
				.security_descriptor(sd.try_clone()?)
				.create_sync()
		})?;
	let _ = Stream::connect(Arc::try_unwrap(name).unwrap()).opname("client connect")?;

	let listener_handle = match listener {
		Listener::NamedPipe(l) => OwnedHandle::from(l),
	};
	let listener_sd =
		get_sd(listener_handle.as_handle(), SE_KERNEL_OBJECT).opname("get listener SD")?;

	sd.serialize(SECINFO, |old_s| {
		listener_sd.serialize(SECINFO, |new_s| {
			eprintln!("SDDL of the local socket listener: {}", new_s.display());
			let start = ensure_equal_non_acl_part(old_s, new_s)?;
			ensure_equal_number_of_opening_parentheses(&old_s[start..], &new_s[start..])?;
			TestResult::Ok(())
		})
	})
	.opname("serialize and check")???;

	Ok(())
}
