//! A bimap backed by two `HashMap`s.

use crate::{
    mem::{Ref, Wrapper},
    Overwritten,
};
use std::{
    borrow::Borrow,
    collections::{hash_map, HashMap},
    fmt,
    hash::{BuildHasher, Hash},
    iter::{Extend, FromIterator, FusedIterator},
    rc::Rc,
};

/// A bimap backed by two `HashMap`s.
///
/// See the [module-level documentation] for more details and examples.
///
/// [module-level documentation]: crate
pub struct BiHashMap<L, R, LS = hash_map::RandomState, RS = hash_map::RandomState> {
    left2right: HashMap<Ref<L>, Ref<R>, LS>,
    right2left: HashMap<Ref<R>, Ref<L>, RS>,
}

impl<L, R> BiHashMap<L, R, hash_map::RandomState, hash_map::RandomState>
where
    L: Eq + Hash,
    R: Eq + Hash,
{
    /// Creates an empty `BiHashMap`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let bimap = BiHashMap::<char, i32>::new();
    /// ```
    pub fn new() -> Self {
        Self {
            left2right: HashMap::new(),
            right2left: HashMap::new(),
        }
    }

    /// Creates a new empty `BiHashMap` with the given capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let bimap = BiHashMap::<char, i32>::with_capacity(10);
    /// assert!(bimap.capacity() >= 10);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            left2right: HashMap::with_capacity(capacity),
            right2left: HashMap::with_capacity(capacity),
        }
    }
}

impl<L, R, LS, RS> BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
{
    /// Returns the number of left-right pairs in the bimap.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    /// assert_eq!(bimap.len(), 3);
    /// ```
    pub fn len(&self) -> usize {
        self.left2right.len()
    }

    /// Returns `true` if the bimap contains no left-right pairs, and `false`
    /// otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// assert!(bimap.is_empty());
    /// bimap.insert('a', 1);
    /// assert!(!bimap.is_empty());
    /// bimap.remove_by_right(&1);
    /// assert!(bimap.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.left2right.is_empty()
    }

    /// Returns a lower bound on the number of left-right pairs the `BiHashMap`
    /// can store without reallocating memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let bimap = BiHashMap::<char, i32>::with_capacity(10);
    /// assert!(bimap.capacity() >= 10);
    /// ```
    pub fn capacity(&self) -> usize {
        self.left2right.capacity().min(self.right2left.capacity())
    }

    /// Removes all left-right pairs from the bimap.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    /// bimap.clear();
    /// assert!(bimap.len() == 0);
    /// ```
    pub fn clear(&mut self) {
        self.left2right.clear();
        self.right2left.clear();
    }

    /// Creates an iterator over the left-right pairs in the bimap in arbitrary
    /// order.
    ///
    /// The iterator element type is `(&L, &R)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    ///
    /// for (left, right) in bimap.iter() {
    ///     println!("({}, {})", left, right);
    /// }
    /// ```
    pub fn iter(&self) -> Iter<'_, L, R> {
        Iter {
            inner: self.left2right.iter(),
        }
    }

    /// Creates an iterator over the left values in the bimap in arbitrary
    /// order.
    ///
    /// The iterator element type is `&L`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    ///
    /// for char_value in bimap.left_values() {
    ///     println!("{}", char_value);
    /// }
    /// ```
    pub fn left_values(&self) -> LeftValues<'_, L, R> {
        LeftValues {
            inner: self.left2right.iter(),
        }
    }

    /// Creates an iterator over the right values in the bimap in arbitrary
    /// order.
    ///
    /// The iterator element type is `&R`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    ///
    /// for int_value in bimap.right_values() {
    ///     println!("{}", int_value);
    /// }
    /// ```
    pub fn right_values(&self) -> RightValues<'_, L, R> {
        RightValues {
            inner: self.right2left.iter(),
        }
    }
}

