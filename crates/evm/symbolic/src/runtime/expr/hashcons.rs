use alloy_primitives::map::foldhash::fast::FixedState;
use hashbrown::{HashTable, hash_table::Entry};
use std::{
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

pub(in crate::runtime::expr) type HashConsHasher = FixedState;

/// Hash-consing table for sharing structurally equal immutable values.
///
/// The table stores weak references so interned values disappear when the rest of
/// the symbolic state stops using them. `make` only looks up and inserts; dead
/// weak entries are ignored and left in the table until the context is dropped.
pub(in crate::runtime) struct HashCons<T, S = HashConsHasher> {
    table: HashTable<HashConsEntry<T>>,
    hash_builder: S,
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
        Self::with_hasher(HashConsHasher::default())
    }
}

impl<T, S> HashCons<T, S> {
    pub(in crate::runtime) const fn with_hasher(hash_builder: S) -> Self {
        Self { table: HashTable::new(), hash_builder }
    }
}

impl<T, S: BuildHasher> HashCons<T, S> {
    fn hash<Q: Hash + ?Sized>(&self, value: &Q) -> u64 {
        self.hash_builder.hash_one(value)
    }
}

impl<T: Eq + Hash, S: BuildHasher> HashCons<T, S> {
    pub(in crate::runtime) fn make(&mut self, value: T) -> HashConsed<T> {
        let hash = self.hash(&value);
        let mut found = None;
        match self.table.entry(
            hash,
            |entry| {
                if entry.hash == hash
                    && let Some(existing) = entry.value.upgrade()
                    && existing.value == value
                {
                    found = Some(existing);
                    true
                } else {
                    false
                }
            },
            HashConsEntry::hash,
        ) {
            Entry::Occupied(_) => HashConsed { inner: found.expect("matched live value") },
            Entry::Vacant(entry) => {
                let inner = HashConsedInner { hash, value };
                let inner = Arc::new(inner);
                entry.insert(HashConsEntry { hash, value: Arc::downgrade(&inner) });
                HashConsed { inner }
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
    fn equality_is_pointer_only() {
        let mut first_table = HashCons::<String>::new();
        let mut second_table = HashCons::<String>::new();

        let first = first_table.make("same".to_string());
        let second = second_table.make("same".to_string());

        assert_ne!(first, second);
        assert_eq!(first.value(), second.value());
    }
}
