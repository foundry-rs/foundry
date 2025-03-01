#![allow(dead_code)]

use std::{
	fmt::{self, Debug, Formatter},
	marker::PhantomData,
	mem::{transmute_copy, ManuallyDrop},
	sync::atomic::{
		AtomicU8,
		Ordering::{self, *},
	},
};

pub struct AtomicEnum<E: ReprU8>(AtomicU8, PhantomData<E>);
impl<E: ReprU8> AtomicEnum<E> {
	#[inline]
	pub fn new(val: E) -> Self {
		Self(AtomicU8::new(val.to_u8()), PhantomData)
	}
	#[inline]
	pub fn load(&self, ordering: Ordering) -> E {
		let v = self.0.load(ordering);
		unsafe { E::from_u8(v) }
	}
	#[inline]
	pub fn store(&self, val: E, ordering: Ordering) {
		self.0.store(val.to_u8(), ordering)
	}
	#[inline]
	pub fn compare_exchange(
		&self,
		current: E,
		new: E,
		success: Ordering,
		failure: Ordering,
	) -> Result<E, E> {
		let unwr = |v: u8| unsafe { E::from_u8(v) };
		self.0
			.compare_exchange(current.to_u8(), new.to_u8(), success, failure)
			.map(unwr)
			.map_err(unwr)
	}
	#[inline]
	pub fn get_mut(&mut self) -> &mut E {
		// SAFETY: ReprU8
		unsafe { &mut *self.0.as_ptr().cast() }
	}
}
impl<E: ReprU8 + Debug> Debug for AtomicEnum<E> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let val: E = unsafe { E::from_u8(self.0.load(SeqCst)) };
		f.debug_tuple("AtomicEnum").field(&val).finish()
	}
}

/// # Safety
/// Must be `#[repr(u8)]`.
pub unsafe trait ReprU8: Sized {
	#[inline(always)]
	fn to_u8(self) -> u8 {
		let slf = ManuallyDrop::new(self);
		unsafe { transmute_copy(&slf) }
	}
	#[inline(always)]
	unsafe fn from_u8(v: u8) -> Self {
		unsafe { transmute_copy(&v) }
	}
}
