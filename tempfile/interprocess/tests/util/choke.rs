use std::sync::{Arc, Condvar, Mutex, Weak};

/// Choke – a rate-limiting semaphore that does not protect any concurrently accessed resource.
#[derive(Debug)]
pub struct Choke(Arc<ChokeInner>);
impl Choke {
	pub fn new(limit: u32) -> Self {
		let inner = ChokeInner {
			count: Mutex::new(0),
			limit,
			condvar: Condvar::new(),
		};
		Self(Arc::new(inner))
	}
	pub fn take(&self) -> ChokeGuard {
		let mut lock = Some(self.0.count.lock().unwrap());
		loop {
			let mut c_lock = lock.take().unwrap();
			if *c_lock < self.0.limit {
				*c_lock += 1;
				return self.make_guard();
			} else {
				let c_lock = self.0.condvar.wait(c_lock).unwrap();
				lock = Some(c_lock);
			}
		}
	}
	fn make_guard(&self) -> ChokeGuard {
		ChokeGuard(Arc::downgrade(&self.0))
	}
}
impl Clone for Choke {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}

#[derive(Debug)]
struct ChokeInner {
	count: Mutex<u32>,
	limit: u32,
	condvar: Condvar,
}
impl ChokeInner {
	fn decrement(&self) {
		let mut count = self.count.lock().unwrap();
		*count = count.checked_sub(1).expect("choke counter underflow");
		self.condvar.notify_one();
	}
}

/// Guard for `Choke` that owns one unit towards the limit.
pub struct ChokeGuard(Weak<ChokeInner>);
impl Drop for ChokeGuard {
	fn drop(&mut self) {
		if let Some(inner) = self.0.upgrade() {
			inner.decrement();
		}
	}
}
