//! The limbo which dropped streams are sent to if send buffer preservation is enabled.
//!
//! Because dropping a named pipe file handle, be it a client or a server, discards its send buffer,
//! the portability-conscious local socket interface requires this additional feature to allow for
//! the common use case of dropping right after sending a graceful shutdown message.

use crate::SubUsizeExt;

const LIMBO_SLOTS: u8 = 16;

/// Common result type for operations that complete with no output but may reject their input,
/// requiring some form of retry.
pub(crate) type MaybeReject<T> = Result<(), T>;

#[allow(clippy::as_conversions)]
pub(crate) struct LimboPool<S> {
	senders: [Option<S>; LIMBO_SLOTS as _],
	count: u8,
	count_including_overflow: usize,
}
impl<S> LimboPool<S> {
	fn incr_count_including_overflow(&mut self) {
		self.count_including_overflow = self.count_including_overflow.saturating_add(1);
	}
	pub fn add_sender(&mut self, s: S) -> MaybeReject<S> {
		self.incr_count_including_overflow();
		#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
		if self.count < LIMBO_SLOTS {
			self.senders[self.count.to_usize()] = Some(s);
			self.count += 1;
			Ok(())
		} else {
			Err(s)
		}
	}
	/// Tries shoving the given accumulant into the given maybe-rejecting function with every
	/// available sender.
	#[allow(clippy::unwrap_used, clippy::unwrap_in_result)] // Used to work with ownership.
	pub fn linear_try<T>(
		&mut self,
		acc: T,
		mut f: impl FnMut(&mut S, T) -> MaybeReject<T>,
	) -> MaybeReject<T> {
		let mut acc = Some(acc);
		#[allow(clippy::indexing_slicing)]
		for sender in &mut self.senders[0..(usize::from(self.count))] {
			let regain = match f(sender.as_mut().unwrap(), acc.take().unwrap()) {
				Ok(()) => return Ok(()),
				Err(r) => r,
			};
			acc = Some(regain);
		}
		Err(acc.unwrap())
	}
	/// Performs `linear_try` with `acc` and `tryf`, and if that fails, calls `createf` and consumes
	/// its output with `add_sender` if the pool has vacant space, resorting to `fullf` otherwise.
	pub fn linear_try_or_create<T>(
		&mut self,
		acc: T,
		tryf: impl FnMut(&mut S, T) -> MaybeReject<T>,
		// First argument is the index of the new sender.
		createf: impl FnOnce(usize, T) -> S,
		// Same here.
		fullf: impl FnOnce(usize, T),
	) {
		let acc = match self.linear_try(acc, tryf) {
			Ok(()) => return,
			Err(regain) => regain,
		};
		if self.count < LIMBO_SLOTS {
			// Cannot error.
			let _ = self.add_sender(createf(self.count.into(), acc));
		} else {
			fullf(self.count_including_overflow, acc);
			self.incr_count_including_overflow();
		}
	}
}
impl<S> Default for LimboPool<S> {
	fn default() -> Self {
		Self {
			// hmm today i will initialize an array
			senders: Default::default(),
			count: 0,
			count_including_overflow: 0,
		}
	}
}
