//! Notify async tasks or threads.
//!
//! This is a synchronization primitive similar to [eventcounts] invented by Dmitry Vyukov.
//!
//! You can use this crate to turn non-blocking data structures into async or blocking data
//! structures. See a [simple mutex] implementation that exposes an async and a blocking interface
//! for acquiring locks.
//!
//! [eventcounts]: https://www.1024cores.net/home/lock-free-algorithms/eventcounts
//! [simple mutex]: https://github.com/smol-rs/event-listener/blob/master/examples/mutex.rs
//!
//! # Examples
//!
//! Wait until another thread sets a boolean flag:
//!
//! ```
//! use std::sync::atomic::{AtomicBool, Ordering};
//! use std::sync::Arc;
//! use std::thread;
//! use std::time::Duration;
//! use std::usize;
//! use event_listener::Event;
//!
//! let flag = Arc::new(AtomicBool::new(false));
//! let event = Arc::new(Event::new());
//!
//! // Spawn a thread that will set the flag after 1 second.
//! thread::spawn({
//!     let flag = flag.clone();
//!     let event = event.clone();
//!     move || {
//!         // Wait for a second.
//!         thread::sleep(Duration::from_secs(1));
//!
//!         // Set the flag.
//!         flag.store(true, Ordering::SeqCst);
//!
//!         // Notify all listeners that the flag has been set.
//!         event.notify(usize::MAX);
//!     }
//! });
//!
//! // Wait until the flag is set.
//! loop {
//!     // Check the flag.
//!     if flag.load(Ordering::SeqCst) {
//!         break;
//!     }
//!
//!     // Start listening for events.
//!     let mut listener = event.listen();
//!
//!     // Check the flag again after creating the listener.
//!     if flag.load(Ordering::SeqCst) {
//!         break;
//!     }
//!
//!     // Wait for a notification and continue the loop.
//!     listener.as_mut().wait();
//! }
//! ```
//!
//! # Features
//!
//! - The `portable-atomic` feature enables the use of the [`portable-atomic`] crate to provide
//!   atomic operations on platforms that don't support them.
//!
//! [`portable-atomic`]: https://crates.io/crates/portable-atomic

#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/smol-rs/smol/master/assets/images/logo_fullsize_transparent.png"
)]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/smol-rs/smol/master/assets/images/logo_fullsize_transparent.png"
)]

extern crate alloc;

#[cfg_attr(feature = "std", path = "std.rs")]
#[cfg_attr(not(feature = "std"), path = "no_std.rs")]
mod sys;

mod notify;

use alloc::boxed::Box;

use core::borrow::Borrow;
use core::fmt;
use core::future::Future;
use core::mem::ManuallyDrop;
use core::pin::Pin;
use core::ptr;
use core::task::{Context, Poll, Waker};

#[cfg(all(feature = "std", not(target_family = "wasm")))]
use {
    parking::{Parker, Unparker},
    std::time::{Duration, Instant},
};

use sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use sync::{Arc, WithMut};

use notify::{Internal, NotificationPrivate};
pub use notify::{IntoNotification, Notification};

/// Useful traits for notifications.
pub mod prelude {
    pub use crate::{IntoNotification, Notification};
}

/// Inner state of [`Event`].
struct Inner<T> {
    /// The number of notified entries, or `usize::MAX` if all of them have been notified.
    ///
    /// If there are no entries, this value is set to `usize::MAX`.
    notified: AtomicUsize,

    /// Inner queue of event listeners.
    ///
    /// On `std` platforms, this is an intrusive linked list. On `no_std` platforms, this is a
    /// more traditional `Vec` of listeners, with an atomic queue used as a backup for high
    /// contention.
    list: sys::List<T>,
}

impl<T> Inner<T> {
    fn new() -> Self {
        Self {
            notified: AtomicUsize::new(core::usize::MAX),
            list: sys::List::new(),
        }
    }
}

/// A synchronization primitive for notifying async tasks and threads.
///
/// Listeners can be registered using [`Event::listen()`]. There are two ways to notify listeners:
///
/// 1. [`Event::notify()`] notifies a number of listeners.
/// 2. [`Event::notify_additional()`] notifies a number of previously unnotified listeners.
///
/// If there are no active listeners at the time a notification is sent, it simply gets lost.
///
/// There are two ways for a listener to wait for a notification:
///
/// 1. In an asynchronous manner using `.await`.
/// 2. In a blocking manner by calling [`EventListener::wait()`] on it.
///
/// If a notified listener is dropped without receiving a notification, dropping will notify
/// another active listener. Whether one *additional* listener will be notified depends on what
/// kind of notification was delivered.
///
/// Listeners are registered and notified in the first-in first-out fashion, ensuring fairness.
pub struct Event<T = ()> {
    /// A pointer to heap-allocated inner state.
    ///
    /// This pointer is initially null and gets lazily initialized on first use. Semantically, it
    /// is an `Arc<Inner>` so it's important to keep in mind that it contributes to the [`Arc`]'s
    /// reference count.
    inner: AtomicPtr<Inner<T>>,
}

