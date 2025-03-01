use crate::{AtomicEnum, ReprU8};
use std::sync::atomic::Ordering::{self, *};

#[derive(Debug)]
pub(crate) struct NeedsFlush(AtomicEnum<NeedsFlushVal>);
impl NeedsFlush {
	#[inline]
	pub(crate) fn mark_dirty(&self) {
		let _ = self.0.compare_exchange(
			NeedsFlushVal::No,
			NeedsFlushVal::Once,
			AcqRel,
			Relaxed, // We do not care about the loaded value
		);
	}
	#[inline]
	pub(crate) fn on_clone(&self) {
		self.0.store(NeedsFlushVal::Always, Release);
	}
	#[inline]
	pub(crate) fn take(&self) -> bool {
		match self
			.0
			.compare_exchange(NeedsFlushVal::Once, NeedsFlushVal::No, AcqRel, Acquire)
		{
			Ok(..) => true,
			Err(NeedsFlushVal::Always) => true,
			Err(.. /* NeedsFlushVal::No */) => false,
		}
	}
	#[inline]
	pub(crate) fn clear(&self) {
		let _ = self
			.0
			.compare_exchange(NeedsFlushVal::Once, NeedsFlushVal::No, AcqRel, Relaxed);
	}
	#[inline]
	pub(crate) fn get(&self, ordering: Ordering) -> bool {
		matches!(
			self.0.load(ordering),
			NeedsFlushVal::Once | NeedsFlushVal::Always
		)
	}
	#[inline]
	pub(crate) fn get_mut(&mut self) -> bool {
		matches!(
			self.0.get_mut(),
			NeedsFlushVal::Once | NeedsFlushVal::Always
		)
	}
}
impl From<NeedsFlushVal> for NeedsFlush {
	#[inline]
	fn from(val: NeedsFlushVal) -> Self {
		Self(AtomicEnum::new(val))
	}
}

#[derive(Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum NeedsFlushVal {
	No,
	Once,
	Always,
}
unsafe impl ReprU8 for NeedsFlushVal {}
