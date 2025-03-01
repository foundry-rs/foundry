#![no_std]
#![allow(clippy::bool_comparison)]
//! This crate allows you to store a value that you can later take out atomically. As this
//! crate uses atomics, no locking is involved in taking the value out.
//!
//! As an example, you could store the [`Sender`] of an oneshot channel in an
//! [`AtomicTake`], which would allow you to notify the first time a closure is called.
//!
//! ```
//! use atomic_take::AtomicTake;
//! use tokio::sync::oneshot;
//!
//! let (send, mut recv) = oneshot::channel();
//!
//! let take = AtomicTake::new(send);
//! let closure = move || {
//!     if let Some(send) = take.take() {
//!         // Notify the first time this closure is called.
//!         send.send(()).unwrap();
//!     }
//! };
//!
//! closure();
//! assert!(recv.try_recv().is_ok());
//!
//! closure(); // This does nothing.
//! ```
//!
//! Additionally the closure above can be called concurrently from many threads. For
//! example, if you put the `AtomicTake` in an [`Arc`], you can share it between several
//! threads and receive a message from the first thread to run.
//!
//! ```
//! use std::thread;
//! use std::sync::Arc;
//! use atomic_take::AtomicTake;
//! use tokio::sync::oneshot;
//!
//! let (send, mut recv) = oneshot::channel();
//!
//! // Use an Arc to share the AtomicTake between several threads.
//! let take = Arc::new(AtomicTake::new(send));
//!
//! // Spawn three threads and try to send a message from each.
//! let mut handles = Vec::new();
//! for i in 0..3 {
//!     let take_clone = Arc::clone(&take);
//!     let join_handle = thread::spawn(move || {
//!
//!         // Check if this thread is first and send a message if so.
//!         if let Some(send) = take_clone.take() {
//!             // Send the index of the thread.
//!             send.send(i).unwrap();
//!         }
//!
//!     });
//!     handles.push(join_handle);
//! }
//! // Wait for all three threads to finish.
//! for handle in handles {
//!     handle.join().unwrap();
//! }
//!
//! // After all the threads finished, try to send again.
//! if let Some(send) = take.take() {
//!     // This will definitely not happen.
//!     send.send(100).unwrap();
//! }
//!
//! // Confirm that one of the first three threads got to send the message first.
//! assert!(recv.try_recv().unwrap() < 3);
//! ```
//!
//! This crate does not require the standard library.
//!
//! [`Sender`]: https://docs.rs/tokio/latest/tokio/sync/oneshot/struct.Sender.html
//! [`AtomicTake`]: struct.AtomicTake.html
//! [`Arc`]: https://doc.rust-lang.org/std/sync/struct.Arc.html

use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use core::fmt;

type PhantomUnsync = PhantomData<Cell<u8>>;

/// A container with an atomic take operation.
pub struct AtomicTake<T> {
    taken: AtomicBool,
    value: MaybeUninit<T>,
    _unsync: PhantomUnsync,
}

impl<T> AtomicTake<T> {
    /// Create a new `AtomicTake` with the given value.
    pub const fn new(value: T) -> Self {
        AtomicTake {
            taken: AtomicBool::new(false),
            value: MaybeUninit::new(value),
            _unsync: PhantomData,
        }
    }
    /// Create an empty `AtomicTake` that contains no value.
    pub const fn empty() -> Self {
        AtomicTake {
            taken: AtomicBool::new(true),
            value: MaybeUninit::uninit(),
            _unsync: PhantomData,
        }
    }
    /// Takes out the value from this `AtomicTake`. It is guaranteed that exactly one
    /// caller will receive the value and all others will receive `None`.
    pub fn take(&self) -> Option<T> {
        if self.taken.swap(true, Ordering::Relaxed) == false {
            unsafe { Some(ptr::read(self.value.as_ptr())) }
        } else {
            None
        }
    }
    /// This methods does the same as `take`, but does not use an atomic swap.
    ///
    /// This is safe because you cannot call this method without unique access to the
    /// `AtomicTake`, so no other threads will try to take it concurrently.
    pub fn take_mut(&mut self) -> Option<T> {
        if mem::replace(self.taken.get_mut(), true) == false {
            unsafe { Some(ptr::read(self.value.as_ptr())) }
        } else {
            None
        }
    }