unsafe impl<T: Send> Send for Event<T> {}
unsafe impl<T: Send> Sync for Event<T> {}

impl<T> core::panic::UnwindSafe for Event<T> {}
impl<T> core::panic::RefUnwindSafe for Event<T> {}

impl<T> fmt::Debug for Event<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_inner() {
            Some(inner) => {
                let notified_count = inner.notified.load(Ordering::Relaxed);
                let total_count = match inner.list.total_listeners() {
                    Ok(total_count) => total_count,
                    Err(_) => {
                        return f
                            .debug_tuple("Event")
                            .field(&format_args!("<locked>"))
                            .finish()
                    }
                };

                f.debug_struct("Event")
                    .field("listeners_notified", &notified_count)
                    .field("listeners_total", &total_count)
                    .finish()
            }
            None => f
                .debug_tuple("Event")
                .field(&format_args!("<uninitialized>"))
                .finish(),
        }
    }
}

impl Default for Event {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Event<T> {
    /// Creates a new `Event` with a tag type.
    ///
    /// Tagging cannot be implemented efficiently on `no_std`, so this is only available when the
    /// `std` feature is enabled.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::<usize>::with_tag();
    /// ```
    #[cfg(feature = "std")]
    #[inline]
    pub const fn with_tag() -> Self {
        Self {
            inner: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Tell whether any listeners are currently notified.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let listener = event.listen();
    /// assert!(!event.is_notified());
    ///
    /// event.notify(1);
    /// assert!(event.is_notified());
    /// ```
    #[inline]
    pub fn is_notified(&self) -> bool {
        self.try_inner()
            .map_or(false, |inner| inner.notified.load(Ordering::Acquire) > 0)
    }

    /// Returns a guard listening for a notification.
    ///
    /// This method emits a `SeqCst` fence after registering a listener. For now, this method
    /// is an alias for calling [`EventListener::new()`], pinning it to the heap, and then
    /// inserting it into a list.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let listener = event.listen();
    /// ```
    ///
    /// # Caveats
    ///
    /// The above example is equivalent to this code:
    ///
    /// ```
    /// use event_listener::{Event, EventListener};
    ///
    /// let event = Event::new();
    /// let mut listener = Box::pin(EventListener::new());
    /// listener.as_mut().listen(&event);
    /// ```
    ///
    /// It creates a new listener, pins it to the heap, and inserts it into the linked list
    /// of listeners. While this type of usage is simple, it may be desired to eliminate this
    /// heap allocation. In this case, consider using the [`EventListener::new`] constructor
    /// directly, which allows for greater control over where the [`EventListener`] is
    /// allocated. However, users of this `new` method must be careful to ensure that the
    /// [`EventListener`] is `listen`ing before waiting on it; panics may occur otherwise.
    #[cold]
    pub fn listen(&self) -> Pin<Box<EventListener<T>>> {
        let mut listener = Box::pin(EventListener::new());
        listener.as_mut().listen(self);
        listener
    }

    /// Notifies a number of active listeners.
    ///
    /// The number is allowed to be zero or exceed the current number of listeners.
    ///
    /// The [`Notification`] trait is used to define what kind of notification is delivered.
    /// The default implementation (implemented on `usize`) is a notification that only notifies
    /// *at least* the specified number of listeners.
    ///
    /// In certain cases, this function emits a `SeqCst` fence before notifying listeners.
    ///
    /// This function returns the number of [`EventListener`]s that were notified by this call.
    ///
    /// # Caveats
    ///
    /// If the `std` feature is disabled, the notification will be delayed under high contention,
    /// such as when another thread is taking a while to `notify` the event. In this circumstance,
    /// this function will return `0` instead of the number of listeners actually notified. Therefore
    /// if the `std` feature is disabled the return value of this function should not be relied upon
    /// for soundness and should be used only as a hint.
    ///
    /// If the `std` feature is enabled, no spurious returns are possible, since the `std`
    /// implementation uses system locking primitives to ensure there is no unavoidable
    /// contention.
    ///
    /// # Examples
    ///
    /// Use the default notification strategy:
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify(1);
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify(2);
    /// ```
    ///
    /// Notify without emitting a `SeqCst` fence. This uses the [`relaxed`] notification strategy.
    /// This is equivalent to calling [`Event::notify_relaxed()`].
    ///
    /// [`relaxed`]: IntoNotification::relaxed
    ///
    /// ```
    /// use event_listener::{prelude::*, Event};
    /// use std::sync::atomic::{self, Ordering};
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify(1.relaxed());
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // We should emit a fence manually when using relaxed notifications.
    /// atomic::fence(Ordering::SeqCst);
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify(2.relaxed());
    /// ```
    ///
    /// Notify additional listeners. In contrast to [`Event::notify()`], this method will notify `n`
    /// *additional* listeners that were previously unnotified. This uses the [`additional`]
    /// notification strategy. This is equivalent to calling [`Event::notify_additional()`].
    ///
    /// [`additional`]: IntoNotification::additional
    ///
    /// ```
    /// use event_listener::{prelude::*, Event};
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify(1.additional());
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify(1.additional());
    /// event.notify(1.additional());
    /// ```
    ///
    /// Notifies with the [`additional`] and [`relaxed`] strategies at the same time. This is
    /// equivalent to calling [`Event::notify_additional_relaxed()`].
    ///
    /// ```
    /// use event_listener::{prelude::*, Event};
    /// use std::sync::atomic::{self, Ordering};
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify(1.additional().relaxed());
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // We should emit a fence manually when using relaxed notifications.
    /// atomic::fence(Ordering::SeqCst);
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify(1.additional().relaxed());
    /// event.notify(1.additional().relaxed());
    /// ```
    #[inline]
    pub fn notify(&self, notify: impl IntoNotification<Tag = T>) -> usize {
        let notify = notify.into_notification();

        // Make sure the notification comes after whatever triggered it.
        notify.fence(notify::Internal::new());

        if let Some(inner) = self.try_inner() {
            let limit = if notify.is_additional(Internal::new()) {
                core::usize::MAX
            } else {
                notify.count(Internal::new())
            };

            // Notify if there is at least one unnotified listener and the number of notified
            // listeners is less than `limit`.
            if inner.needs_notification(limit) {
                return inner.notify(notify);
            }
        }

        0
    }

    /// Return a reference to the inner state if it has been initialized.
    #[inline]
    fn try_inner(&self) -> Option<&Inner<T>> {
        let inner = self.inner.load(Ordering::Acquire);
        unsafe { inner.as_ref() }
    }

    /// Returns a raw, initialized pointer to the inner state.
    ///
    /// This returns a raw pointer instead of reference because `from_raw`
    /// requires raw/mut provenance: <https://github.com/rust-lang/rust/pull/67339>.
    fn inner(&self) -> *const Inner<T> {
        let mut inner = self.inner.load(Ordering::Acquire);

        // If this is the first use, initialize the state.
        if inner.is_null() {
            // Allocate the state on the heap.
            let new = Arc::new(Inner::<T>::new());

            // Convert the state to a raw pointer.
            let new = Arc::into_raw(new) as *mut Inner<T>;

            // Replace the null pointer with the new state pointer.
            inner = self
                .inner
                .compare_exchange(inner, new, Ordering::AcqRel, Ordering::Acquire)
                .unwrap_or_else(|x| x);

            // Check if the old pointer value was indeed null.
            if inner.is_null() {
                // If yes, then use the new state pointer.
                inner = new;
            } else {
                // If not, that means a concurrent operation has initialized the state.
                // In that case, use the old pointer and deallocate the new one.
                unsafe {
                    drop(Arc::from_raw(new));
                }
            }
        }

        inner
    }
}

impl Event<()> {
    /// Creates a new [`Event`].
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// ```
    #[inline]
    pub const fn new() -> Self {
        Self {
            inner: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Notifies a number of active listeners without emitting a `SeqCst` fence.
    ///
    /// The number is allowed to be zero or exceed the current number of listeners.
    ///
    /// In contrast to [`Event::notify_additional()`], this method only makes sure *at least* `n`
    /// listeners among the active ones are notified.
    ///
    /// Unlike [`Event::notify()`], this method does not emit a `SeqCst` fence.
    ///
    /// This method only works for untagged events. In other cases, it is recommended to instead
    /// use [`Event::notify()`] like so:
    ///
    /// ```
    /// use event_listener::{prelude::*, Event};
    /// let event = Event::new();
    ///
    /// // Old way:
    /// event.notify_relaxed(1);
    ///
    /// // New way:
    /// event.notify(1.relaxed());
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    /// use std::sync::atomic::{self, Ordering};
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify_relaxed(1);
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // We should emit a fence manually when using relaxed notifications.
    /// atomic::fence(Ordering::SeqCst);
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify_relaxed(2);
    /// ```
    #[inline]
    pub fn notify_relaxed(&self, n: usize) -> usize {
        self.notify(n.relaxed())
    }

    /// Notifies a number of active and still unnotified listeners.
    ///
    /// The number is allowed to be zero or exceed the current number of listeners.
    ///
    /// In contrast to [`Event::notify()`], this method will notify `n` *additional* listeners that
    /// were previously unnotified.
    ///
    /// This method emits a `SeqCst` fence before notifying listeners.
    ///
    /// This method only works for untagged events. In other cases, it is recommended to instead
    /// use [`Event::notify()`] like so:
    ///
    /// ```
    /// use event_listener::{prelude::*, Event};
    /// let event = Event::new();
    ///
    /// // Old way:
    /// event.notify_additional(1);
    ///
    /// // New way:
    /// event.notify(1.additional());
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify_additional(1);
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify_additional(1);
    /// event.notify_additional(1);
    /// ```
    #[inline]
    pub fn notify_additional(&self, n: usize) -> usize {
        self.notify(n.additional())
    }

    /// Notifies a number of active and still unnotified listeners without emitting a `SeqCst`
    /// fence.
    ///
    /// The number is allowed to be zero or exceed the current number of listeners.
    ///
    /// In contrast to [`Event::notify()`], this method will notify `n` *additional* listeners that
    /// were previously unnotified.
    ///
    /// Unlike [`Event::notify_additional()`], this method does not emit a `SeqCst` fence.
    ///
    /// This method only works for untagged events. In other cases, it is recommended to instead
    /// use [`Event::notify()`] like so:
    ///
    /// ```
    /// use event_listener::{prelude::*, Event};
    /// let event = Event::new();
    ///
    /// // Old way:
    /// event.notify_additional_relaxed(1);
    ///
    /// // New way:
    /// event.notify(1.additional().relaxed());
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    /// use std::sync::atomic::{self, Ordering};
    ///
    /// let event = Event::new();
    ///
    /// // This notification gets lost because there are no listeners.
    /// event.notify(1);
    ///
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    /// let listener3 = event.listen();
    ///
    /// // We should emit a fence manually when using relaxed notifications.
    /// atomic::fence(Ordering::SeqCst);
    ///
    /// // Notifies two listeners.
    /// //
    /// // Listener queueing is fair, which means `listener1` and `listener2`
    /// // get notified here since they start listening before `listener3`.
    /// event.notify_additional_relaxed(1);
    /// event.notify_additional_relaxed(1);
    /// ```
    #[inline]
    pub fn notify_additional_relaxed(&self, n: usize) -> usize {
        self.notify(n.additional().relaxed())
    }
}

impl<T> Drop for Event<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.with_mut(|&mut inner| {
            // If the state pointer has been initialized, drop it.
            if !inner.is_null() {
                unsafe {
                    drop(Arc::from_raw(inner));
                }
            }
        })
    }
}

pin_project_lite::pin_project! {
    /// A guard waiting for a notification from an [`Event`].
    ///
    /// There are two ways for a listener to wait for a notification:
    ///
    /// 1. In an asynchronous manner using `.await`.
    /// 2. In a blocking manner by calling [`EventListener::wait()`] on it.
    ///
    /// If a notified listener is dropped without receiving a notification, dropping will notify
    /// another active listener. Whether one *additional* listener will be notified depends on what
    /// kind of notification was delivered.
    ///
    /// The listener is not registered into the linked list inside of the [`Event`] by default if
    /// it is created via the `new()` method. It needs to be pinned first before being inserted
    /// using the `listen()` method. After the listener has begun `listen`ing, the user can
    /// `await` it like a future or call `wait()` to block the current thread until it is notified.
    ///
    /// ## Examples
    ///
    /// ```
    /// use event_listener::{Event, EventListener};
    /// use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    /// use std::thread;
    /// use std::time::Duration;
    ///
    /// // Some flag to wait on.
    /// let flag = Arc::new(AtomicBool::new(false));
    ///
    /// // Create an event to wait on.
    /// let event = Arc::new(Event::new());
    ///
    /// thread::spawn({
    ///     let flag = flag.clone();
    ///     let event = event.clone();
    ///     move || {
    ///         thread::sleep(Duration::from_secs(2));
    ///         flag.store(true, Ordering::SeqCst);
    ///
    ///         // Wake up the listener.
    ///         event.notify_additional(std::usize::MAX);
    ///     }
    /// });
    ///
    /// let listener = EventListener::new();
    ///
    /// // Make sure that the event listener is pinned before doing anything else.
    /// //
    /// // We pin the listener to the stack here, as it lets us avoid a heap allocation.
    /// futures_lite::pin!(listener);
    ///
    /// // Wait for the flag to become ready.
    /// loop {
    ///     if flag.load(Ordering::Acquire) {
    ///         // We are done.
    ///         break;
    ///     }
    ///
    ///     if listener.is_listening() {
    ///         // We are inserted into the linked list and we can now wait.
    ///         listener.as_mut().wait();
    ///     } else {
    ///         // We need to insert ourselves into the list. Since this insertion is an atomic
    ///         // operation, we should check the flag again before waiting.
    ///         listener.as_mut().listen(&event);
    ///     }
    /// }
    /// ```
    ///
    /// The above example is equivalent to the one provided in the crate level example. However,
    /// it has some advantages. By directly creating the listener with `EventListener::new()`,
    /// we have control over how the listener is handled in memory. We take advantage of this by
    /// pinning the `listener` variable to the stack using the [`futures_lite::pin`] macro. In
    /// contrast, `Event::listen` binds the listener to the heap.
    ///
    /// However, this additional power comes with additional responsibility. By default, the
    /// event listener is created in an "uninserted" state. This property means that any
    /// notifications delivered to the [`Event`] by default will not wake up this listener.
    /// Before any notifications can be received, the `listen()` method must be called on
    /// `EventListener` to insert it into the list of listeners. After a `.await` or a `wait()`
    /// call has completed, `listen()` must be called again if the user is still interested in
    /// any events.
    ///
    /// [`futures_lite::pin`]: https://docs.rs/futures-lite/latest/futures_lite/macro.pin.html
    #[project(!Unpin)] // implied by Listener, but can generate better docs
    pub struct EventListener<T = ()> {
        #[pin]
        listener: Listener<T, Arc<Inner<T>>>,
    }
}

unsafe impl<T: Send> Send for EventListener<T> {}
unsafe impl<T: Send> Sync for EventListener<T> {}

impl<T> core::panic::UnwindSafe for EventListener<T> {}
impl<T> core::panic::RefUnwindSafe for EventListener<T> {}

impl<T> Default for EventListener<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for EventListener<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventListener")
            .field("listening", &self.is_listening())
            .finish()
    }
}

impl<T> EventListener<T> {
    /// Create a new `EventListener` that will wait for a notification from the given [`Event`].
    ///
    /// This function does not register the `EventListener` into the linked list of listeners
    /// contained within the [`Event`]. Make sure to call `listen` before `await`ing on
    /// this future or calling `wait()`.
    ///
    /// ## Examples
    ///
    /// ```
    /// use event_listener::{Event, EventListener};
    ///
    /// let event = Event::new();
    /// let listener = EventListener::new();
    ///
    /// // Make sure that the listener is pinned and listening before doing anything else.
    /// let mut listener = Box::pin(listener);
    /// listener.as_mut().listen(&event);
    /// ```
    pub fn new() -> Self {
        Self {
            listener: Listener {
                event: None,
                listener: None,
            },
        }
    }

