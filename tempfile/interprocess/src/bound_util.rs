//! Trait bound utilities.

use std::io::prelude::*;
#[cfg(feature = "tokio")]
use tokio::io::{AsyncRead as TokioAsyncRead, AsyncWrite as TokioAsyncWrite};

pub(crate) trait Is<T: ?Sized> {}
impl<T: ?Sized> Is<T> for T {}

macro_rules! bound_util {
	(#[doc = $doc:literal] $trtname:ident of $otrt:ident with $aty:ident mtd $mtd:ident) => {
		#[doc = $doc]
		pub trait $trtname {
			#[doc(hidden)]
			#[allow(private_bounds)]
			type $aty<'a>: $otrt + Is<&'a Self>
			where
				Self: 'a;
			#[doc = concat!(
								"Returns `self` with the guarantee that `&Self` implements `",
								stringify!($otrt),
								"` encoded in a way which is visible to Rust's type system.",
							)]
			fn $mtd(&self) -> Self::$aty<'_>;
		}
		impl<T: ?Sized> $trtname for T
		where
			for<'a> &'a T: $otrt,
		{
			type $aty<'a> = &'a Self
						where Self: 'a;
			#[inline(always)]
			fn $mtd(&self) -> Self::$aty<'_> {
				self
			}
		}
	};
	($(#[doc = $doc:literal] $trtname:ident of $otrt:ident with $aty:ident mtd $mtd:ident)+) => {$(
		bound_util!(#[doc = $doc] $trtname of $otrt with $aty mtd $mtd);
	)+};
}

bound_util! {
	/// [`Read`] by reference.
	RefRead		of Read		with Read	mtd as_read
	/// [`Write`] by reference.
	RefWrite	of Write	with Write	mtd as_write
}

#[cfg(feature = "tokio")]
bound_util! {
	/// [Tokio's `AsyncRead`](TokioAsyncRead) by reference.
	RefTokioAsyncRead	of TokioAsyncRead	with Read	mtd as_tokio_async_read
	/// [Tokio's `AsyncWrite`](TokioAsyncWrite) by reference.
	RefTokioAsyncWrite	of TokioAsyncWrite	with Write	mtd as_tokio_async_write
}