impl<L, R, LS, RS> BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
    LS: BuildHasher,
    RS: BuildHasher,
{
    /// Creates a new empty `BiHashMap` using `hash_builder_left` to hash left
    /// values and `hash_builder_right` to hash right values.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::hash_map::RandomState;
    /// use bimap::BiHashMap;
    ///
    /// let s_left = RandomState::new();
    /// let s_right = RandomState::new();
    /// let mut bimap = BiHashMap::<char, i32>::with_hashers(s_left, s_right);
    /// bimap.insert('a', 42);
    /// ```
    pub fn with_hashers(hash_builder_left: LS, hash_builder_right: RS) -> Self {
        Self {
            left2right: HashMap::with_hasher(hash_builder_left),
            right2left: HashMap::with_hasher(hash_builder_right),
        }
    }

    /// Creates a new empty `BiHashMap` with the given capacity, using
    /// `hash_builder_left` to hash left values and `hash_builder_right` to
    /// hash right values.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::hash_map::RandomState;
    /// use bimap::BiHashMap;
    ///
    /// let s_left = RandomState::new();
    /// let s_right = RandomState::new();
    /// let bimap = BiHashMap::<char, i32>::with_capacity_and_hashers(10, s_left, s_right);
    /// assert!(bimap.capacity() >= 10);
    /// ```
    pub fn with_capacity_and_hashers(
        capacity: usize,
        hash_builder_left: LS,
        hash_builder_right: RS,
    ) -> Self {
        Self {
            left2right: HashMap::with_capacity_and_hasher(capacity, hash_builder_left),
            right2left: HashMap::with_capacity_and_hasher(capacity, hash_builder_right),
        }
    }

    /// Reserves capacity for at least `additional` more elements to be inserted
    /// in the `BiHashMap`. The collection may reserve more space to avoid
    /// frequent reallocations.
    ///
    /// # Panics
    ///
    /// Panics if the new allocation size overflows [`usize`].
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::<char, i32>::new();
    /// bimap.reserve(10);
    /// assert!(bimap.capacity() >= 10);
    /// ```
    pub fn reserve(&mut self, additional: usize) {
        self.left2right.reserve(additional);
        self.right2left.reserve(additional);
    }

    /// Shrinks the capacity of the bimap as much as possible. It will drop
    /// down as much as possible while maintaining the internal rules
    /// and possibly leaving some space in accordance with the resize policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::<char, i32>::with_capacity(100);
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// assert!(bimap.capacity() >= 100);
    /// bimap.shrink_to_fit();
    /// assert!(bimap.capacity() >= 2);
    /// ```
    pub fn shrink_to_fit(&mut self) {
        self.left2right.shrink_to_fit();
        self.right2left.shrink_to_fit();
    }

    /// Shrinks the capacity of the bimap with a lower limit. It will drop
    /// down no lower than the supplied limit while maintaining the internal
    /// rules and possibly leaving some space in accordance with the resize
    /// policy.
    ///
    /// If the current capacity is less than the lower limit, this is a no-op.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::<char, i32>::with_capacity(100);
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// assert!(bimap.capacity() >= 100);
    /// bimap.shrink_to(10);
    /// assert!(bimap.capacity() >= 10);
    /// bimap.shrink_to(0);
    /// assert!(bimap.capacity() >= 2);
    /// ```
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.left2right.shrink_to(min_capacity);
        self.right2left.shrink_to(min_capacity);
    }

    /// Returns a reference to the right value corresponding to the given left
    /// value.
    ///
    /// The input may be any borrowed form of the bimap's left type, but `Eq`
    /// and `Hash` on the borrowed form *must* match those for the left type.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// assert_eq!(bimap.get_by_left(&'a'), Some(&1));
    /// assert_eq!(bimap.get_by_left(&'z'), None);
    /// ```
    pub fn get_by_left<Q>(&self, left: &Q) -> Option<&R>
    where
        L: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.left2right.get(Wrapper::wrap(left)).map(|r| &*r.0)
    }

    /// Returns a reference to the left value corresponding to the given right
    /// value.
    ///
    /// The input may be any borrowed form of the bimap's right type, but `Eq`
    /// and `Hash` on the borrowed form *must* match those for the right type.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// assert_eq!(bimap.get_by_right(&1), Some(&'a'));
    /// assert_eq!(bimap.get_by_right(&2), None);
    /// ```
    pub fn get_by_right<Q>(&self, right: &Q) -> Option<&L>
    where
        R: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.right2left.get(Wrapper::wrap(right)).map(|l| &*l.0)
    }

    /// Returns `true` if the bimap contains the given left value and `false`
    /// otherwise.
    ///
    /// The input may be any borrowed form of the bimap's left type, but `Eq`
    /// and `Hash` on the borrowed form *must* match those for the left type.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// assert!(bimap.contains_left(&'a'));
    /// assert!(!bimap.contains_left(&'b'));
    /// ```
    pub fn contains_left<Q>(&self, left: &Q) -> bool
    where
        L: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.left2right.contains_key(Wrapper::wrap(left))
    }

    /// Returns `true` if the map contains the given right value and `false`
    /// otherwise.
    ///
    /// The input may be any borrowed form of the bimap's right type, but `Eq`
    /// and `Hash` on the borrowed form *must* match those for the right type.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// assert!(bimap.contains_right(&1));
    /// assert!(!bimap.contains_right(&2));
    /// ```
    pub fn contains_right<Q>(&self, right: &Q) -> bool
    where
        R: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.right2left.contains_key(Wrapper::wrap(right))
    }

    /// Removes the left-right pair corresponding to the given left value.
    ///
    /// Returns the previous left-right pair if the map contained the left value
    /// and `None` otherwise.
    ///
    /// The input may be any borrowed form of the bimap's left type, but `Eq`
    /// and `Hash` on the borrowed form *must* match those for the left type.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    ///
    /// assert_eq!(bimap.remove_by_left(&'b'), Some(('b', 2)));
    /// assert_eq!(bimap.remove_by_left(&'b'), None);
    /// ```
    pub fn remove_by_left<Q>(&mut self, left: &Q) -> Option<(L, R)>
    where
        L: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.left2right.remove(Wrapper::wrap(left)).map(|right_rc| {
            // unwrap is safe because we know right2left contains the key (it's a bimap)
            let left_rc = self.right2left.remove(&right_rc).unwrap();
            // at this point we can safely unwrap because the other pointers are gone
            (
                Rc::try_unwrap(left_rc.0).ok().unwrap(),
                Rc::try_unwrap(right_rc.0).ok().unwrap(),
            )
        })
    }

    /// Removes the left-right pair corresponding to the given right value.
    ///
    /// Returns the previous left-right pair if the map contained the right
    /// value and `None` otherwise.
    ///
    /// The input may be any borrowed form of the bimap's right type, but `Eq`
    /// and `Hash` on the borrowed form *must* match those for the right type.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    ///
    /// assert_eq!(bimap.remove_by_right(&2), Some(('b', 2)));
    /// assert_eq!(bimap.remove_by_right(&2), None);
    /// ```
    pub fn remove_by_right<Q>(&mut self, right: &Q) -> Option<(L, R)>
    where
        R: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.right2left.remove(Wrapper::wrap(right)).map(|left_rc| {
            // unwrap is safe because we know left2right contains the key (it's a bimap)
            let right_rc = self.left2right.remove(&left_rc).unwrap();
            // at this point we can safely unwrap because the other pointers are gone
            (
                Rc::try_unwrap(left_rc.0).ok().unwrap(),
                Rc::try_unwrap(right_rc.0).ok().unwrap(),
            )
        })
    }

    /// Inserts the given left-right pair into the bimap.
    ///
    /// Returns an enum `Overwritten` representing any left-right pairs that
    /// were overwritten by the call to `insert`. The example below details
    /// all possible enum variants that can be returned.
    ///
    /// # Warnings
    ///
    /// Somewhat paradoxically, calling `insert()` can actually reduce the size
    /// of the bimap! This is because of the invariant that each left value
    /// maps to exactly one right value and vice versa.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::{BiHashMap, Overwritten};
    ///
    /// let mut bimap = BiHashMap::new();
    /// assert_eq!(bimap.len(), 0); // {}
    ///
    /// // no values are overwritten.
    /// assert_eq!(bimap.insert('a', 1), Overwritten::Neither);
    /// assert_eq!(bimap.len(), 1); // {'a' <> 1}
    ///
    /// // no values are overwritten.
    /// assert_eq!(bimap.insert('b', 2), Overwritten::Neither);
    /// assert_eq!(bimap.len(), 2); // {'a' <> 1, 'b' <> 2}
    ///
    /// // ('a', 1) already exists, so inserting ('a', 4) overwrites 'a', the left value.
    /// // the previous left-right pair ('a', 1) is returned.
    /// assert_eq!(bimap.insert('a', 4), Overwritten::Left('a', 1));
    /// assert_eq!(bimap.len(), 2); // {'a' <> 4, 'b' <> 2}
    ///
    /// // ('b', 2) already exists, so inserting ('c', 2) overwrites 2, the right value.
    /// // the previous left-right pair ('b', 2) is returned.
    /// assert_eq!(bimap.insert('c', 2), Overwritten::Right('b', 2));
    /// assert_eq!(bimap.len(), 2); // {'a' <> 1, 'c' <> 2}
    ///
    /// // both ('a', 4) and ('c', 2) already exist, so inserting ('a', 2) overwrites both.
    /// // ('a', 4) has the overwritten left value ('a'), so it's the first tuple returned.
    /// // ('c', 2) has the overwritten right value (2), so it's the second tuple returned.
    /// assert_eq!(bimap.insert('a', 2), Overwritten::Both(('a', 4), ('c', 2)));
    /// assert_eq!(bimap.len(), 1); // {'a' <> 2} // bimap is smaller than before!
    ///
    /// // ('a', 2) already exists, so inserting ('a', 2) overwrites the pair.
    /// // the previous left-right pair ('a', 2) is returned.
    /// assert_eq!(bimap.insert('a', 2), Overwritten::Pair('a', 2));
    /// assert_eq!(bimap.len(), 1); // {'a' <> 2}
    /// ```
    pub fn insert(&mut self, left: L, right: R) -> Overwritten<L, R> {
        let retval = match (self.remove_by_left(&left), self.remove_by_right(&right)) {
            (None, None) => Overwritten::Neither,
            (None, Some(r_pair)) => Overwritten::Right(r_pair.0, r_pair.1),
            (Some(l_pair), None) => {
                // since remove_by_left() was called first, it's possible the right value was
                // removed if a duplicate pair is being inserted
                if l_pair.1 == right {
                    Overwritten::Pair(l_pair.0, l_pair.1)
                } else {
                    Overwritten::Left(l_pair.0, l_pair.1)
                }
            }
            (Some(l_pair), Some(r_pair)) => Overwritten::Both(l_pair, r_pair),
        };
        self.insert_unchecked(left, right);
        retval
    }

    /// Inserts the given left-right pair into the bimap without overwriting any
    /// existing values.
    ///
    /// Returns `Ok(())` if the pair was successfully inserted into the bimap.
    /// If either value exists in the map, `Err((left, right)` is returned
    /// with the attempted left-right pair and the map is unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// assert_eq!(bimap.insert_no_overwrite('a', 1), Ok(()));
    /// assert_eq!(bimap.insert_no_overwrite('b', 2), Ok(()));
    /// assert_eq!(bimap.insert_no_overwrite('a', 3), Err(('a', 3)));
    /// assert_eq!(bimap.insert_no_overwrite('c', 2), Err(('c', 2)));
    /// ```
    pub fn insert_no_overwrite(&mut self, left: L, right: R) -> Result<(), (L, R)> {
        if self.contains_left(&left) || self.contains_right(&right) {
            Err((left, right))
        } else {
            self.insert_unchecked(left, right);
            Ok(())
        }
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// In other words, remove all left-right pairs `(l, r)` such that `f(&l,
    /// &r)` returns `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::BiHashMap;
    ///
    /// let mut bimap = BiHashMap::new();
    /// bimap.insert('a', 1);
    /// bimap.insert('b', 2);
    /// bimap.insert('c', 3);
    /// bimap.retain(|&l, &r| r >= 2);
    /// assert_eq!(bimap.len(), 2);
    /// assert_eq!(bimap.get_by_left(&'b'), Some(&2));
    /// assert_eq!(bimap.get_by_left(&'c'), Some(&3));
    /// assert_eq!(bimap.get_by_left(&'a'), None);
    /// ```
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&L, &R) -> bool,
    {
        let mut f = f;
        let right2left = &mut self.right2left;
        self.left2right.retain(|l, r| {
            let to_retain = f(&l.0, &r.0);
            if !to_retain {
                right2left.remove(r);
            }
            to_retain
        });
    }

    /// Inserts the given left-right pair into the bimap without checking if the
    /// pair already exists.
    fn insert_unchecked(&mut self, left: L, right: R) {
        let left = Ref(Rc::new(left));
        let right = Ref(Rc::new(right));
        self.left2right.insert(left.clone(), right.clone());
        self.right2left.insert(right, left);
    }
}