    /// Register this listener into the given [`Event`].
    ///
    /// This method can only be called after the listener has been pinned, and must be called before
    /// the listener is polled.
    ///
    /// Notifications that exist when this function is called will be discarded.
    pub fn listen(mut self: Pin<&mut Self>, event: &Event<T>) {
        let inner = {
            let inner = event.inner();
            unsafe { Arc::clone(&ManuallyDrop::new(Arc::from_raw(inner))) }
        };

        let ListenerProject {
            event,
            mut listener,
        } = self.as_mut().project().listener.project();

        // If an event is already registered, make sure to remove it.
        if let Some(current_event) = event.as_ref() {
            current_event.remove(listener.as_mut(), false);
        }

        let inner = event.insert(inner);
        inner.insert(listener);

        // Make sure the listener is registered before whatever happens next.
        notify::full_fence();
    }

    /// Tell if this [`EventListener`] is currently listening for a notification.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::{Event, EventListener};
    ///
    /// let event = Event::new();
    /// let mut listener = Box::pin(EventListener::new());
    ///
    /// // The listener starts off not listening.
    /// assert!(!listener.is_listening());
    ///
    /// // After listen() is called, the listener is listening.
    /// listener.as_mut().listen(&event);
    /// assert!(listener.is_listening());
    ///
    /// // Once the future is notified, the listener is no longer listening.
    /// event.notify(1);
    /// listener.as_mut().wait();
    /// assert!(!listener.is_listening());
    /// ```
    pub fn is_listening(&self) -> bool {
        self.listener.listener.is_some()
    }

