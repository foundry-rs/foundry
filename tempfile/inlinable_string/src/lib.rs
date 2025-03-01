// Copyright 2015, The inlinable_string crate Developers. See the COPYRIGHT file
// at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! The `inlinable_string` crate provides the
//! [`InlinableString`](./enum.InlinableString.html) type &mdash; an owned,
//! grow-able UTF-8 string that stores small strings inline and avoids
//! heap-allocation &mdash; and the
//! [`StringExt`](./string_ext/trait.StringExt.html) trait which abstracts
//! string operations over both `std::string::String` and `InlinableString` (or
//! even your own custom string type).
//!
//! `StringExt`'s API is mostly identical to `std::string::String`; unstable and
//! deprecated methods are not included. A `StringExt` implementation is
//! provided for both `std::string::String` and `InlinableString`. This enables
//! `InlinableString` to generally work as a drop-in replacement for
//! `std::string::String` and `&StringExt` to work with references to either
//! type.
//!
//! # Examples
//!
//! ```
//! use inlinable_string::{InlinableString, StringExt};
//!
//! // Small strings are stored inline and don't perform heap-allocation.
//! let mut s = InlinableString::from("small");
//! assert_eq!(s.capacity(), inlinable_string::INLINE_STRING_CAPACITY);
//!
//! // Inline strings are transparently promoted to heap-allocated strings when
//! // they grow too big.
//! s.push_str("a really long string that's bigger than `INLINE_STRING_CAPACITY`");
//! assert!(s.capacity() > inlinable_string::INLINE_STRING_CAPACITY);
//!
//! // This method can work on strings potentially stored inline on the stack,
//! // on the heap, or plain old `std::string::String`s!
//! fn takes_a_string_reference(string: &mut StringExt) {
//!    // Do something with the string...
//!    string.push_str("it works!");
//! }
//!
//! let mut s1 = String::from("this is a plain std::string::String");
//! let mut s2 = InlinableString::from("inline");
//!
//! // Both work!
//! takes_a_string_reference(&mut s1);
//! takes_a_string_reference(&mut s2);
//! ```
//!
//! # Porting Your Code
//!
//! * If `my_string` is always on the stack: `let my_string = String::new();` →
//! `let my_string = InlinableString::new();`
//!
//! * `fn foo(string: &mut String) { ... }` → `fn foo(string: &mut StringExt) { ... }`
//!
//! * `fn foo(string: &str) { ... }` does not need to be modified.
//!
//! * `struct S { member: String }` is a little trickier. If `S` is always stack
//! allocated, it probably makes sense to make `member` be of type
//! `InlinableString`. If `S` is heap-allocated and `member` is *always* small,
//! consider using the more restrictive
//! [`InlineString`](./inline_string/struct.InlineString.html) type. If `member` is
//! not always small, then it should probably be left as a `String`.
//!
//! # Serialization
//!
//! `InlinableString` implements [`serde`][serde-docs]'s `Serialize` and `Deserialize` traits.
//! Add the `serde` feature to your `Cargo.toml` to enable serialization.
//!
//! [serde-docs]: https://serde.rs

#![forbid(missing_docs)]
#![cfg_attr(feature = "nightly", feature(plugin))]
#![cfg_attr(all(test, feature = "nightly"), feature(test))]
#![cfg_attr(feature = "no_std", no_std)]

#[allow(unused_imports)]
#[cfg_attr(feature = "no_std", macro_use)]
extern crate alloc;

#[cfg(test)]
#[cfg(feature = "nightly")]
extern crate test;

#[cfg(feature = "serde")]
mod serde_impl;

pub mod inline_string;
pub mod string_ext;

pub use inline_string::{InlineString, INLINE_STRING_CAPACITY};
pub use string_ext::StringExt;

use alloc::borrow::{Borrow, Cow};
use alloc::vec::Vec;
use alloc::string::{FromUtf16Error, FromUtf8Error, String};
use core::cmp::Ordering;
use core::convert;
use core::fmt;
use core::hash;
use core::iter;
use core::mem;
use core::ops;
use core::str::FromStr;

