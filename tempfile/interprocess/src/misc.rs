#![allow(dead_code)]

#[cfg(unix)]
use std::os::unix::io::RawFd;
use std::{
	io,
	mem::{transmute, MaybeUninit},
	num::Saturating,
	pin::Pin,
	sync::PoisonError,
};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};

/// Utility trait that, if used as a supertrait, prevents other crates from implementing the
/// trait.
pub(crate) trait Sealed {}
pub(crate) trait DebugExpectExt: Sized {
	fn debug_expect(self, msg: &str);
}

pub(crate) static LOCK_POISON: &str = "unexpected lock poison";
pub(crate) fn poison_error<T>(_: PoisonError<T>) -> io::Error {
	io::Error::other(LOCK_POISON)
}

pub(crate) trait OrErrno<T>: Sized {
	fn true_or_errno(self, f: impl FnOnce() -> T) -> io::Result<T>;
	#[inline(always)]
	fn true_val_or_errno(self, value: T) -> io::Result<T> {
		self.true_or_errno(|| value)
	}
	fn false_or_errno(self, f: impl FnOnce() -> T) -> io::Result<T>;
	#[inline(always)]
	fn false_val_or_errno(self, value: T) -> io::Result<T> {
		self.true_or_errno(|| value)
	}
}
impl<B: ToBool, T> OrErrno<T> for B {
	#[inline]
	fn true_or_errno(self, f: impl FnOnce() -> T) -> io::Result<T> {
		if self.to_bool() {
			Ok(f())
		} else {
			Err(io::Error::last_os_error())
		}
	}
	fn false_or_errno(self, f: impl FnOnce() -> T) -> io::Result<T> {
		if !self.to_bool() {
			Ok(f())
		} else {
			Err(io::Error::last_os_error())
		}
	}
}

#[cfg(unix)]
pub(crate) trait FdOrErrno: Sized {
	fn fd_or_errno(self) -> io::Result<Self>;
}
#[cfg(unix)]
impl FdOrErrno for RawFd {
	#[inline]
	fn fd_or_errno(self) -> io::Result<Self> {
		(self != -1).true_val_or_errno(self)
	}
}

#[cfg(windows)]
pub(crate) trait HandleOrErrno: Sized {
	fn handle_or_errno(self) -> io::Result<Self>;
}
#[cfg(windows)]
impl HandleOrErrno for HANDLE {
	#[inline]
	fn handle_or_errno(self) -> io::Result<Self> {
		(self != INVALID_HANDLE_VALUE).true_val_or_errno(self)
	}
}

pub(crate) trait ToBool {
	fn to_bool(self) -> bool;
}
impl ToBool for bool {
	#[inline(always)]
	fn to_bool(self) -> bool {
		self
	}
}
impl ToBool for i32 {
	#[inline(always)]
	fn to_bool(self) -> bool {
		self != 0
	}
}

pub(crate) trait BoolExt {
	fn to_i32(self) -> i32;
	fn to_usize(self) -> usize;
}
impl BoolExt for bool {
	#[inline(always)] #[rustfmt::skip] // oh come on now
	fn to_i32(self) -> i32 {
		if self { 1 } else { 0 }
	}
	#[inline(always)] #[rustfmt::skip]
	fn to_usize(self) -> usize {
		if self { 1 } else { 0 }
	}
}

pub(crate) trait AsPtr {
	#[inline(always)]
	fn as_ptr(&self) -> *const Self {
		self
	}
}
impl<T: ?Sized> AsPtr for T {}

pub(crate) trait AsMutPtr {
	#[inline(always)]
	fn as_mut_ptr(&mut self) -> *mut Self {
		self
	}
}
impl<T: ?Sized> AsMutPtr for T {}

impl<T, E: std::fmt::Debug> DebugExpectExt for Result<T, E> {
	#[inline]
	#[track_caller]
	fn debug_expect(self, msg: &str) {
		if cfg!(debug_assertions) {
			self.expect(msg);
		}
	}
}
impl<T> DebugExpectExt for Option<T> {
	#[inline]
	#[track_caller]
	fn debug_expect(self, msg: &str) {
		if cfg!(debug_assertions) {
			self.expect(msg);
		}
	}
}

pub(crate) trait NumExt: Sized {
	#[inline]
	fn saturate(self) -> Saturating<Self> {
		Saturating(self)
	}
}
impl<T> NumExt for T {}

pub(crate) trait SubUsizeExt: TryInto<usize> + Sized {
	fn to_usize(self) -> usize;
}
pub(crate) trait SubIsizeExt: TryInto<usize> + Sized {
	fn to_isize(self) -> isize;
}
macro_rules! impl_subsize {
	($src:ident to usize) => {
		impl SubUsizeExt for $src {
			#[inline(always)]
			#[allow(clippy::as_conversions)]
			fn to_usize(self) -> usize {
				self as usize
			}
		}
	};
	($src:ident to isize) => {
		impl SubIsizeExt for $src {
			#[inline(always)]
			#[allow(clippy::as_conversions)]
			fn to_isize(self) -> isize {
				self as isize
			}
		}
	};
	($($src:ident to $dst:ident)+) => {$(
		impl_subsize!($src to $dst);
	)+};
}
// See platform_check.rs.
impl_subsize! {
	u8	to usize
	u16	to usize
	u32	to usize
	i8	to isize
	i16	to isize
	i32	to isize
	u8	to isize
	u16	to isize
}

// TODO(2.3.0) find a more elegant way
pub(crate) trait RawOsErrorExt {
	fn eeq(self, other: u32) -> bool;
}
impl RawOsErrorExt for Option<i32> {
	#[inline(always)]
	#[allow(clippy::as_conversions)]
	fn eeq(self, other: u32) -> bool {
		match self {
			Some(n) => n as u32 == other,
			None => false,
		}
	}
}

#[inline(always)]
pub(crate) fn weaken_buf_init<T>(r: &[T]) -> &[MaybeUninit<T>] {
	unsafe {
		// SAFETY: same slice, weaker refinement
		transmute(r)
	}
}
#[inline(always)]
pub(crate) fn weaken_buf_init_mut<T>(r: &mut [T]) -> &mut [MaybeUninit<T>] {
	unsafe {
		// SAFETY: same here
		transmute(r)
	}
}

#[inline(always)]
pub(crate) unsafe fn assume_slice_init<T>(r: &[MaybeUninit<T>]) -> &[T] {
	unsafe {
		// SAFETY: same slice, stronger refinement
		transmute(r)
	}
}

pub(crate) trait UnpinExt: Unpin {
	#[inline]
	fn pin(&mut self) -> Pin<&mut Self> {
		Pin::new(self)
	}
}
impl<T: Unpin + ?Sized> UnpinExt for T {}