    /// Blocks until a notification is received.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let mut listener = event.listen();
    ///
    /// // Notify `listener`.
    /// event.notify(1);
    ///
    /// // Receive the notification.
    /// listener.as_mut().wait();
    /// ```
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    pub fn wait(self: Pin<&mut Self>) -> T {
        self.listener().wait_internal(None).unwrap()
    }

    /// Blocks until a notification is received or a timeout is reached.
    ///
    /// Returns `true` if a notification was received.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let mut listener = event.listen();
    ///
    /// // There are no notification so this times out.
    /// assert!(listener.as_mut().wait_timeout(Duration::from_secs(1)).is_none());
    /// ```
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    pub fn wait_timeout(self: Pin<&mut Self>, timeout: Duration) -> Option<T> {
        self.listener()
            .wait_internal(Instant::now().checked_add(timeout))
    }

    /// Blocks until a notification is received or a deadline is reached.
    ///
    /// Returns `true` if a notification was received.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::{Duration, Instant};
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let mut listener = event.listen();
    ///
    /// // There are no notification so this times out.
    /// assert!(listener.as_mut().wait_deadline(Instant::now() + Duration::from_secs(1)).is_none());
    /// ```
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    pub fn wait_deadline(self: Pin<&mut Self>, deadline: Instant) -> Option<T> {
        self.listener().wait_internal(Some(deadline))
    }

