use alloy_primitives::map::foldhash::fast::FixedState;
use hashbrown::{HashTable, hash_table::Entry};
use std::{
    cmp::Ordering,
    fmt,
    hash::{BuildHasher, Hash, Hasher},
    sync::{Arc, Weak},
};

/// Shared handle for a hash-consed value.
///
/// Equality is pointer equality only. Hashing writes the cached structural hash
/// instead of walking the value.
pub(in crate::runtime) struct HashConsed<T> {
    inner: Arc<HashConsedInner<T>>,
}

struct HashConsedInner<T> {
    hash: u64,
    value: T,
}

impl<T> HashConsed<T> {
    #[inline]
    pub(in crate::runtime::expr) fn stable_hash_cmp(&self, other: &Self) -> Ordering {
        self.inner.hash.cmp(&other.inner.hash)
    }

    /// Orders nodes within one hash-consing context without inspecting or rendering their value.
    ///
    /// The cached structural hash handles the common case. Pointer identity is only a tie-breaker
    /// for distinct nodes with the same hash; structurally equal values share one node.
    #[inline]
    pub(in crate::runtime::expr) fn identity_cmp(&self, other: &Self) -> Ordering {
        self.inner.hash.cmp(&other.inner.hash).then_with(|| {
            let left = Arc::as_ptr(&self.inner);
            let right = Arc::as_ptr(&other.inner);
            left.cmp(&right)
        })
    }

    #[inline]
    pub(in crate::runtime) fn value(&self) -> &T {
        &self.inner.value
    }

    #[inline]
    pub(in crate::runtime) fn into_value(self) -> T
    where
        T: Clone,
    {
        match Arc::try_unwrap(self.inner) {
            Ok(inner) => inner.value,
            Err(inner) => inner.value.clone(),
        }
    }
}

impl<T> Clone for HashConsed<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<T> PartialEq for HashConsed<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T> Eq for HashConsed<T> {}

impl<T> Hash for HashConsed<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash.hash(state);
    }
}

impl<T: PartialOrd> PartialOrd for HashConsed<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value().partial_cmp(other.value())
    }
}

impl<T: Ord> Ord for HashConsed<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value().cmp(other.value())
    }
}

impl<T: fmt::Debug> fmt::Debug for HashConsed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}

type HashConsHasher = FixedState;
const MIN_GC_THRESHOLD: usize = 1024;

/// Hash-consing table for sharing structurally equal immutable values.
///
/// The table stores weak references so interned values disappear when the rest of
/// the symbolic state stops using them. `make` removes dead entries encountered
/// during lookup and periodically sweeps distinct dead values before they can
/// grow the table without bound.
pub(in crate::runtime) struct HashCons<T> {
    table: HashTable<HashConsEntry<T>>,
    hash_builder: HashConsHasher,
    gc_threshold: usize,
}

struct HashConsEntry<T> {
    hash: u64,
    value: Weak<HashConsedInner<T>>,
}

impl<T> HashConsEntry<T> {
    const fn hash(&self) -> u64 {
        self.hash
    }
}

impl<T> HashCons<T> {
    pub(in crate::runtime) fn new() -> Self {
        Self {
            table: HashTable::new(),
            hash_builder: HashConsHasher::default(),
            gc_threshold: MIN_GC_THRESHOLD,
        }
    }

    fn hash<Q: Hash + ?Sized>(&self, value: &Q) -> u64 {
        self.hash_builder.hash_one(value)
    }
}

impl<T: Eq + Hash> HashCons<T> {
    pub(in crate::runtime) fn make(&mut self, value: T) -> HashConsed<T> {
        if self.table.len() >= self.gc_threshold {
            self.table.retain(|entry| entry.value.strong_count() != 0);
            self.gc_threshold = self.table.len().saturating_mul(2).max(MIN_GC_THRESHOLD);
        }

        let hash = self.hash(&value);
        loop {
            let mut found = None;
            let mut matched_dead_entry = false;
            match self.table.entry(
                hash,
                |entry| {
                    if entry.hash != hash {
                        return false;
                    }
                    match entry.value.upgrade() {
                        Some(existing) if existing.value == value => {
                            found = Some(existing);
                            true
                        }
                        None => {
                            matched_dead_entry = true;
                            true
                        }
                        Some(_) => false,
                    }
                },
                HashConsEntry::hash,
            ) {
                Entry::Occupied(entry) => {
                    if let Some(inner) = found {
                        return HashConsed { inner };
                    }
                    debug_assert!(matched_dead_entry);
                    let _ = entry.remove();
                }
                Entry::Vacant(entry) => {
                    let inner = Arc::new(HashConsedInner { hash, value });
                    entry.insert(HashConsEntry { hash, value: Arc::downgrade(&inner) });
                    return HashConsed { inner };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_reuses_existing_value() {
        let mut table = HashCons::<String>::new();

        let first = table.make("same".to_string());
        let second = table.make("same".to_string());

        assert_eq!(first, second);
        assert_eq!(first.inner.hash, second.inner.hash);
    }

    #[test]
    fn make_keeps_distinct_values_apart() {
        let mut table = HashCons::<String>::new();

        let first = table.make("first".to_string());
        let second = table.make("second".to_string());

        assert_ne!(first, second);
    }

    #[test]
    fn dropped_values_are_not_reused() {
        let mut table = HashCons::<String>::new();

        let first = table.make("same".to_string());
        let weak = Arc::downgrade(&first.inner);
        drop(first);
        assert!(weak.upgrade().is_none());

        let second = table.make("same".to_string());

        assert_eq!(second.value().as_str(), "same");
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn make_reclaims_repeatedly_dropped_values() {
        let mut table = HashCons::<String>::new();

        for _ in 0..128 {
            drop(table.make("same".to_string()));
        }

        assert_eq!(table.table.len(), 1);
    }

    #[test]
    fn make_reclaims_distinct_dropped_values() {
        let mut table = HashCons::<String>::new();
        let retained = table.make("retained".to_string());

        for value in 0..MIN_GC_THRESHOLD - 1 {
            drop(table.make(value.to_string()));
        }
        let same = table.make("retained".to_string());

        assert_eq!(table.table.len(), 1);
        assert_eq!(retained, same);
    }

    #[test]
    fn equality_is_pointer_only() {
        let mut first_table = HashCons::<String>::new();
        let mut second_table = HashCons::<String>::new();

        let first = first_table.make("same".to_string());
        let second = second_table.make("same".to_string());

        assert_ne!(first, second);
        assert_eq!(first.value(), second.value());
    }
}