    /// Check whether the value is taken. Note that if this returns `false`, then this
    /// is immediately stale if another thread could be concurrently trying to take it.
    pub fn is_taken(&self) -> bool {
        self.taken.load(Ordering::Relaxed)
    }

    /// Insert a new value into the `AtomicTake` and return the previous value.
    ///
    /// This function requires unique access to ensure no other threads accesses the
    /// `AtomicTake` concurrently, as this operation cannot be performed atomically
    /// without a lock.
    pub fn insert(&mut self, value: T) -> Option<T> {
        let previous = self.take_mut();

        unsafe {
            ptr::write(self.value.as_mut_ptr(), value);
            *self.taken.get_mut() = false;
        }

        // Could also be written as below, but this avoids running the destructor.
        // *self = AtomicTake::new(value);

        previous
    }
}

impl<T> Drop for AtomicTake<T> {
    fn drop(&mut self) {
        if !*self.taken.get_mut() {
            unsafe {
                ptr::drop_in_place(self.value.as_mut_ptr());
            }
        }
    }
}

// As this api can only be used to move values between threads, Sync is not needed.
unsafe impl<T: Send> Sync for AtomicTake<T> {}

impl<T> From<T> for AtomicTake<T> {
    fn from(t: T) -> AtomicTake<T> {
        AtomicTake::new(t)
    }
}

impl<T> fmt::Debug for AtomicTake<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_taken() {
            write!(f, "Empty AtomicTake")
        } else {
            write!(f, "Non-Empty AtomicTake")
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::AtomicTake;

    struct CountDrops {
        counter: *mut u32,
    }
    impl Drop for CountDrops {
        fn drop(&mut self) {
            unsafe {
                *self.counter += 1;
            }
        }
    }

    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("Panic on drop called.");
        }
    }

    #[test]
    fn drop_calls_drop() {
        let mut counter = 0;

        let take = AtomicTake::new(CountDrops {
            counter: &mut counter,
        });
        drop(take);

        assert_eq!(counter, 1);
    }

    #[test]
    fn taken_not_dropped_twice() {
        let mut counter = 0;

        let take = AtomicTake::new(CountDrops {
            counter: &mut counter,
        });
        take.take();

        assert_eq!(counter, 1);

        drop(take);

        assert_eq!(counter, 1);
    }

    #[test]
    fn taken_mut_not_dropped_twice() {
        let mut counter = 0;

        let mut take = AtomicTake::new(CountDrops {
            counter: &mut counter,
        });
        take.take_mut();

        assert_eq!(counter, 1);

        drop(take);

        assert_eq!(counter, 1);
    }

    #[test]
    fn insert_dropped_once() {
        let mut counter1 = 0;
        let mut counter2 = 0;

        let mut take = AtomicTake::new(CountDrops {
            counter: &mut counter1,
        });
        assert!(!take.is_taken());
        take.insert(CountDrops {
            counter: &mut counter2,
        });
        assert!(!take.is_taken());
        drop(take);

        assert_eq!(counter1, 1);
        assert_eq!(counter2, 1);
    }

    #[test]
    fn insert_take() {
        let mut counter1 = 0;
        let mut counter2 = 0;

        let mut take = AtomicTake::new(CountDrops {
            counter: &mut counter1,
        });
        take.insert(CountDrops {
            counter: &mut counter2,
        });

        assert!(!take.is_taken());

        assert_eq!(counter1, 1);
        assert_eq!(counter2, 0);

        drop(take);

        assert_eq!(counter1, 1);
        assert_eq!(counter2, 1);
    }

    #[test]
    fn empty_no_drop() {
        let take: AtomicTake<PanicOnDrop> = AtomicTake::empty();
        assert!(take.is_taken());
        drop(take);
    }

    #[test]
    fn empty_insert() {
        let mut take = AtomicTake::empty();

        assert!(take.is_taken());

        let mut counter = 0;

        take.insert(CountDrops {
            counter: &mut counter,
        });

        assert!(!take.is_taken());

        drop(take);

        assert_eq!(counter, 1);
    }
}