    /// Drops this listener and discards its notification (if any) without notifying another
    /// active listener.
    ///
    /// Returns `true` if a notification was discarded.
    ///
    /// # Examples
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let mut listener1 = event.listen();
    /// let mut listener2 = event.listen();
    ///
    /// event.notify(1);
    ///
    /// assert!(listener1.as_mut().discard());
    /// assert!(!listener2.as_mut().discard());
    /// ```
    pub fn discard(self: Pin<&mut Self>) -> bool {
        self.project().listener.discard()
    }

    /// Returns `true` if this listener listens to the given `Event`.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let listener = event.listen();
    ///
    /// assert!(listener.listens_to(&event));
    /// ```
    #[inline]
    pub fn listens_to(&self, event: &Event<T>) -> bool {
        if let Some(inner) = &self.listener.event {
            return ptr::eq::<Inner<T>>(&**inner, event.inner.load(Ordering::Acquire));
        }

        false
    }

    /// Returns `true` if both listeners listen to the same `Event`.
    ///
    /// # Examples
    ///
    /// ```
    /// use event_listener::Event;
    ///
    /// let event = Event::new();
    /// let listener1 = event.listen();
    /// let listener2 = event.listen();
    ///
    /// assert!(listener1.same_event(&listener2));
    /// ```
    pub fn same_event(&self, other: &EventListener<T>) -> bool {
        if let (Some(inner1), Some(inner2)) = (self.inner(), other.inner()) {
            return ptr::eq::<Inner<T>>(&**inner1, &**inner2);
        }

        false
    }

