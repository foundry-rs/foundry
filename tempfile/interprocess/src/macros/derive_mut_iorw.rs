//! Macros that derive `Read` and `Write` (and their Tokio counterparts) on all `T` that satisfy
//! `for<'a> &'a T: Trait` for the corresponding trait.

macro_rules! derive_sync_mut_read {
	($({$($lt:tt)*})? $ty:ty) => {
		impl $(<$($lt)*>)? ::std::io::Read for $ty {
			#[inline(always)]
			fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> {
				(&*self).read(buf)
			}
			#[inline(always)]
			fn read_vectored(
				&mut self,
				bufs: &mut [::std::io::IoSliceMut<'_>],
			) -> ::std::io::Result<usize> { (&*self).read_vectored(bufs) }
			// read_to_end isn't here because this macro isn't supposed to be used on Chain-like
			// adapters
			// FUTURE is_read_vectored
		}
	};
}

macro_rules! derive_sync_mut_write {
	($({$($lt:tt)*})? $ty:ty) => {
		impl $(<$($lt)*>)? ::std::io::Write for $ty {
			#[inline(always)]
			fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
				(&*self).write(buf)
			}
			#[inline(always)]
			fn flush(&mut self) -> ::std::io::Result<()> {
				(&*self).flush()
			}
			#[inline(always)]
			fn write_vectored(
				&mut self,
				bufs: &[::std::io::IoSlice<'_>],
			) -> ::std::io::Result<usize> { (&*self).write_vectored(bufs) }
			// FUTURE is_write_vectored
		}
	};
}

macro_rules! derive_sync_mut_rw {
	($({$($lt:tt)*})? $ty:ty) => {
		forward_sync_read!($({$($lt)*})? $ty);
		forward_sync_write!($({$($lt)*})? $ty);
	};
}

macro_rules! derive_tokio_mut_read {
	($({$($lt:tt)*})? $ty:ty) => {
		const _: () = {
			use ::tokio::io::{AsyncRead, ReadBuf};
			use ::std::{io, pin::Pin, task::{Context, Poll}};
			impl $(<$($lt)*>)? AsyncRead for $ty {
				#[inline(always)]
				fn poll_read(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					buf: &mut ReadBuf<'_>,
				) -> Poll<io::Result<()>> {
					AsyncRead::poll_read(Pin::new(&mut &*self), cx, buf)
				}
			}
		};
	};
}

macro_rules! derive_tokio_mut_write {
	($({$($lt:tt)*})? $ty:ty) => {
		const _: () = {
			use ::tokio::io::AsyncWrite;
			use ::std::{io::{self, IoSlice}, pin::Pin, task::{Context, Poll}};
			impl $(<$($lt)*>)? AsyncWrite for $ty {
				#[inline(always)]
				fn poll_write(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					buf: &[u8],
				) -> Poll<io::Result<usize>> {
					AsyncWrite::poll_write(Pin::new(&mut &*self), cx, buf)
				}
				#[inline(always)]
				fn poll_write_vectored(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					bufs: &[IoSlice<'_>],
				) -> Poll<io::Result<usize>> {
					AsyncWrite::poll_write_vectored(Pin::new(&mut &*self), cx, bufs)
				}
				#[inline(always)]
				fn is_write_vectored(&self) -> bool {
					AsyncWrite::is_write_vectored(self.refwd())
				}
				#[inline(always)]
				fn poll_flush(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
				) -> Poll<io::Result<()>> {
					AsyncWrite::poll_flush(Pin::new(&mut &*self), cx)
				}
				#[inline(always)]
				fn poll_shutdown(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
				) -> Poll<io::Result<()>> {
					AsyncWrite::poll_shutdown(Pin::new(&mut &*self), cx)
				}
			}
		};
	};
}

macro_rules! derive_tokio_mut_rw {
	($({$($lt:tt)*})? $ty:ty) => {
		derive_tokio_mut_read!($({$($lt)*})? $ty);
		derive_tokio_mut_write!($({$($lt)*})? $ty);
	};
}
