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
    /// Summary bits of the whole subtree, computed once from the children's bits.
    flags: u8,
    value: T,
}

/// Summary bits a hash-consed value derives from its own kind and its children's bits, so
/// subtree queries such as "contains an unsigned division" take constant time.
pub(in crate::runtime) trait NodeFlags {
    /// Returns this node's bits, including those of every child.
    fn flags(&self) -> u8 {
        0
    }
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
        self.stable_hash_cmp(other).then_with(|| {
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
    pub(in crate::runtime) fn flags(&self) -> u8 {
        self.inner.flags
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

const MIN_GC_THRESHOLD: usize = 1024;

/// Hash-consing table for sharing structurally equal immutable values.
///
/// The table stores weak references so interned values disappear when the rest of
/// the symbolic state stops using them. `make` removes dead entries encountered
/// during lookup and periodically sweeps distinct dead values before they can
/// grow the table without bound.
pub(in crate::runtime) struct HashCons<T> {
    table: HashTable<HashConsEntry<T>>,
    hash_builder: FixedState,
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
            hash_builder: FixedState::default(),
            gc_threshold: MIN_GC_THRESHOLD,
        }
    }

    fn hash<Q: Hash + ?Sized>(&self, value: &Q) -> u64 {
        self.hash_builder.hash_one(value)
    }
}

impl<T: Eq + Hash + NodeFlags> HashCons<T> {
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
                    if entry.hash() != hash {
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
                    let flags = value.flags();
                    let inner = Arc::new(HashConsedInner { hash, flags, value });
                    entry.insert(HashConsEntry { hash, value: Arc::downgrade(&inner) });
                    return HashConsed { inner };
                }
            }
        }
    }
}