    fn listener(self: Pin<&mut Self>) -> Pin<&mut Listener<T, Arc<Inner<T>>>> {
        self.project().listener
    }

    fn inner(&self) -> Option<&Arc<Inner<T>>> {
        self.listener.event.as_ref()
    }
}

impl<T> Future for EventListener<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.listener().poll_internal(cx)
    }
}

pin_project_lite::pin_project! {
    #[project(!Unpin)]
    #[project = ListenerProject]
    struct Listener<T, B: Borrow<Inner<T>>>
    where
        B: Unpin,
    {
        // The reference to the original event.
        event: Option<B>,

        // The inner state of the listener.
        //
        // This is only ever `None` during initialization. After `listen()` has completed, this
        // should be `Some`.
        #[pin]
        listener: Option<sys::Listener<T>>,
    }

    impl<T, B: Borrow<Inner<T>>> PinnedDrop for Listener<T, B>
    where
        B: Unpin,
    {
        fn drop(mut this: Pin<&mut Self>) {
            // If we're being dropped, we need to remove ourself from the list.
            let this = this.project();
            if let Some(inner) = this.event {
                (*inner).borrow().remove(this.listener, true);
            }
        }
    }
}

unsafe impl<T: Send, B: Borrow<Inner<T>> + Unpin + Send> Send for Listener<T, B> {}
unsafe impl<T: Send, B: Borrow<Inner<T>> + Unpin + Sync> Sync for Listener<T, B> {}