impl<L, R, LS, RS> Clone for BiHashMap<L, R, LS, RS>
where
    L: Clone + Eq + Hash,
    R: Clone + Eq + Hash,
    LS: BuildHasher + Clone,
    RS: BuildHasher + Clone,
{
    fn clone(&self) -> BiHashMap<L, R, LS, RS> {
        let mut new_bimap = BiHashMap::with_capacity_and_hashers(
            self.capacity(),
            self.left2right.hasher().clone(),
            self.right2left.hasher().clone(),
        );
        for (l, r) in self.iter() {
            new_bimap.insert(l.clone(), r.clone());
        }
        new_bimap
    }
}

impl<L, R, LS, RS> fmt::Debug for BiHashMap<L, R, LS, RS>
where
    L: fmt::Debug,
    R: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        struct EntryDebugger<'a, L, R> {
            left: &'a L,
            right: &'a R,
        }
        impl<'a, L, R> fmt::Debug for EntryDebugger<'a, L, R>
        where
            L: fmt::Debug,
            R: fmt::Debug,
        {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                self.left.fmt(f)?;
                write!(f, " <> ")?;
                self.right.fmt(f)
            }
        }
        f.debug_set()
            .entries(
                self.left2right
                    .iter()
                    .map(|(left, right)| EntryDebugger { left, right }),
            )
            .finish()
    }
}