/// An owned, grow-able UTF-8 string that allocates short strings inline on the
/// stack.
///
/// See the [module level documentation](./index.html) for more.
#[derive(Clone, Eq)]
pub enum InlinableString {
    /// A heap-allocated string.
    Heap(String),
    /// A small string stored inline.
    Inline(InlineString),
}

impl fmt::Debug for InlinableString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self as &str, f)
    }
}

impl iter::FromIterator<char> for InlinableString {
    fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> InlinableString {
        let mut buf = InlinableString::new();
        buf.extend(iter);
        buf
    }
}

impl<'a> iter::FromIterator<&'a str> for InlinableString {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> InlinableString {
        let mut buf = InlinableString::new();
        buf.extend(iter);
        buf
    }
}

impl Extend<char> for InlinableString {
    fn extend<I: IntoIterator<Item = char>>(&mut self, iterable: I) {
        let iterator = iterable.into_iter();
        let (lower_bound, _) = iterator.size_hint();
        self.reserve(lower_bound);
        for ch in iterator {
            self.push(ch);
        }
    }
}

impl<'a> Extend<&'a char> for InlinableString {
    fn extend<I: IntoIterator<Item = &'a char>>(&mut self, iter: I) {
        self.extend(iter.into_iter().cloned());
    }
}

impl<'a> Extend<&'a str> for InlinableString {
    fn extend<I: IntoIterator<Item = &'a str>>(&mut self, iterable: I) {
        let iterator = iterable.into_iter();
        let (lower_bound, _) = iterator.size_hint();
        self.reserve(lower_bound);
        for s in iterator {
            self.push_str(s);
        }
    }
}

impl<'a> ops::Add<&'a str> for InlinableString {
    type Output = InlinableString;

    #[inline]
    fn add(mut self, other: &str) -> InlinableString {
        self.push_str(other);
        self
    }
}

impl PartialOrd<InlinableString> for InlinableString {
    fn partial_cmp(&self, rhs: &InlinableString) -> Option<Ordering> {
        Some(Ord::cmp(&self[..], &rhs[..]))
    }
}

impl Ord for InlinableString {
    #[inline]
    fn cmp(&self, rhs: &InlinableString) -> Ordering {
        Ord::cmp(&self[..], &rhs[..])
    }
}

impl hash::Hash for InlinableString {
    #[inline]
    fn hash<H: hash::Hasher>(&self, hasher: &mut H) {
        (**self).hash(hasher)
    }
}

impl Borrow<str> for InlinableString {
    fn borrow(&self) -> &str {
        &*self
    }
}

impl AsRef<str> for InlinableString {
    fn as_ref(&self) -> &str {
        match *self {
            InlinableString::Heap(ref s) => &*s,
            InlinableString::Inline(ref s) => &*s,
        }
    }
}

impl AsMut<str> for InlinableString {
    fn as_mut(&mut self) -> &mut str {
        match *self {
            InlinableString::Heap(ref mut s) => s.as_mut_str(),
            InlinableString::Inline(ref mut s) => &mut s[..],
        }
    }
}

impl<'a> From<&'a str> for InlinableString {
    #[inline]
    fn from(string: &'a str) -> InlinableString {
        if string.len() <= INLINE_STRING_CAPACITY {
            InlinableString::Inline(string.into())
        } else {
            InlinableString::Heap(string.into())
        }
    }
}

impl From<String> for InlinableString {
    #[inline]
    fn from(string: String) -> InlinableString {
        if string.len() <= INLINE_STRING_CAPACITY {
            InlinableString::Inline(string.as_str().into())
        } else {
            InlinableString::Heap(string)
        }
    }
}

impl FromStr for InlinableString {
    type Err = convert::Infallible;

    #[inline]
    fn from_str(s: &str) -> Result<InlinableString, convert::Infallible> {
        Ok(InlinableString::from(s))
    }
}

impl Default for InlinableString {
    fn default() -> Self {
        InlinableString::new()
    }
}

