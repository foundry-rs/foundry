//! A fast two-way bijective map.
//!
//! A bimap is a [bijective map] between values of type `L`, called left values,
//! and values of type `R`, called right values. This means every left value is
//! associated with exactly one right value and vice versa. Compare this to a
//! [`HashMap`] or [`BTreeMap`], where every key is associated with exactly one
//! value but a value can be associated with more than one key.
//!
//! This crate provides two kinds of bimap: a [`BiHashMap`] and a
//! [`BiBTreeMap`]. Internally, each one is composed of two maps, one for the
//! left-to-right direction and one for right-to-left. As such, the big-O
//! performance of the `get`, `remove`, `insert`, and `contains` methods are the
//! same as those of the backing map.
//!
//! For convenience, the type definition [`BiMap`] corresponds to a `BiHashMap`.
//! If you're using this crate without the standard library, it instead
//! corresponds to a `BiBTreeMap`.
//!
//! # Examples
//!
//! ```
//! use bimap::BiMap;
//!
//! let mut elements = BiMap::new();
//!
//! // insert chemicals and their corresponding symbols
//! elements.insert("hydrogen", "H");
//! elements.insert("carbon", "C");
//! elements.insert("bromine", "Br");
//! elements.insert("neodymium", "Nd");
//!
//! // retrieve chemical symbol by name (left to right)
//! assert_eq!(elements.get_by_left(&"bromine"), Some(&"Br"));
//! assert_eq!(elements.get_by_left(&"oxygen"), None);
//!
//! // retrieve name by chemical symbol (right to left)
//! assert_eq!(elements.get_by_right(&"C"), Some(&"carbon"));
//! assert_eq!(elements.get_by_right(&"Al"), None);
//!
//! // check membership
//! assert!(elements.contains_left(&"hydrogen"));
//! assert!(!elements.contains_right(&"He"));
//!
//! // remove elements
//! assert_eq!(
//!     elements.remove_by_left(&"neodymium"),
//!     Some(("neodymium", "Nd"))
//! );
//! assert_eq!(elements.remove_by_right(&"Nd"), None);
//!
//! // iterate over elements
//! for (left, right) in &elements {
//!     println!("the chemical symbol for {} is {}", left, right);
//! }
//! ```
//!
//! ## Insertion and overwriting
//!
//! Consider the following example:
//!
//! ```
//! use bimap::BiMap;
//!
//! let mut bimap = BiMap::new();
//! bimap.insert('a', 1);
//! bimap.insert('b', 1); // what to do here?
//! ```
//!
//! In order to maintain the bijection, the bimap cannot have both left-right
//! pairs `('a', 1)` and `('b', 1)`. Otherwise, the right-value `1` would have
//! two left values associated with it. Either we should allow the call to
//! `insert` to go through and overwrite `('a', 1)`, or not let `('b', 1)` be
//! inserted at all. This crate allows for both possibilities. To insert with
//! overwriting, use [`insert`], and to insert without overwriting, use
//! [`insert_no_overwrite`]. The return type of `insert` is the `enum`
//! [`Overwritten`], which indicates what values, if any, were overwritten; the
//! return type of `insert_no_overwrite` is a `Result` indicating if the
//! insertion was successful.
//!
//! This is especially important when dealing with types that can be equal while
//! having different data. Unlike a `HashMap` or `BTreeMap`, which [doesn't
//! update an equal key upon insertion], a bimap updates both the left values
//! and the right values.
//!
//! ```
//! use bimap::{BiMap, Overwritten};
//! use std::cmp::Ordering;
//! use std::hash::{Hash, Hasher};
//!
//! #[derive(Clone, Copy, Debug)]
//! struct Foo {
//!     important: char,
//!     unimportant: u32,
//! }
//!
//! // equality only depends on the important data
//! impl PartialEq for Foo {
//!     fn eq(&self, other: &Foo) -> bool {
//!         self.important == other.important
//!     }
//! }
//!
//! impl Eq for Foo {}
//!
//! impl PartialOrd for Foo {
//!     fn partial_cmp(&self, other: &Foo) -> Option<Ordering> {
//!         Some(self.cmp(other))
//!     }
//! }
//!
//! // ordering only depends on the important data
//! impl Ord for Foo {
//!     fn cmp(&self, other: &Foo) -> Ordering {
//!         self.important.cmp(&other.important)
//!     }
//! }
//!
//! // hash only depends on the important data
//! impl Hash for Foo {
//!     fn hash<H: Hasher>(&self, state: &mut H) {
//!         self.important.hash(state);
//!     }
//! }
//!
//! // create two Foos that are equal but have different data
//! let foo1 = Foo {
//!     important: 'a',
//!     unimportant: 1,
//! };
//! let foo2 = Foo {
//!     important: 'a',
//!     unimportant: 2,
//! };
//! assert_eq!(foo1, foo2);
//!
//! // insert both Foos into a bimap
//! let mut bimap = BiMap::new();
//! bimap.insert(foo1, 99);
//! let overwritten = bimap.insert(foo2, 100);
//!
//! // foo1 is overwritten and returned
//! match overwritten {
//!     Overwritten::Left(foo, 99) => assert_eq!(foo.unimportant, foo1.unimportant),
//!     _ => unreachable!(),
//! };
//!
//! // foo2 is in the bimap
//! assert_eq!(
//!     bimap.get_by_right(&100).unwrap().unimportant,
//!     foo2.unimportant
//! );
//! ```
//!
//! Note that the `FromIterator` and `Extend` implementations for both
//! `BiHashMap` and `BiBTreeMap` use the `insert` method internally, meaning
//! that values from the original iterator/collection can be silently
//! overwritten.
//!
//! ```
//! use bimap::BiMap;
//! use std::iter::FromIterator;
//!
//! // note that both 'b' and 'c' have the right-value 2
//! let mut bimap = BiMap::from_iter(vec![('a', 1), ('b', 2), ('c', 2)]);
//!
//! // ('b', 2) was overwritten by ('c', 2)
//! assert_eq!(bimap.len(), 2);
//! assert_eq!(bimap.get_by_left(&'b'), None);
//! assert_eq!(bimap.get_by_left(&'c'), Some(&2));
//! ```
//!
//! ## `no_std` compatibility
//!
//! This crate can be used without the standard library when the `std` feature
//! is disabled. If you choose to do this, only `BiBTreeMap` is available, not
//! `BiHashMap`.
//!
//! ## serde compatibility
//!
//! When the `serde` feature is enabled, implementations of `Serialize` and
//! `Deserialize` are provided for [`BiHashMap`] and [`BiBTreeMap`], allowing
//! them to be serialized or deserialized painlessly. See the [`serde`] module
//! for examples and more information.
//!
//! [bijective map]: https://en.wikipedia.org/wiki/Bijection
//! [doesn't update an equal key upon insertion]:
//! https://doc.rust-lang.org/std/collections/index.html#insert-and-complex-keys
//! [`HashMap`]: https://doc.rust-lang.org/std/collections/struct.HashMap.html
//! [`BTreeMap`]: https://doc.rust-lang.org/std/collections/struct.BTreeMap.html
//! [`insert`]: BiHashMap::insert
//! [`insert_no_overwrite`]: BiHashMap::insert_no_overwrite

