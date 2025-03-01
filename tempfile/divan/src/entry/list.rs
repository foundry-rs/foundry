use std::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering as AtomicOrdering},
};

/// Linked list of entries.
///
/// This is implemented in a thread-safe way despite the fact that constructors
/// are run single-threaded.
pub struct EntryList<T: 'static> {
    entry: Option<&'static T>,
    next: AtomicPtr<Self>,
}

impl<T> EntryList<T> {
    pub(crate) const fn root() -> Self {
        Self { entry: None, next: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Dereferences the `next` pointer.
    #[inline]
    fn next(&self) -> Option<&Self> {
        // SAFETY: `next` is only assigned by `push`, which always receives a
        // 'static lifetime.
        unsafe { self.next.load(AtomicOrdering::Relaxed).as_ref() }
    }
}

// Externally used by macros or tests.
#[allow(missing_docs)]
impl<T> EntryList<T> {
    #[inline]
    pub const fn new(entry: &'static T) -> Self {
        Self { entry: Some(entry), next: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Creates an iterator over entries in `self`.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let mut list = Some(self);
        std::iter::from_fn(move || -> Option<Option<&T>> {
            let current = list?;
            list = current.next();
            Some(current.entry.as_ref().copied())
        })
        .flatten()
    }

    /// Inserts `other` to the front of the list.
    ///
    /// # Safety
    ///
    /// This function must be safe to call before `main`.
    #[inline]
    pub fn push(&'static self, other: &'static Self) {
        let mut old_next = self.next.load(AtomicOrdering::Relaxed);
        loop {
            // Each publicly-created instance has `list.next` be null, so we can
            // simply store `self.next` there.
            other.next.store(old_next, AtomicOrdering::Release);

            // SAFETY: The content of `other` can already be seen, so we don't
            // need to strongly order reads into it.
            let other = other as *const Self as *mut Self;
            match self.next.compare_exchange_weak(
                old_next,
                other,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            ) {
                // Successfully wrote our thread's value to the list.
                Ok(_) => return,

                // Lost the race, store winner's value in `other.next`.
                Err(new) => old_next = new,
            }
        }
    }
}