impl fmt::Display for InlinableString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match *self {
            InlinableString::Heap(ref s) => s.fmt(f),
            InlinableString::Inline(ref s) => s.fmt(f),
        }
    }
}

impl fmt::Write for InlinableString {
    fn write_char(&mut self, ch: char) -> Result<(), fmt::Error> {
        self.push(ch);
        Ok(())
    }
    fn write_str(&mut self, s: &str) -> Result<(), fmt::Error> {
        self.push_str(s);
        Ok(())
    }
}

impl ops::Index<ops::Range<usize>> for InlinableString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::Range<usize>) -> &str {
        match *self {
            InlinableString::Heap(ref s) => s.index(index),
            InlinableString::Inline(ref s) => s.index(index),
        }
    }
}

impl ops::Index<ops::RangeTo<usize>> for InlinableString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::RangeTo<usize>) -> &str {
        match *self {
            InlinableString::Heap(ref s) => s.index(index),
            InlinableString::Inline(ref s) => s.index(index),
        }
    }
}

impl ops::Index<ops::RangeFrom<usize>> for InlinableString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::RangeFrom<usize>) -> &str {
        match *self {
            InlinableString::Heap(ref s) => s.index(index),
            InlinableString::Inline(ref s) => s.index(index),
        }
    }
}

impl ops::Index<ops::RangeFull> for InlinableString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::RangeFull) -> &str {
        match *self {
            InlinableString::Heap(ref s) => s.index(index),
            InlinableString::Inline(ref s) => s.index(index),
        }
    }
}

impl ops::IndexMut<ops::Range<usize>> for InlinableString {
    #[inline]
    fn index_mut(&mut self, index: ops::Range<usize>) -> &mut str {
        match *self {
            InlinableString::Heap(ref mut s) => s.index_mut(index),
            InlinableString::Inline(ref mut s) => s.index_mut(index),
        }
    }
}

impl ops::IndexMut<ops::RangeTo<usize>> for InlinableString {
    #[inline]
    fn index_mut(&mut self, index: ops::RangeTo<usize>) -> &mut str {
        match *self {
            InlinableString::Heap(ref mut s) => s.index_mut(index),
            InlinableString::Inline(ref mut s) => s.index_mut(index),
        }
    }
}

impl ops::IndexMut<ops::RangeFrom<usize>> for InlinableString {
    #[inline]
    fn index_mut(&mut self, index: ops::RangeFrom<usize>) -> &mut str {
        match *self {
            InlinableString::Heap(ref mut s) => s.index_mut(index),
            InlinableString::Inline(ref mut s) => s.index_mut(index),
        }
    }
}

impl ops::IndexMut<ops::RangeFull> for InlinableString {
    #[inline]
    fn index_mut(&mut self, index: ops::RangeFull) -> &mut str {
        match *self {
            InlinableString::Heap(ref mut s) => s.index_mut(index),
            InlinableString::Inline(ref mut s) => s.index_mut(index),
        }
    }
}

impl ops::Deref for InlinableString {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        match *self {
            InlinableString::Heap(ref s) => s.deref(),
            InlinableString::Inline(ref s) => s.deref(),
        }
    }
}

impl ops::DerefMut for InlinableString {
    #[inline]
    fn deref_mut(&mut self) -> &mut str {
        match *self {
            InlinableString::Heap(ref mut s) => s.deref_mut(),
            InlinableString::Inline(ref mut s) => s.deref_mut(),
        }
    }
}

impl PartialEq<InlinableString> for InlinableString {
    #[inline]
    fn eq(&self, rhs: &InlinableString) -> bool {
        PartialEq::eq(&self[..], &rhs[..])
    }
}

macro_rules! impl_eq {
    ($lhs:ty, $rhs: ty) => {
        impl<'a> PartialEq<$rhs> for $lhs {
            #[inline]
            fn eq(&self, other: &$rhs) -> bool {
                PartialEq::eq(&self[..], &other[..])
            }
        }

        impl<'a> PartialEq<$lhs> for $rhs {
            #[inline]
            fn eq(&self, other: &$lhs) -> bool {
                PartialEq::eq(&self[..], &other[..])
            }
        }
    };
}