// Document everything!
#![deny(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]

// Necessary to support no_std setups
#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

mod mem;

pub mod btree;
pub use btree::BiBTreeMap;

#[cfg(feature = "std")]
pub mod hash;
#[cfg(feature = "std")]
pub use hash::BiHashMap;

/// Type definition for convenience and compatibility with older versions of
/// this crate.
#[cfg(feature = "std")]
pub type BiMap<L, R> = BiHashMap<L, R>;

/// Type definition for convenience and compatibility with older versions of
/// this crate.
#[cfg(not(feature = "std"))]
pub type BiMap<L, R> = BiBTreeMap<L, R>;

#[cfg(all(feature = "serde", feature = "std"))]
pub mod serde;

/// The previous left-right pairs, if any, that were overwritten by a call to
/// the [`insert`](BiHashMap::insert) method of a bimap.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Overwritten<L, R> {
    /// Neither the left nor the right value previously existed in the bimap.
    Neither,

    /// The left value existed in the bimap, and the previous left-right pair is
    /// returned.
    Left(L, R),

    /// The right value existed in the bimap, and the previous left-right pair
    /// is returned.
    Right(L, R),

    /// The left-right pair already existed in the bimap, and the previous
    /// left-right pair is returned.
    Pair(L, R),

    /// Both the left and the right value existed in the bimap, but as part of
    /// separate pairs. The first tuple is the left-right pair of the
    /// previous left value, and the second is the left-right pair of the
    /// previous right value.
    Both((L, R), (L, R)),
}

impl<L, R> Overwritten<L, R> {
    /// Returns a boolean indicating if the `Overwritten` variant implies any
    /// values were overwritten.
    ///
    /// This method is `true` for all variants other than `Neither`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bimap::{BiMap, Overwritten};
    ///
    /// let mut bimap = BiMap::new();
    /// assert!(!bimap.insert('a', 1).did_overwrite());
    /// assert!(bimap.insert('a', 2).did_overwrite());
    /// ```
    pub fn did_overwrite(&self) -> bool {
        !matches!(self, Overwritten::Neither)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_overwrite() {
        assert_eq!(Overwritten::<char, i32>::Neither.did_overwrite(), false);
        assert_eq!(Overwritten::Left('a', 1).did_overwrite(), true);
        assert_eq!(Overwritten::Right('a', 1).did_overwrite(), true);
        assert_eq!(Overwritten::Pair('a', 1).did_overwrite(), true);
        assert_eq!(Overwritten::Both(('a', 1), ('b', 2)).did_overwrite(), true);
    }
}