impl<L, R, LS, RS> Default for BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
    LS: BuildHasher + Default,
    RS: BuildHasher + Default,
{
    fn default() -> BiHashMap<L, R, LS, RS> {
        BiHashMap {
            left2right: HashMap::default(),
            right2left: HashMap::default(),
        }
    }
}

impl<L, R, LS, RS> Eq for BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
    LS: BuildHasher,
    RS: BuildHasher,
{
}

impl<L, R, LS, RS> FromIterator<(L, R)> for BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
    LS: BuildHasher + Default,
    RS: BuildHasher + Default,
{
    fn from_iter<I>(iter: I) -> BiHashMap<L, R, LS, RS>
    where
        I: IntoIterator<Item = (L, R)>,
    {
        let iter = iter.into_iter();
        let mut bimap = match iter.size_hint() {
            (lower, None) => {
                BiHashMap::with_capacity_and_hashers(lower, LS::default(), RS::default())
            }
            (_, Some(upper)) => {
                BiHashMap::with_capacity_and_hashers(upper, LS::default(), RS::default())
            }
        };
        for (left, right) in iter {
            bimap.insert(left, right);
        }
        bimap
    }
}

impl<'a, L, R, LS, RS> IntoIterator for &'a BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
{
    type Item = (&'a L, &'a R);
    type IntoIter = Iter<'a, L, R>;

