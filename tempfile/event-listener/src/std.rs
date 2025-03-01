//! libstd-based implementation of `event-listener`.
//!
//! This implementation crates an intrusive linked list of listeners.

use crate::notify::{GenericNotify, Internal, Notification};
use crate::sync::atomic::Ordering;
use crate::sync::cell::{Cell, UnsafeCell};
use crate::sync::{Mutex, MutexGuard};
use crate::{RegisterResult, State, TaskRef};

use core::marker::PhantomPinned;
use core::mem;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::ptr::NonNull;

pub(super) struct List<T>(Mutex<Inner<T>>);

struct Inner<T> {
    /// The head of the linked list.
    head: Option<NonNull<Link<T>>>,

    /// The tail of the linked list.
    tail: Option<NonNull<Link<T>>>,

    /// The first unnotified listener.
    next: Option<NonNull<Link<T>>>,

    /// Total number of listeners.
    len: usize,

    /// The number of notified listeners.
    notified: usize,
}

impl<T> List<T> {
    /// Create a new, empty event listener list.
    pub(super) fn new() -> Self {
        Self(Mutex::new(Inner {
            head: None,
            tail: None,
            next: None,
            len: 0,
            notified: 0,
        }))
    }
    // Accessor method because fields are private, not sure how to go around it
    pub fn total_listeners(&self) -> Result<usize, &str> {
        match self.0.try_lock() {
            Ok(mutex) => {
                let len = mutex.len;
                Ok(len)
            }
            Err(_) => Err("<locked>"),
        }
    }
}

impl<T> crate::Inner<T> {
    fn lock(&self) -> ListLock<'_, '_, T> {
        ListLock {
            inner: self,
            lock: self.list.0.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }

    /// Whether or not this number of listeners would lead to a notification.
    pub(crate) fn needs_notification(&self, limit: usize) -> bool {
        self.notified.load(Ordering::Acquire) < limit
    }

    /// Add a new listener to the list.
    pub(crate) fn insert(&self, mut listener: Pin<&mut Option<Listener<T>>>) {
        let mut inner = self.lock();

        // SAFETY: We are locked, so we can access the inner `link`.
        let entry = unsafe {
            listener.as_mut().set(Some(Listener {
                link: UnsafeCell::new(Link {
                    state: Cell::new(State::Created),
                    prev: Cell::new(inner.tail),
                    next: Cell::new(None),
                }),
                _pin: PhantomPinned,
            }));
            let listener = listener.as_pin_mut().unwrap();

            // Get the inner pointer.
            &*listener.link.get()
        };

        // Replace the tail with the new entry.
        match mem::replace(&mut inner.tail, Some(entry.into())) {
            None => inner.head = Some(entry.into()),
            Some(t) => unsafe { t.as_ref().next.set(Some(entry.into())) },
        };

        // If there are no unnotified entries, this is the first one.
        if inner.next.is_none() {
            inner.next = inner.tail;
        }

        // Bump the entry count.
        inner.len += 1;
    }

    /// Remove a listener from the list.
    pub(crate) fn remove(
        &self,
        listener: Pin<&mut Option<Listener<T>>>,
        propagate: bool,
    ) -> Option<State<T>> {
        self.lock().remove(listener, propagate)
    }

    /// Notifies a number of entries.
    #[cold]
    pub(crate) fn notify(&self, notify: impl Notification<Tag = T>) -> usize {
        self.lock().notify(notify)
    }

    /// Register a task to be notified when the event is triggered.
    ///
    /// Returns `true` if the listener was already notified, and `false` otherwise. If the listener
    /// isn't inserted, returns `None`.
    pub(crate) fn register(
        &self,
        mut listener: Pin<&mut Option<Listener<T>>>,
        task: TaskRef<'_>,
    ) -> RegisterResult<T> {
        let mut inner = self.lock();

        // SAFETY: We are locked, so we can access the inner `link`.
        let entry = unsafe {
            let listener = match listener.as_mut().as_pin_mut() {
                Some(listener) => listener,
                None => return RegisterResult::NeverInserted,
            };
            &*listener.link.get()
        };

        // Take out the state and check it.
        match entry.state.replace(State::NotifiedTaken) {
            State::Notified { tag, .. } => {
                // We have been notified, remove the listener.
                inner.remove(listener, false);
                RegisterResult::Notified(tag)
            }

            State::Task(other_task) => {
                // Only replace the task if it's different.
                entry.state.set(State::Task({
                    if !task.will_wake(other_task.as_task_ref()) {
                        task.into_task()
                    } else {
                        other_task
                    }
                }));

                RegisterResult::Registered
            }

            _ => {
                // We have not been notified, register the task.
                entry.state.set(State::Task(task.into_task()));
                RegisterResult::Registered
            }
        }
    }
}

impl<T> Inner<T> {
    fn remove(
        &mut self,
        mut listener: Pin<&mut Option<Listener<T>>>,
        propagate: bool,
    ) -> Option<State<T>> {
        let entry = unsafe {
            let listener = listener.as_mut().as_pin_mut()?;

            // Get the inner pointer.
            &*listener.link.get()
        };

        let prev = entry.prev.get();
        let next = entry.next.get();

        // Unlink from the previous entry.
        match prev {
            None => self.head = next,
            Some(p) => unsafe {
                p.as_ref().next.set(next);
            },
        }

        // Unlink from the next entry.
        match next {
            None => self.tail = prev,
            Some(n) => unsafe {
                n.as_ref().prev.set(prev);
            },
        }

        // If this was the first unnotified entry, update the next pointer.
        if self.next == Some(entry.into()) {
            self.next = next;
        }

        // The entry is now fully unlinked, so we can now take it out safely.
        let entry = unsafe {
            listener
                .get_unchecked_mut()
                .take()
                .unwrap()
                .link
                .into_inner()
        };

        let mut state = entry.state.into_inner();

        // Update the notified count.
        if state.is_notified() {
            self.notified -= 1;

            if propagate {
                let state = mem::replace(&mut state, State::NotifiedTaken);
                if let State::Notified { additional, tag } = state {
                    let tags = {
                        let mut tag = Some(tag);
                        move || tag.take().expect("tag already taken")
                    };
                    self.notify(GenericNotify::new(1, additional, tags));
                }
            }
        }
        self.len -= 1;

        Some(state)
    }

