use super::{c_wrappers, unixprelude::*};
use crate::{weaken_buf_init_mut, OrErrno, TryClone};
use std::{
	io::{self, prelude::*, IoSlice, IoSliceMut},
	mem::MaybeUninit,
};

#[allow(clippy::as_conversions)]
fn i2u(i: isize) -> usize {
	i as usize
}

#[repr(transparent)]
pub(super) struct FdOps(pub(super) OwnedFd);
impl FdOps {
	pub(super) fn read_uninit(
		fd: BorrowedFd<'_>,
		buf: &mut [MaybeUninit<u8>],
	) -> io::Result<usize> {
		let length_to_read = buf.len();
		let bytes_read =
			unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), length_to_read) };
		(bytes_read >= 0).true_val_or_errno(i2u(bytes_read))
	}
	pub(super) fn write(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
		let length_to_write = buf.len();
		let bytes_written =
			unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), length_to_write) };
		(bytes_written >= 0).true_val_or_errno(i2u(bytes_written))
	}
}
impl Read for &FdOps {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		FdOps::read_uninit(self.as_fd(), weaken_buf_init_mut(buf))
	}
	fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
		let num_bufs = c_int::try_from(bufs.len()).unwrap_or(c_int::MAX);
		let bytes_read =
			unsafe { libc::readv(self.0.as_raw_fd(), bufs.as_ptr().cast(), num_bufs) };
		(bytes_read >= 0).true_val_or_errno(i2u(bytes_read))
	}
	// FUTURE can_vector
}
impl Write for &FdOps {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		FdOps::write(self.as_fd(), buf)
	}
	fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
		let num_bufs = c_int::try_from(bufs.len()).unwrap_or(c_int::MAX);
		let bytes_written =
			unsafe { libc::writev(self.0.as_raw_fd(), bufs.as_ptr().cast(), num_bufs) };
		(bytes_written >= 0).true_val_or_errno(i2u(bytes_written))
	}
	// FUTURE can_vector
	fn flush(&mut self) -> io::Result<()> {
		unsafe { libc::fsync(self.0.as_raw_fd()) >= 0 }.true_val_or_errno(())
	}
}

impl TryClone for FdOps {
	fn try_clone(&self) -> std::io::Result<Self> {
		let fd = c_wrappers::duplicate_fd(self.0.as_fd())?;
		Ok(Self(fd))
	}
}

multimacro! {
	FdOps,
	forward_handle,
	forward_debug,
	derive_raw,
}