    fn into_iter(self) -> Iter<'a, L, R> {
        self.iter()
    }
}

impl<L, R, LS, RS> IntoIterator for BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
{
    type Item = (L, R);
    type IntoIter = IntoIter<L, R>;

    fn into_iter(self) -> IntoIter<L, R> {
        IntoIter {
            inner: self.left2right.into_iter(),
        }
    }
}

impl<L, R, LS, RS> Extend<(L, R)> for BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
    LS: BuildHasher,
    RS: BuildHasher,
{
    fn extend<T: IntoIterator<Item = (L, R)>>(&mut self, iter: T) {
        iter.into_iter().for_each(move |(l, r)| {
            self.insert(l, r);
        });
    }
}

impl<L, R, LS, RS> PartialEq for BiHashMap<L, R, LS, RS>
where
    L: Eq + Hash,
    R: Eq + Hash,
    LS: BuildHasher,
    RS: BuildHasher,
{
    fn eq(&self, other: &Self) -> bool {
        self.left2right == other.left2right
    }
}

/// An owning iterator over the left-right pairs in a `BiHashMap`.
pub struct IntoIter<L, R> {
    inner: hash_map::IntoIter<Ref<L>, Ref<R>>,
}

impl<L, R> ExactSizeIterator for IntoIter<L, R> {}