    #[cold]
    fn notify(&mut self, mut notify: impl Notification<Tag = T>) -> usize {
        let mut n = notify.count(Internal::new());
        let is_additional = notify.is_additional(Internal::new());

        if !is_additional {
            if n < self.notified {
                return 0;
            }
            n -= self.notified;
        }

        let original_count = n;
        while n > 0 {
            n -= 1;

            // Notify the next entry.
            match self.next {
                None => return original_count - n - 1,

                Some(e) => {
                    // Get the entry and move the pointer forwards.
                    let entry = unsafe { e.as_ref() };
                    self.next = entry.next.get();

                    // Set the state to `Notified` and notify.
                    let tag = notify.next_tag(Internal::new());
                    if let State::Task(task) = entry.state.replace(State::Notified {
                        additional: is_additional,
                        tag,
                    }) {
                        task.wake();
                    }

                    // Bump the notified count.
                    self.notified += 1;
                }
            }
        }

        original_count - n
    }
}

struct ListLock<'a, 'b, T> {
    lock: MutexGuard<'a, Inner<T>>,
    inner: &'b crate::Inner<T>,
}

impl<T> Deref for ListLock<'_, '_, T> {
    type Target = Inner<T>;

    fn deref(&self) -> &Self::Target {
        &self.lock
    }
}

impl<T> DerefMut for ListLock<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.lock
    }
}

impl<T> Drop for ListLock<'_, '_, T> {
    fn drop(&mut self) {
        let list = &mut **self;

        // Update the notified count.
        let notified = if list.notified < list.len {
            list.notified
        } else {
            core::usize::MAX
        };

        self.inner.notified.store(notified, Ordering::Release);
    }
}

pub(crate) struct Listener<T> {
    /// The inner link in the linked list.
    ///
    /// # Safety
    ///
    /// This can only be accessed while the central mutex is locked.
    link: UnsafeCell<Link<T>>,

    /// This listener cannot be moved after being pinned.
    _pin: PhantomPinned,
}

struct Link<T> {
    /// The current state of the listener.
    state: Cell<State<T>>,

    /// The previous link in the linked list.
    prev: Cell<Option<NonNull<Link<T>>>>,

    /// The next link in the linked list.
    next: Cell<Option<NonNull<Link<T>>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::pin;

    #[cfg(target_family = "wasm")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    macro_rules! make_listeners {
        ($($id:ident),*) => {
            $(
                let $id = Option::<Listener<()>>::None;
                pin!($id);
            )*
        };
    }

    #[test]
    fn insert() {
        let inner = crate::Inner::new();
        make_listeners!(listen1, listen2, listen3);

        // Register the listeners.
        inner.insert(listen1.as_mut());
        inner.insert(listen2.as_mut());
        inner.insert(listen3.as_mut());

        assert_eq!(inner.lock().len, 3);

        // Remove one.
        assert_eq!(inner.remove(listen2, false), Some(State::Created));
        assert_eq!(inner.lock().len, 2);

        // Remove another.
        assert_eq!(inner.remove(listen1, false), Some(State::Created));
        assert_eq!(inner.lock().len, 1);
    }

    #[test]
    fn drop_non_notified() {
        let inner = crate::Inner::new();
        make_listeners!(listen1, listen2, listen3);

        // Register the listeners.
        inner.insert(listen1.as_mut());
        inner.insert(listen2.as_mut());
        inner.insert(listen3.as_mut());

        // Notify one.
        inner.notify(GenericNotify::new(1, false, || ()));

        // Remove one.
        inner.remove(listen3, true);

        // Remove the rest.
        inner.remove(listen1, true);
        inner.remove(listen2, true);
    }
}
