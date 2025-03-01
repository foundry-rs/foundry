//! Forwarding of `Debug` for newtypes that allows specifying a descriptive typename.

use std::fmt::{self, Debug, Formatter};
#[allow(dead_code)]
pub(crate) fn debug_forward_with_custom_name(
	nm: &str,
	fld: &dyn Debug,
	f: &mut Formatter<'_>,
) -> fmt::Result {
	f.debug_tuple(nm).field(fld).finish()
}

macro_rules! forward_debug {
	($({$($lt:tt)*})? $ty:ty, $nm:literal) => {
		impl $(<$($lt)*>)? ::std::fmt::Debug for $ty {
			#[inline(always)]
			fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
				$crate::macros::debug_forward_with_custom_name($nm, &self.0, f)
			}
		}
	};
	($({$($lt:tt)*})? $ty:ty) => {
		impl $(<$($lt)*>)? ::std::fmt::Debug for $ty {
			#[inline(always)]
			fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
				::std::fmt::Debug::fmt(&self.0, f)
			}
		}
	};
}