impl<L, R> FusedIterator for IntoIter<L, R> {}

impl<L, R> Iterator for IntoIter<L, R> {
    type Item = (L, R);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(l, r)| {
            (
                Rc::try_unwrap(l.0).ok().unwrap(),
                Rc::try_unwrap(r.0).ok().unwrap(),
            )
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// An iterator over the left-right pairs in a `BiHashMap`.
///
/// This struct is created by the [`iter`] method of `BiHashMap`.
///
/// [`iter`]: BiHashMap::iter
#[derive(Debug, Clone)]
pub struct Iter<'a, L, R> {
    inner: hash_map::Iter<'a, Ref<L>, Ref<R>>,
}

impl<'a, L, R> ExactSizeIterator for Iter<'a, L, R> {}

impl<'a, L, R> FusedIterator for Iter<'a, L, R> {}

impl<'a, L, R> Iterator for Iter<'a, L, R> {
    type Item = (&'a L, &'a R);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(l, r)| (&*l.0, &*r.0))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// An iterator over the left values in a `BiHashMap`.
///
/// This struct is created by the [`left_values`] method of `BiHashMap`.
///
/// [`left_values`]: BiHashMap::left_values
#[derive(Debug, Clone)]
pub struct LeftValues<'a, L, R> {
    inner: hash_map::Iter<'a, Ref<L>, Ref<R>>,
}

impl<'a, L, R> ExactSizeIterator for LeftValues<'a, L, R> {}

impl<'a, L, R> FusedIterator for LeftValues<'a, L, R> {}

impl<'a, L, R> Iterator for LeftValues<'a, L, R> {
    type Item = &'a L;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(l, _)| &*l.0)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// An iterator over the right values in a `BiHashMap`.
///
/// This struct is created by the [`right_values`] method of `BiHashMap`.
///
/// [`right_values`]: BiHashMap::right_values
#[derive(Debug, Clone)]
pub struct RightValues<'a, L, R> {
    inner: hash_map::Iter<'a, Ref<R>, Ref<L>>,
}

impl<'a, L, R> ExactSizeIterator for RightValues<'a, L, R> {}

impl<'a, L, R> FusedIterator for RightValues<'a, L, R> {}