impl_eq! { InlinableString, str }
impl_eq! { InlinableString, String }
impl_eq! { InlinableString, &'a str }
impl_eq! { InlinableString, InlineString }
impl_eq! { Cow<'a, str>, InlinableString }

impl<'a> StringExt<'a> for InlinableString {
    #[inline]
    fn new() -> Self {
        InlinableString::Inline(InlineString::new())
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        if capacity <= INLINE_STRING_CAPACITY {
            InlinableString::Inline(InlineString::new())
        } else {
            InlinableString::Heap(String::with_capacity(capacity))
        }
    }

    #[inline]
    fn from_utf8(vec: Vec<u8>) -> Result<Self, FromUtf8Error> {
        String::from_utf8(vec).map(InlinableString::Heap)
    }

    #[inline]
    fn from_utf16(v: &[u16]) -> Result<Self, FromUtf16Error> {
        String::from_utf16(v).map(InlinableString::Heap)
    }

    #[inline]
    fn from_utf16_lossy(v: &[u16]) -> Self {
        InlinableString::Heap(String::from_utf16_lossy(v))
    }

    #[inline]
    unsafe fn from_raw_parts(buf: *mut u8, length: usize, capacity: usize) -> Self {
        InlinableString::Heap(String::from_raw_parts(buf, length, capacity))
    }

    #[inline]
    unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Self {
        InlinableString::Heap(String::from_utf8_unchecked(bytes))
    }

    #[inline]
    fn into_bytes(self) -> Vec<u8> {
        match self {
            InlinableString::Heap(s) => s.into_bytes(),
            InlinableString::Inline(s) => Vec::from(&s[..]),
        }
    }

    #[inline]
    fn push_str(&mut self, string: &str) {
        let promoted = match *self {
            InlinableString::Inline(ref mut s) => {
                if s.push_str(string).is_ok() {
                    return;
                }
                let mut promoted = String::with_capacity(string.len() + s.len());
                promoted.push_str(&*s);
                promoted.push_str(string);
                promoted
            }
            InlinableString::Heap(ref mut s) => {
                s.push_str(string);
                return;
            }
        };
        mem::swap(self, &mut InlinableString::Heap(promoted));
    }

    #[inline]
    fn capacity(&self) -> usize {
        match *self {
            InlinableString::Heap(ref s) => s.capacity(),
            InlinableString::Inline(_) => INLINE_STRING_CAPACITY,
        }
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        let promoted = match *self {
            InlinableString::Inline(ref s) => {
                let new_capacity = s.len() + additional;
                if new_capacity <= INLINE_STRING_CAPACITY {
                    return;
                }
                let mut promoted = String::with_capacity(new_capacity);
                promoted.push_str(&s);
                promoted
            }
            InlinableString::Heap(ref mut s) => {
                s.reserve(additional);
                return;
            }
        };
        mem::swap(self, &mut InlinableString::Heap(promoted));
    }

    #[inline]
    fn reserve_exact(&mut self, additional: usize) {
        let promoted = match *self {
            InlinableString::Inline(ref s) => {
                let new_capacity = s.len() + additional;
                if new_capacity <= INLINE_STRING_CAPACITY {
                    return;
                }
                let mut promoted = String::with_capacity(new_capacity);
                promoted.push_str(&s);
                promoted
            }
            InlinableString::Heap(ref mut s) => {
                s.reserve_exact(additional);
                return;
            }
        };
        mem::swap(self, &mut InlinableString::Heap(promoted));
    }

    #[inline]
    fn shrink_to_fit(&mut self) {
        if self.len() <= INLINE_STRING_CAPACITY {
            let demoted = if let InlinableString::Heap(ref s) = *self {
                InlineString::from(&s[..])
            } else {
                return;
            };
            mem::swap(self, &mut InlinableString::Inline(demoted));
            return;
        }

        match *self {
            InlinableString::Heap(ref mut s) => s.shrink_to_fit(),
            _ => panic!("inlinable_string: internal error: this branch should be unreachable"),
        };
    }

    #[inline]
    fn push(&mut self, ch: char) {
        let promoted = match *self {
            InlinableString::Inline(ref mut s) => {
                if s.push(ch).is_ok() {
                    return;
                }

                let mut promoted = String::with_capacity(s.len() + 1);
                promoted.push_str(&*s);
                promoted.push(ch);
                promoted
            }
            InlinableString::Heap(ref mut s) => {
                s.push(ch);
                return;
            }
        };

        mem::swap(self, &mut InlinableString::Heap(promoted));
    }

    #[inline]
    fn as_bytes(&self) -> &[u8] {
        match *self {
            InlinableString::Heap(ref s) => s.as_bytes(),
            InlinableString::Inline(ref s) => s.as_bytes(),
        }
    }

    #[inline]
    fn truncate(&mut self, new_len: usize) {
        match *self {
            InlinableString::Heap(ref mut s) => s.truncate(new_len),
            InlinableString::Inline(ref mut s) => s.truncate(new_len),
        };
    }

    #[inline]
    fn pop(&mut self) -> Option<char> {
        match *self {
            InlinableString::Heap(ref mut s) => s.pop(),
            InlinableString::Inline(ref mut s) => s.pop(),
        }
    }

    #[inline]
    fn remove(&mut self, idx: usize) -> char {
        match *self {
            InlinableString::Heap(ref mut s) => s.remove(idx),
            InlinableString::Inline(ref mut s) => s.remove(idx),
        }
    }

    #[inline]
    fn insert(&mut self, idx: usize, ch: char) {
        let promoted = match *self {
            InlinableString::Heap(ref mut s) => {
                s.insert(idx, ch);
                return;
            }
            InlinableString::Inline(ref mut s) => {
                if s.insert(idx, ch).is_ok() {
                    return;
                }

                let mut promoted = String::with_capacity(s.len() + 1);
                promoted.push_str(&s[..idx]);
                promoted.push(ch);
                promoted.push_str(&s[idx..]);
                promoted
            }
        };

        mem::swap(self, &mut InlinableString::Heap(promoted));
    }

    #[inline]
    fn insert_str(&mut self, idx: usize, string: &str) {
        let promoted = match *self {
            InlinableString::Heap(ref mut s) => {
                s.insert_str(idx, string);
                return;
            }
            InlinableString::Inline(ref mut s) => {
                if s.insert_str(idx, string).is_ok() {
                    return;
                }

                let mut promoted = String::with_capacity(s.len() + string.len());
                promoted.push_str(&s[..idx]);
                promoted.push_str(string);
                promoted.push_str(&s[idx..]);
                promoted
            }
        };

        mem::swap(self, &mut InlinableString::Heap(promoted));
    }

    #[inline]
    unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
        match *self {
            InlinableString::Heap(ref mut s) => &mut s.as_mut_vec()[..],
            InlinableString::Inline(ref mut s) => s.as_mut_slice(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match *self {
            InlinableString::Heap(ref s) => s.len(),
            InlinableString::Inline(ref s) => s.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use core::iter::FromIterator;
    use super::{InlinableString, StringExt, INLINE_STRING_CAPACITY};
    use core::cmp::Ordering;
    use core::str::FromStr;

    #[test]
    fn test_size() {
        use core::mem::size_of;
        assert_eq!(size_of::<InlinableString>(), 4 * size_of::<usize>());
    }

    // First, specifically test operations that overflow InlineString's capacity
    // and require promoting the string to heap allocation.

    #[test]
    fn test_push_str() {
        let mut s = InlinableString::new();
        s.push_str("small");
        assert_eq!(s, "small");

        let long_str = "this is a really long string that is much larger than
                        INLINE_STRING_CAPACITY and so cannot be stored inline.";
        s.push_str(long_str);
        assert_eq!(s, String::from("small") + long_str);
    }

    #[test]
    fn test_write() {
        use core::fmt::Write;
        let mut s = InlinableString::new();
        write!(&mut s, "small").expect("!write");
        assert_eq!(s, "small");

        let long_str = "this is a really long string that is much larger than
                        INLINE_STRING_CAPACITY and so cannot be stored inline.";
        write!(&mut s, "{}", long_str).expect("!write");
        assert_eq!(s, String::from("small") + long_str);
    }

    #[test]
    fn test_push() {
        let mut s = InlinableString::new();

        for _ in 0..INLINE_STRING_CAPACITY {
            s.push('a');
        }
        s.push('a');

        assert_eq!(
            s,
            String::from_iter((0..INLINE_STRING_CAPACITY + 1).map(|_| 'a'))
        );
    }

    #[test]
    fn test_insert() {
        let mut s = InlinableString::new();

        for _ in 0..INLINE_STRING_CAPACITY {
            s.insert(0, 'a');
        }
        s.insert(0, 'a');

        assert_eq!(
            s,
            String::from_iter((0..INLINE_STRING_CAPACITY + 1).map(|_| 'a'))
        );
    }

    #[test]
    fn test_insert_str() {
        let mut s = InlinableString::new();

        for _ in 0..(INLINE_STRING_CAPACITY / 3) {
            s.insert_str(0, "foo");
        }
        s.insert_str(0, "foo");

        assert_eq!(
            s,
            String::from_iter((0..(INLINE_STRING_CAPACITY / 3) + 1).map(|_| "foo"))
        );
    }

    // Next, some general sanity tests.

    #[test]
    fn test_new() {
        let s = <InlinableString as StringExt>::new();
        assert!(StringExt::is_empty(&s));
    }

    #[test]
    fn test_with_capacity() {
        let s = <InlinableString as StringExt>::with_capacity(10);
        assert!(StringExt::capacity(&s) >= 10);
    }

    #[test]
    fn test_from_utf8() {
        let s = <InlinableString as StringExt>::from_utf8(vec![104, 101, 108, 108, 111]);
        assert_eq!(s.unwrap(), "hello");
    }

    #[test]
    fn test_from_utf16() {
        let v = &mut [0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0x0069, 0x0063];
        let s = <InlinableString as StringExt>::from_utf16(v);
        assert_eq!(s.unwrap(), "𝄞music");
    }

    #[test]
    fn test_from_utf16_lossy() {
        let input = b"Hello \xF0\x90\x80World";
        let output = <InlinableString as StringExt>::from_utf8_lossy(input);
        assert_eq!(output, "Hello \u{FFFD}World");
    }

    #[test]
    fn test_into_bytes() {
        let s = InlinableString::from("hello");
        let bytes = StringExt::into_bytes(s);
        assert_eq!(bytes, [104, 101, 108, 108, 111]);
    }

    #[test]
    fn test_capacity() {
        let s = <InlinableString as StringExt>::with_capacity(100);
        assert!(InlinableString::capacity(&s) >= 100);
    }

    #[test]
    fn test_reserve() {
        let mut s = <InlinableString as StringExt>::new();
        StringExt::reserve(&mut s, 100);
        assert!(InlinableString::capacity(&s) >= 100);
    }

    #[test]
    fn test_reserve_exact() {
        let mut s = <InlinableString as StringExt>::new();
        StringExt::reserve_exact(&mut s, 100);
        assert!(InlinableString::capacity(&s) >= 100);
    }

    #[test]
    fn test_shrink_to_fit() {
        let mut s = <InlinableString as StringExt>::with_capacity(100);
        StringExt::push_str(&mut s, "foo");
        StringExt::shrink_to_fit(&mut s);
        assert_eq!(InlinableString::capacity(&s), INLINE_STRING_CAPACITY);
    }

    #[test]
    fn test_truncate() {
        let mut s = InlinableString::from("foo");
        StringExt::truncate(&mut s, 1);
        assert_eq!(s, "f");
    }

    #[test]
    fn test_pop() {
        let mut s = InlinableString::from("foo");
        assert_eq!(StringExt::pop(&mut s), Some('o'));
        assert_eq!(StringExt::pop(&mut s), Some('o'));
        assert_eq!(StringExt::pop(&mut s), Some('f'));
        assert_eq!(StringExt::pop(&mut s), None);
    }

    #[test]
    fn test_ord() {
        let s1 = InlinableString::from("foo");
        let s2 = InlinableString::from("bar");
        assert_eq!(Ord::cmp(&s1, &s2), Ordering::Greater);
        assert_eq!(Ord::cmp(&s1, &s1), Ordering::Equal);
    }

    #[test]
    fn test_display() {
        let short = InlinableString::from("he");
        let long = InlinableString::from("hello world");
        assert_eq!(format!("{}", short), "he".to_string());
        assert_eq!(format!("{}", long), "hello world".to_string());
    }

    #[test]
    fn test_debug() {
        let short = InlinableString::from("he");
        let long = InlinableString::from("hello world hello world hello world");
        assert_eq!(format!("{:?}", short), "\"he\"");
        assert_eq!(
            format!("{:?}", long),
            "\"hello world hello world hello world\""
        );
    }

    // example generic function where impl FromStr for InlinableString is useful
    fn parse_non_empty<T: FromStr>(s: &str) -> Option<T> {
        if s.len() == 0 {
            None
        } else {
            let val = T::from_str(s).unwrap_or_else(|_| panic!("unwrap"));
            Some(val)
        }
    }

    #[test]
    fn test_fromstr() {
        assert_eq!(parse_non_empty::<InlinableString>(""), None);
        assert_eq!(parse_non_empty::<u8>("10"), Some(10u8));
        assert_eq!(
            parse_non_empty::<InlinableString>("foo"),
            Some(InlinableString::from("foo"))
        );
    }
}

#[cfg(test)]
#[cfg(feature = "nightly")]
mod benches {
    #[cfg(feature = "no_std")]
    use alloc::string::String;
    use super::{InlinableString, StringExt};
    use test::{black_box, Bencher};

    const SMALL_STR: &'static str = "foobar";

    const LARGE_STR: &'static str =
        "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
         abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";

    #[bench]
    fn bench_std_string_push_str_small_onto_empty(b: &mut Bencher) {
        b.iter(|| {
            let mut s = String::new();
            s.push_str(SMALL_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_inlinable_string_push_str_small_onto_empty(b: &mut Bencher) {
        b.iter(|| {
            let mut s = InlinableString::new();
            s.push_str(SMALL_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_std_string_push_str_large_onto_empty(b: &mut Bencher) {
        b.iter(|| {
            let mut s = String::new();
            s.push_str(LARGE_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_inlinable_string_push_str_large_onto_empty(b: &mut Bencher) {
        b.iter(|| {
            let mut s = InlinableString::new();
            s.push_str(LARGE_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_std_string_push_str_small_onto_small(b: &mut Bencher) {
        b.iter(|| {
            let mut s = String::from(SMALL_STR);
            s.push_str(SMALL_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_inlinable_string_push_str_small_onto_small(b: &mut Bencher) {
        b.iter(|| {
            let mut s = InlinableString::from(SMALL_STR);
            s.push_str(SMALL_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_std_string_push_str_large_onto_large(b: &mut Bencher) {
        b.iter(|| {
            let mut s = String::from(LARGE_STR);
            s.push_str(LARGE_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_inlinable_string_push_str_large_onto_large(b: &mut Bencher) {
        b.iter(|| {
            let mut s = InlinableString::from(LARGE_STR);
            s.push_str(LARGE_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_std_string_from_small(b: &mut Bencher) {
        b.iter(|| {
            let s = String::from(SMALL_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_inlinable_string_from_small(b: &mut Bencher) {
        b.iter(|| {
            let s = InlinableString::from(SMALL_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_std_string_from_large(b: &mut Bencher) {
        b.iter(|| {
            let s = String::from(LARGE_STR);
            black_box(s);
        });
    }

    #[bench]
    fn bench_inlinable_string_from_large(b: &mut Bencher) {
        b.iter(|| {
            let s = InlinableString::from(LARGE_STR);
            black_box(s);
        });
    }
}
