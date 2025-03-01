//! Derive macros for trivial `From`. Rust 1.63+. Lifetime arguments on impls can be specified in
//! curly braces.

macro_rules! derive_trivial_from {
	($({$($forcl:tt)*})? $dst:ty, $src:ty) => {
		impl $(<$($forcl)*>)? ::std::convert::From<$src> for $dst {
			#[inline]
			fn from(src: $src) -> Self { Self(src) }
		}
	};
}

macro_rules! derive_trivial_into {
	($({$($forcl:tt)*})? $src:ty, $dst:ty) => {
		impl $(<$($forcl)*>)? ::std::convert::From<$src> for $dst {
			#[inline]
			fn from(src: $src) -> Self { src.0 }
		}
	};
}

macro_rules! derive_trivial_conv {
	($({$($forcl:tt)*})? $ty1:ty, $ty2:ty) => {
		derive_trivial_from!($({$($forcl)*})? $ty1, $ty2);
		derive_trivial_into!($({$($forcl)*})? $ty1, $ty2);
	};
}
