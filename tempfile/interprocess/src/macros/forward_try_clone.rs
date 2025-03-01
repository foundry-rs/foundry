//! Forwarding of `Debug` for newtypes. Is also a derive macro in some sense.

macro_rules! forward_try_clone {
	($({$($lt:tt)*})? $ty:ty) => {
		impl $(<$($lt)*>)? crate::TryClone for $ty {
			#[inline]
			fn try_clone(&self) -> ::std::io::Result<Self> {
				Ok(Self(crate::TryClone::try_clone(&self.0)?))
			}
		}
	};
}
