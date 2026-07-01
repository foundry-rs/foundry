use hashbrown::{HashTable, hash_table::Entry};
use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{BuildHasher, BuildHasherDefault, Hash, Hasher},
    sync::{Arc, Weak},
};

type HashBuilder = BuildHasherDefault<DefaultHasher>;

/// Shared handle for a hash-consed value.
///
/// Equality first checks whether both handles point at the same allocation,
/// falling back to structural equality for values built by different contexts.
/// Hashing writes the cached structural hash instead of walking the value.
#[derive(Clone)]
pub(crate) struct HashConsed<T> {
    inner: Arc<HashConsedInner<T>>,
}

struct HashConsedInner<T> {
    hash: u64,
    value: T,
}

impl<T> HashConsed<T> {
    const fn from_inner(inner: Arc<HashConsedInner<T>>) -> Self {
        Self { inner }
    }

    fn from_value(hash: u64, value: T) -> Self {
        Self::from_inner(Arc::new(HashConsedInner { hash, value }))
    }

    fn cached_hash(&self) -> u64 {
        self.inner.hash
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn value(&self) -> &T {
        &self.inner.value
    }

    pub(crate) fn into_value(self) -> T
    where
        T: Clone,
    {
        match Arc::try_unwrap(self.inner) {
            Ok(inner) => inner.value,
            Err(inner) => inner.value.clone(),
        }
    }
}

impl<T: PartialEq> PartialEq for HashConsed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.value() == other.value()
    }
}

impl<T: Eq> Eq for HashConsed<T> {}

impl<T> Hash for HashConsed<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.cached_hash());
    }
}

impl<T: PartialOrd> PartialOrd for HashConsed<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.value().partial_cmp(other.value())
    }
}

impl<T: Ord> Ord for HashConsed<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value().cmp(other.value())
    }
}

impl<T: fmt::Debug> fmt::Debug for HashConsed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}

/// Hash-consing table for sharing structurally equal immutable values.
///
/// The table stores weak references so interned values disappear when the rest of
/// the symbolic state stops using them. `make` only looks up and inserts; dead
/// weak entries are ignored and left in the table until the context is dropped.
pub(crate) struct HashCons<T> {
    table: HashTable<HashConsEntry<T>>,
    hash_builder: HashBuilder,
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
    pub(crate) fn new() -> Self {
        Self { table: HashTable::new(), hash_builder: HashBuilder::default() }
    }

    fn hash<Q: Hash + ?Sized>(&self, value: &Q) -> u64 {
        self.hash_builder.hash_one(value)
    }
}

impl<T: Eq + Hash> HashCons<T> {
    pub(crate) fn uninterned(value: T) -> HashConsed<T> {
        let hashcons = Self::new();
        let hash = hashcons.hash(&value);
        HashConsed::from_value(hash, value)
    }

    pub(crate) fn make(&mut self, value: T) -> HashConsed<T> {
        let hash = self.hash(&value);
        let mut found = None;
        match self.table.entry(
            hash,
            |entry| {
                if entry.hash == hash
                    && let Some(existing) = entry.value.upgrade()
                    && existing.value.eq(&value)
                {
                    found = Some(existing);
                    true
                } else {
                    false
                }
            },
            HashConsEntry::hash,
        ) {
            Entry::Occupied(_) => HashConsed::from_inner(found.expect("matched live value")),
            Entry::Vacant(entry) => {
                let inner = HashConsedInner { hash, value };
                let inner = Arc::new(inner);
                entry.insert(HashConsEntry { hash, value: Arc::downgrade(&inner) });
                HashConsed::from_inner(inner)
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

        assert!(first.ptr_eq(&second));
        assert_eq!(first.cached_hash(), second.cached_hash());
    }

    #[test]
    fn make_keeps_distinct_values_apart() {
        let mut table = HashCons::<String>::new();

        let first = table.make("first".to_string());
        let second = table.make("second".to_string());

        assert!(!first.ptr_eq(&second));
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
    fn uninterned_and_interned_values_hash_the_same() {
        let mut table = HashCons::<String>::new();

        let interned = table.make("same".to_string());
        let raw = HashCons::<String>::uninterned("same".to_string());

        assert_eq!(interned, raw);
        assert_eq!(interned.cached_hash(), raw.cached_hash());
    }
}