impl<T, B: Borrow<Inner<T>> + Unpin> Listener<T, B> {
    /// Wait until the provided deadline.
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    fn wait_internal(mut self: Pin<&mut Self>, deadline: Option<Instant>) -> Option<T> {
        use std::cell::RefCell;

        std::thread_local! {
            /// Cached thread-local parker/unparker pair.
            static PARKER: RefCell<Option<(Parker, Task)>> = RefCell::new(None);
        }

        // Try to borrow the thread-local parker/unparker pair.
        PARKER
            .try_with({
                let this = self.as_mut();
                |parker| {
                    let mut pair = parker
                        .try_borrow_mut()
                        .expect("Shouldn't be able to borrow parker reentrantly");
                    let (parker, unparker) = pair.get_or_insert_with(|| {
                        let (parker, unparker) = parking::pair();
                        (parker, Task::Unparker(unparker))
                    });

                    this.wait_with_parker(deadline, parker, unparker.as_task_ref())
                }
            })
            .unwrap_or_else(|_| {
                // If the pair isn't accessible, we may be being called in a destructor.
                // Just create a new pair.
                let (parker, unparker) = parking::pair();
                self.wait_with_parker(deadline, &parker, TaskRef::Unparker(&unparker))
            })
    }

    /// Wait until the provided deadline using the specified parker/unparker pair.
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    fn wait_with_parker(
        self: Pin<&mut Self>,
        deadline: Option<Instant>,
        parker: &Parker,
        unparker: TaskRef<'_>,
    ) -> Option<T> {
        let mut this = self.project();
        let inner = (*this
            .event
            .as_ref()
            .expect("must listen() on event listener before waiting"))
        .borrow();

        // Set the listener's state to `Task`.
        if let Some(tag) = inner.register(this.listener.as_mut(), unparker).notified() {
            // We were already notified, so we don't need to park.
            return Some(tag);
        }

        // Wait until a notification is received or the timeout is reached.
        loop {
            match deadline {
                None => parker.park(),

                Some(deadline) => {
                    // Make sure we're not timed out already.
                    let now = Instant::now();
                    if now >= deadline {
                        // Remove our entry and check if we were notified.
                        return inner
                            .remove(this.listener, false)
                            .expect("We never removed ourself from the list")
                            .notified();
                    }
                    parker.park_deadline(deadline);
                }
            }

            // See if we were notified.
            if let Some(tag) = inner.register(this.listener.as_mut(), unparker).notified() {
                return Some(tag);
            }
        }
    }

    /// Drops this listener and discards its notification (if any) without notifying another
    /// active listener.
    fn discard(self: Pin<&mut Self>) -> bool {
        let this = self.project();

        if let Some(inner) = this.event.as_ref() {
            (*inner)
                .borrow()
                .remove(this.listener, false)
                .map_or(false, |state| state.is_notified())
        } else {
            false
        }
    }

    /// Poll this listener for a notification.
    fn poll_internal(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut this = self.project();
        let inner = match &this.event {
            Some(inner) => (*inner).borrow(),
            None => panic!(""),
        };

        // Try to register the listener.
        match inner
            .register(this.listener.as_mut(), TaskRef::Waker(cx.waker()))
            .notified()
        {
            Some(tag) => {
                // We were already notified, so we don't need to park.
                Poll::Ready(tag)
            }

            None => {
                // We're now waiting for a notification.
                Poll::Pending
            }
        }
    }
}

/// The state of a listener.
#[derive(PartialEq)]
enum State<T> {
    /// The listener was just created.
    Created,

    /// The listener has received a notification.
    ///
    /// The `bool` is `true` if this was an "additional" notification.
    Notified {
        /// Whether or not this is an "additional" notification.
        additional: bool,

        /// The tag associated with the notification.
        tag: T,
    },

    /// A task is waiting for a notification.
    Task(Task),

    /// Empty hole used to replace a notified listener.
    NotifiedTaken,
}

impl<T> fmt::Debug for State<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => f.write_str("Created"),
            Self::Notified { additional, .. } => f
                .debug_struct("Notified")
                .field("additional", additional)
                .finish(),
            Self::Task(_) => f.write_str("Task(_)"),
            Self::NotifiedTaken => f.write_str("NotifiedTaken"),
        }
    }
}

