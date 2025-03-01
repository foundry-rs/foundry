//! Forwarding of `Read` and `Write` (and their Tokio counterparts) for newtypes. Allows attributes
//! on the impl block and on every individual method – only one attribute per type, comma-separated.

macro_rules! forward_sync_read {
	($({$($lt:tt)*})? $ty:ty $(, #[$a1:meta] $(, #[$a2:meta] $(, #[$a3:meta])?)?)?) => {
		$(#[$a1])?
		impl $(<$($lt)*>)? ::std::io::Read for $ty {
			$($(#[$a2])?)?
			#[inline(always)]
			fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> { self.0.read(buf) }
			$($($(#[$a3])?)?)?
			#[inline(always)]
			fn read_vectored(
				&mut self,
				bufs: &mut [::std::io::IoSliceMut<'_>],
			) -> ::std::io::Result<usize> { self.0.read_vectored(bufs) }
			// read_to_end isn't here because this macro isn't supposed to be used on Chain-like
			// adapters
			// FUTURE is_read_vectored
		}
	};
}

macro_rules! forward_sync_write {
	(
		$({$($lt:tt)*})? $ty:ty
		$(, #[$a1:meta] $(, #[$a2:meta] $(, #[$a3:meta] $(, #[$a4:meta])?)?)?)?
	) => {
		$(#[$a1])?
		impl $(<$($lt)*>)? ::std::io::Write for $ty {
			$($(#[$a2])?)?
			#[inline(always)]
			fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> { self.0.write(buf) }
			$($(#[$a2])?)?
			#[inline(always)]
			fn flush(&mut self) -> ::std::io::Result<()> { self.0.flush() }
			$($($($(#[$a4])?)?)?)?
			#[inline(always)]
			fn write_vectored(
				&mut self,
				bufs: &[::std::io::IoSlice<'_>],
			) -> ::std::io::Result<usize> { self.0.write_vectored(bufs) }
			// FUTURE is_write_vectored
		}
	};
}

macro_rules! forward_sync_rw {
	(
		$({$($lt:tt)*})? $ty:ty
		$(, #[$a1:meta] $(, #[$a2:meta] $(, #[$a3:meta] $(, #[$a4:meta])?)?)?)?
	) => {
		forward_sync_read!($({$($lt)*})? $ty $(, #[$a1] $(, #[$a2] $(, #[$a3])?)?)?);
		forward_sync_write!($({$($lt)*})? $ty $(, #[$a1] $(, #[$a2] $(, #[$a3] $(, #[$a4])?)?)?)?);
	};
}

macro_rules! forward_sync_ref_read {
	($({$($lt:tt)*})? $ty:ty $(, #[$a1:meta] $(, #[$a2:meta] $(, #[$a3:meta])?)?)?) => {
		$(#[$a1])?
		impl $(<$($lt)*>)? ::std::io::Read for &$ty {
			$($(#[$a2])?)?
			#[inline(always)]
			fn read(&mut self, buf: &mut [u8]) -> ::std::io::Result<usize> {
				self.refwd().read(buf)
			}
			$($($(#[$a3])?)?)?
			#[inline(always)]
			fn read_vectored(
				&mut self,
				bufs: &mut [::std::io::IoSliceMut<'_>],
			) -> ::std::io::Result<usize> { self.refwd().read_vectored(bufs) }
			// FUTURE is_read_vectored
		}
	};
}

macro_rules! forward_sync_ref_write {
	(
		$({$($lt:tt)*})? $ty:ty
		$(, #[$a1:meta] $(, #[$a2:meta] $(, #[$a3:meta] $(, #[$a4:meta])?)?)?)?
	) => {
		$(#[$a1])?
		impl $(<$($lt)*>)? ::std::io::Write for &$ty {
			$($(#[$a2])?)?
			#[inline(always)]
			fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> { self.refwd().write(buf) }
			$($($(#[$a3])?)?)?
			#[inline(always)]
			fn flush(&mut self) -> ::std::io::Result<()> { self.refwd().flush() }
			$($($($(#[$a4])?)?)?)?
			#[inline(always)]
			fn write_vectored(
				&mut self,
				bufs: &[::std::io::IoSlice<'_>],
			) -> ::std::io::Result<usize> { self.refwd().write_vectored(bufs) }
			// FUTURE is_write_vectored
		}
	};
}

macro_rules! forward_sync_ref_rw {
	(
		$({$($lt:tt)*})? $ty:ty
		$(, #[$a1:meta] $(, #[$a2:meta] $(, #[$a3:meta] $(, #[$a4:meta])?)?)?)?
	) => {
		forward_sync_ref_read!($({$($lt)*})? $ty $(, #[$a1] $(, #[$a2] $(, #[$a3])?)?)?);
		forward_sync_ref_write!(
			$({$($lt)*})? $ty
			$(, #[$a1] $(, #[$a2] $(, #[$a3] $(, #[$a4])?)?)?)?
		);
	};
}

macro_rules! forward_tokio_read {
	($({$($lt:tt)*})? $ty:ty, $pinproj:ident) => {
		const _: () = {
			use ::tokio::io::{AsyncRead, ReadBuf};
			use ::std::{io, pin::Pin, task::{Context, Poll}};
			impl $(<$($lt)*>)? AsyncRead for $ty {
				#[inline(always)]
				fn poll_read(
					mut self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					buf: &mut ReadBuf<'_>,
				) -> Poll<io::Result<()>> {
					self.$pinproj().poll_read(cx, buf)
				}
			}
		};
	};
	($({$($lt:tt)*})? $ty:ty) => {
		forward_tokio_read!($({$($lt)*})? $ty, pinproj);
	};
}

macro_rules! forward_tokio_write {
	($({$($lt:tt)*})? $ty:ty, $pinproj:ident) => {
		const _: () = {
			use ::tokio::io::AsyncWrite;
			use ::std::{io::{self, IoSlice}, pin::Pin, task::{Context, Poll}};
			impl $(<$($lt)*>)? AsyncWrite for $ty {
				#[inline(always)]
				fn poll_write(
					mut self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					buf: &[u8],
				) -> Poll<io::Result<usize>> {
					self.$pinproj().poll_write(cx, buf)
				}
				#[inline(always)]
				fn poll_write_vectored(
					mut self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					bufs: &[IoSlice<'_>],
				) -> Poll<io::Result<usize>> {
					self.$pinproj().poll_write_vectored(cx, bufs)
				}
				#[inline(always)]
				fn is_write_vectored(&self) -> bool {
					self.refwd().is_write_vectored()
				}
				#[inline(always)]
				fn poll_flush(
					mut self: Pin<&mut Self>,
					cx: &mut Context<'_>,
				) -> Poll<io::Result<()>> {
					self.$pinproj().poll_flush(cx)
				}
				#[inline(always)]
				fn poll_shutdown(
					mut self: Pin<&mut Self>,
					cx: &mut Context<'_>,
				) -> Poll<io::Result<()>> {
					self.$pinproj().poll_shutdown(cx)
				}
			}
		};
	};
	($({$($lt:tt)*})? $ty:ty) => {
		forward_tokio_write!($({$($lt)*})? $ty, pinproj);
	};
}

macro_rules! forward_tokio_rw {
	($({$($lt:tt)*})? $ty:ty $(, $pinproj:ident)?) => {
		forward_tokio_read!($({$($lt)*})? $ty $(, $pinproj)?);
		forward_tokio_write!($({$($lt)*})? $ty $(, $pinproj)?);
	};
}

macro_rules! forward_tokio_ref_read {
	($({$($lt:tt)*})? $ty:ty) => {
		const _: () = {
			use ::tokio::io::{AsyncRead, ReadBuf};
			use ::std::{io, pin::Pin, task::{Context, Poll}};
			impl $(<$($lt)*>)? AsyncRead for &$ty {
				#[inline(always)]
				fn poll_read(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					buf: &mut ReadBuf<'_>,
				) -> Poll<io::Result<()>> {
					Pin::new(&mut (**self).refwd()).poll_read(cx, buf)
				}
			}
		};
	};
}

macro_rules! forward_tokio_ref_write {
	($({$($lt:tt)*})? $ty:ty) => {
		const _: () = {
			use ::tokio::io::AsyncWrite;
			use ::std::{io::{self, IoSlice}, pin::Pin, task::{Context, Poll}};
			impl $(<$($lt)*>)? AsyncWrite for &$ty {
				#[inline(always)]
				fn poll_write(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					buf: &[u8],
				) -> Poll<io::Result<usize>> {
					Pin::new(&mut (**self).refwd()).poll_write(cx, buf)
				}
				#[inline(always)]
				fn poll_write_vectored(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
					bufs: &[IoSlice<'_>],
				) -> Poll<io::Result<usize>> {
					Pin::new(&mut (**self).refwd()).poll_write_vectored(cx, bufs)
				}
				#[inline(always)]
				fn is_write_vectored(&self) -> bool {
					self.refwd().is_write_vectored()
				}
				#[inline(always)]
				fn poll_flush(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
				) -> Poll<io::Result<()>> {
					Pin::new(&mut (**self).refwd()).poll_flush(cx)
				}
				#[inline(always)]
				fn poll_shutdown(
					self: Pin<&mut Self>,
					cx: &mut Context<'_>,
				) -> Poll<io::Result<()>> {
					Pin::new(&mut (**self).refwd()).poll_shutdown(cx)
				}
			}
		};
	};
}

macro_rules! forward_tokio_ref_rw {
	($({$($lt:tt)*})? $ty:ty) => {
		forward_tokio_ref_read!($({$($lt)*})? $ty);
		forward_tokio_ref_write!($({$($lt)*})? $ty);
	};
}