impl<'a, L, R> Iterator for RightValues<'a, L, R> {
    type Item = &'a R;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(r, _)| &*r.0)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

// safe because internal Rcs are not exposed by the api and the reference counts
// only change in methods with &mut self
unsafe impl<L, R, LS, RS> Send for BiHashMap<L, R, LS, RS>
where
    L: Send,
    R: Send,
    LS: Send,
    RS: Send,
{
}
unsafe impl<L, R, LS, RS> Sync for BiHashMap<L, R, LS, RS>
where
    L: Sync,
    R: Sync,
    LS: Sync,
    RS: Sync,
{
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 1);
        bimap.insert('b', 2);
        let bimap2 = bimap.clone();
        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn deep_clone() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 1);
        bimap.insert('b', 2);
        let mut bimap2 = bimap.clone();

        // would panic if clone() didn't deep clone
        bimap.insert('b', 5);
        bimap2.insert('a', 12);
        bimap2.remove_by_left(&'a');
        bimap.remove_by_right(&2);
    }

    #[test]
    fn debug() {
        let mut bimap = BiHashMap::new();
        assert_eq!("{}", format!("{:?}", bimap));

        bimap.insert('a', 1);
        assert_eq!("{'a' <> 1}", format!("{:?}", bimap));

        bimap.insert('b', 2);
        let expected1 = "{'a' <> 1, 'b' <> 2}";
        let expected2 = "{'b' <> 2, 'a' <> 1}";
        let formatted = format!("{:?}", bimap);
        assert!(formatted == expected1 || formatted == expected2);
    }

    #[test]
    fn default() {
        let _ = BiHashMap::<char, i32>::default();
    }

    #[test]
    fn eq() {
        let mut bimap = BiHashMap::new();
        assert_eq!(bimap, bimap);
        bimap.insert('a', 1);
        assert_eq!(bimap, bimap);
        bimap.insert('b', 2);
        assert_eq!(bimap, bimap);

        let mut bimap2 = BiHashMap::new();
        assert_ne!(bimap, bimap2);
        bimap2.insert('a', 1);
        assert_ne!(bimap, bimap2);
        bimap2.insert('b', 2);
        assert_eq!(bimap, bimap2);
        bimap2.insert('c', 3);
        assert_ne!(bimap, bimap2);
    }

    #[test]
    fn from_iter() {
        let bimap = BiHashMap::from_iter(vec![
            ('a', 1),
            ('b', 2),
            ('c', 3),
            ('b', 2),
            ('a', 4),
            ('b', 3),
        ]);
        let mut bimap2 = BiHashMap::new();
        bimap2.insert('a', 4);
        bimap2.insert('b', 3);
        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn into_iter() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 3);
        bimap.insert('b', 2);
        bimap.insert('c', 1);
        let mut pairs = bimap.into_iter().collect::<Vec<_>>();
        pairs.sort();
        assert_eq!(pairs, vec![('a', 3), ('b', 2), ('c', 1)]);
    }

    #[test]
    fn into_iter_ref() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 3);
        bimap.insert('b', 2);
        bimap.insert('c', 1);
        let mut pairs = (&bimap).into_iter().collect::<Vec<_>>();
        pairs.sort();
        assert_eq!(pairs, vec![(&'a', &3), (&'b', &2), (&'c', &1)]);
    }

    #[test]
    fn extend() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 3);
        bimap.insert('b', 2);
        bimap.extend(vec![('c', 3), ('b', 1), ('a', 4)]);
        let mut bimap2 = BiHashMap::new();
        bimap2.insert('a', 4);
        bimap2.insert('b', 1);
        bimap2.insert('c', 3);
        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn iter() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 1);
        bimap.insert('b', 2);
        bimap.insert('c', 3);
        let mut pairs = bimap.iter().map(|(c, i)| (*c, *i)).collect::<Vec<_>>();
        pairs.sort();
        assert_eq!(pairs, vec![('a', 1), ('b', 2), ('c', 3)]);
    }

    #[test]
    fn left_values() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 3);
        bimap.insert('b', 2);
        bimap.insert('c', 1);
        let mut left_values = bimap.left_values().cloned().collect::<Vec<_>>();
        left_values.sort();
        assert_eq!(left_values, vec!['a', 'b', 'c'])
    }

    #[test]
    fn right_values() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 3);
        bimap.insert('b', 2);
        bimap.insert('c', 1);
        let mut right_values = bimap.right_values().cloned().collect::<Vec<_>>();
        right_values.sort();
        assert_eq!(right_values, vec![1, 2, 3])
    }

    #[test]
    fn capacity() {
        let bimap = BiHashMap::<char, i32>::with_capacity(10);
        assert!(bimap.capacity() >= 10);
    }

    #[test]
    fn with_hashers() {
        let s_left = hash_map::RandomState::new();
        let s_right = hash_map::RandomState::new();
        let mut bimap = BiHashMap::<char, i32>::with_hashers(s_left, s_right);
        bimap.insert('a', 42);
        assert_eq!(Some(&'a'), bimap.get_by_right(&42));
        assert_eq!(Some(&42), bimap.get_by_left(&'a'));
    }

    #[test]
    fn reserve() {
        let mut bimap = BiHashMap::<char, i32>::new();
        assert!(bimap.is_empty());
        assert_eq!(bimap.len(), 0);
        assert_eq!(bimap.capacity(), 0);

        bimap.reserve(10);
        assert!(bimap.is_empty());
        assert_eq!(bimap.len(), 0);
        assert!(bimap.capacity() >= 10);
    }

    #[test]
    fn shrink_to_fit() {
        let mut bimap = BiHashMap::<char, i32>::with_capacity(100);
        assert!(bimap.is_empty());
        assert_eq!(bimap.len(), 0);
        assert!(bimap.capacity() >= 100);

        bimap.insert('a', 1);
        bimap.insert('b', 2);
        assert!(!bimap.is_empty());
        assert_eq!(bimap.len(), 2);
        assert!(bimap.capacity() >= 100);

        bimap.shrink_to_fit();
        assert!(!bimap.is_empty());
        assert_eq!(bimap.len(), 2);
        assert!(bimap.capacity() >= 2);
    }

    #[test]
    fn shrink_to() {
        let mut bimap = BiHashMap::<char, i32>::with_capacity(100);
        assert!(bimap.is_empty());
        assert_eq!(bimap.len(), 0);
        assert!(bimap.capacity() >= 100);

        bimap.insert('a', 1);
        bimap.insert('b', 2);
        assert!(!bimap.is_empty());
        assert_eq!(bimap.len(), 2);
        assert!(bimap.capacity() >= 100);

        bimap.shrink_to(10);
        assert!(!bimap.is_empty());
        assert_eq!(bimap.len(), 2);
        assert!(bimap.capacity() >= 10);

        bimap.shrink_to(0);
        assert!(!bimap.is_empty());
        assert_eq!(bimap.len(), 2);
        assert!(bimap.capacity() >= 2);
    }

    #[test]
    fn clear() {
        let mut bimap = vec![('a', 1)].into_iter().collect::<BiHashMap<_, _>>();
        assert_eq!(bimap.len(), 1);
        assert!(!bimap.is_empty());

        bimap.clear();

        assert_eq!(bimap.len(), 0);
        assert!(bimap.is_empty());
    }

    #[test]
    fn get_contains() {
        let bimap = vec![('a', 1)].into_iter().collect::<BiHashMap<_, _>>();

        assert_eq!(bimap.get_by_left(&'a'), Some(&1));
        assert!(bimap.contains_left(&'a'));

        assert_eq!(bimap.get_by_left(&'b'), None);
        assert!(!bimap.contains_left(&'b'));

        assert_eq!(bimap.get_by_right(&1), Some(&'a'));
        assert!(bimap.contains_right(&1));

        assert_eq!(bimap.get_by_right(&2), None);
        assert!(!bimap.contains_right(&2));
    }

    #[test]
    fn insert() {
        let mut bimap = BiHashMap::new();

        assert_eq!(bimap.insert('a', 1), Overwritten::Neither);
        assert_eq!(bimap.insert('a', 2), Overwritten::Left('a', 1));
        assert_eq!(bimap.insert('b', 2), Overwritten::Right('a', 2));
        assert_eq!(bimap.insert('b', 2), Overwritten::Pair('b', 2));

        assert_eq!(bimap.insert('c', 3), Overwritten::Neither);
        assert_eq!(bimap.insert('b', 3), Overwritten::Both(('b', 2), ('c', 3)));
    }

    #[test]
    fn insert_no_overwrite() {
        let mut bimap = BiHashMap::new();

        assert!(bimap.insert_no_overwrite('a', 1).is_ok());
        assert!(bimap.insert_no_overwrite('a', 2).is_err());
        assert!(bimap.insert_no_overwrite('b', 1).is_err());
    }

    #[test]
    fn retain_calls_f_once() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 1);
        bimap.insert('b', 2);
        bimap.insert('c', 3);

        // retain one element
        let mut i = 0;
        bimap.retain(|_l, _r| {
            i += 1;
            i <= 1
        });
        assert_eq!(bimap.len(), 1);
        assert_eq!(i, 3);
    }
}