impl<T> State<T> {
    fn is_notified(&self) -> bool {
        matches!(self, Self::Notified { .. } | Self::NotifiedTaken)
    }

    /// If this state was notified, return the tag associated with the notification.
    #[allow(unused)]
    fn notified(self) -> Option<T> {
        match self {
            Self::Notified { tag, .. } => Some(tag),
            Self::NotifiedTaken => panic!("listener was already notified but taken"),
            _ => None,
        }
    }
}

/// The result of registering a listener.
#[derive(Debug, PartialEq)]
enum RegisterResult<T> {
    /// The listener was already notified.
    Notified(T),

    /// The listener has been registered.
    Registered,

    /// The listener was never inserted into the list.
    NeverInserted,
}

impl<T> RegisterResult<T> {
    /// Whether or not the listener was notified.
    ///
    /// Panics if the listener was never inserted into the list.
    fn notified(self) -> Option<T> {
        match self {
            Self::Notified(tag) => Some(tag),
            Self::Registered => None,
            Self::NeverInserted => panic!("listener was never inserted into the list"),
        }
    }
}

/// A task that can be woken up.
#[derive(Debug, Clone)]
enum Task {
    /// A waker that wakes up a future.
    Waker(Waker),

    /// An unparker that wakes up a thread.
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    Unparker(Unparker),
}

impl Task {
    fn as_task_ref(&self) -> TaskRef<'_> {
        match self {
            Self::Waker(waker) => TaskRef::Waker(waker),
            #[cfg(all(feature = "std", not(target_family = "wasm")))]
            Self::Unparker(unparker) => TaskRef::Unparker(unparker),
        }
    }

    fn wake(self) {
        match self {
            Self::Waker(waker) => waker.wake(),
            #[cfg(all(feature = "std", not(target_family = "wasm")))]
            Self::Unparker(unparker) => {
                unparker.unpark();
            }
        }
    }
}

impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.as_task_ref().will_wake(other.as_task_ref())
    }
}

/// A reference to a task.
#[derive(Clone, Copy)]
enum TaskRef<'a> {
    /// A waker that wakes up a future.
    Waker(&'a Waker),

    /// An unparker that wakes up a thread.
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    Unparker(&'a Unparker),
}

impl TaskRef<'_> {
    /// Tells if this task will wake up the other task.
    #[allow(unreachable_patterns)]
    fn will_wake(self, other: Self) -> bool {
        match (self, other) {
            (Self::Waker(a), Self::Waker(b)) => a.will_wake(b),
            #[cfg(all(feature = "std", not(target_family = "wasm")))]
            (Self::Unparker(_), Self::Unparker(_)) => {
                // TODO: Use unreleased will_unpark API.
                false
            }
            _ => false,
        }
    }

    /// Converts this task reference to a task by cloning.
    fn into_task(self) -> Task {
        match self {
            Self::Waker(waker) => Task::Waker(waker.clone()),
            #[cfg(all(feature = "std", not(target_family = "wasm")))]
            Self::Unparker(unparker) => Task::Unparker(unparker.clone()),
        }
    }
}

/// Synchronization primitive implementation.
mod sync {
    pub(super) use core::cell;

    #[cfg(not(feature = "portable-atomic"))]
    pub(super) use alloc::sync::Arc;
    #[cfg(not(feature = "portable-atomic"))]
    pub(super) use core::sync::atomic;

    #[cfg(feature = "portable-atomic")]
    pub(super) use portable_atomic_crate as atomic;
    #[cfg(feature = "portable-atomic")]
    pub(super) use portable_atomic_util::Arc;

    #[cfg(feature = "std")]
    pub(super) use std::sync::{Mutex, MutexGuard};

    pub(super) trait WithMut {
        type Output;

        fn with_mut<F, R>(&mut self, f: F) -> R
        where
            F: FnOnce(&mut Self::Output) -> R;
    }

    impl<T> WithMut for atomic::AtomicPtr<T> {
        type Output = *mut T;

        #[inline]
        fn with_mut<F, R>(&mut self, f: F) -> R
        where
            F: FnOnce(&mut Self::Output) -> R,
        {
            f(self.get_mut())
        }
    }
}

fn __test_send_and_sync() {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}

    _assert_send::<Event<()>>();
    _assert_sync::<Event<()>>();
    _assert_send::<EventListener<()>>();
    _assert_sync::<EventListener<()>>();
}
