// Copyright 2015, The inlinable_string crate Developers. See the COPYRIGHT file
// at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! A short UTF-8 string that uses inline storage and does no heap
//! allocation. It may be no longer than `INLINE_STRING_CAPACITY` bytes long.
//!
//! The capacity restriction makes many operations that would otherwise be
//! infallible on `std::string::String` fallible. Additionally, many trait
//! interfaces don't allow returning an error when a string runs out of space,
//! and so the trait implementation simply panics. As such, `InlineString` does
//! not implement `StringExt` and is ***not*** a drop-in replacement for
//! `std::string::String` in the way that `inlinable_string::InlinableString`
//! aims to be, and is generally difficult to work with. It is not recommended
//! to use this type directly unless you really, really want to avoid heap
//! allocation, can live with the imposed size restrictions, and are willing
//! work around potential sources of panics (eg, in the `From` trait
//! implementation).
//!
//! # Examples
//!
//! ```
//! use inlinable_string::InlineString;
//!
//! let mut s = InlineString::new();
//! assert!(s.push_str("hi world").is_ok());
//! assert_eq!(s, "hi world");
//!
//! assert!(s.push_str("a really long string that is much bigger than `INLINE_STRING_CAPACITY`").is_err());
//! assert_eq!(s, "hi world");
//! ```

use alloc::borrow;
use core::fmt;
use core::hash;
use core::ops;
use core::ptr;
use core::str;


/// The capacity (in bytes) of inline storage for small strings.
/// `InlineString::len()` may never be larger than this.
///
/// Sometime in the future, when Rust's generics support specializing with
/// compile-time static integers, this number should become configurable.
#[cfg(target_pointer_width = "64")]
pub const INLINE_STRING_CAPACITY: usize = 30;
#[cfg(target_pointer_width = "32")]
pub const INLINE_STRING_CAPACITY: usize = 14;

/// A short UTF-8 string that uses inline storage and does no heap allocation.
///
/// See the [module level documentation](./index.html) for more.
#[derive(Clone, Debug, Eq)]
pub struct InlineString {
    length: u8,
    bytes: [u8; INLINE_STRING_CAPACITY],
}

/// The error returned when there is not enough space in a `InlineString` for the
/// requested operation.
#[derive(Debug, PartialEq)]
pub struct NotEnoughSpaceError;

impl AsRef<str> for InlineString {
    fn as_ref(&self) -> &str {
        self.assert_sanity();
        unsafe { str::from_utf8_unchecked(&self.bytes[..self.len()]) }
    }
}

impl AsRef<[u8]> for InlineString {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl AsMut<str> for InlineString {
    fn as_mut(&mut self) -> &mut str {
        self.assert_sanity();
        let length = self.len();
        unsafe { str::from_utf8_unchecked_mut(&mut self.bytes[..length]) }
    }
}

impl AsMut<[u8]> for InlineString {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        self.assert_sanity();
        let length = self.len();
        &mut self.bytes[0..length]
    }
}

/// Create a `InlineString` from the given `&str`.
///
/// # Panics
///
/// If the given string's size is greater than `INLINE_STRING_CAPACITY`, this
/// method panics.
impl<'a> From<&'a str> for InlineString {
    fn from(string: &'a str) -> InlineString {
        let string_len = string.len();
        assert!(string_len <= INLINE_STRING_CAPACITY);

        let mut ss = InlineString::new();
        unsafe {
            ptr::copy_nonoverlapping(string.as_ptr(), ss.bytes.as_mut_ptr(), string_len);
        }
        ss.length = string_len as u8;

        ss.assert_sanity();
        ss
    }
}

impl fmt::Display for InlineString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        self.assert_sanity();
        write!(f, "{}", self as &str)
    }
}

impl fmt::Write for InlineString {
    fn write_char(&mut self, ch: char) -> Result<(), fmt::Error> {
        self.push(ch).map_err(|_| fmt::Error)
    }
    fn write_str(&mut self, s: &str) -> Result<(), fmt::Error> {
        self.push_str(s).map_err(|_| fmt::Error)
    }
}

impl hash::Hash for InlineString {
    #[inline]
    fn hash<H: hash::Hasher>(&self, hasher: &mut H) {
        (**self).hash(hasher)
    }
}

impl ops::Index<ops::Range<usize>> for InlineString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::Range<usize>) -> &str {
        self.assert_sanity();
        &self[..][index]
    }
}

impl ops::Index<ops::RangeTo<usize>> for InlineString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::RangeTo<usize>) -> &str {
        self.assert_sanity();
        &self[..][index]
    }
}

impl ops::Index<ops::RangeFrom<usize>> for InlineString {
    type Output = str;

    #[inline]
    fn index(&self, index: ops::RangeFrom<usize>) -> &str {
        self.assert_sanity();
        &self[..][index]
    }
}

impl ops::Index<ops::RangeFull> for InlineString {
    type Output = str;

    #[inline]
    fn index(&self, _index: ops::RangeFull) -> &str {
        self.assert_sanity();
        unsafe { str::from_utf8_unchecked(&self.bytes[..self.len()]) }
    }
}

impl ops::IndexMut<ops::Range<usize>> for InlineString {
    #[inline]
    fn index_mut(&mut self, index: ops::Range<usize>) -> &mut str {
        self.assert_sanity();
        &mut self[..][index]
    }
}

impl ops::IndexMut<ops::RangeTo<usize>> for InlineString {
    #[inline]
    fn index_mut(&mut self, index: ops::RangeTo<usize>) -> &mut str {
        self.assert_sanity();
        &mut self[..][index]
    }
}

impl ops::IndexMut<ops::RangeFrom<usize>> for InlineString {
    #[inline]
    fn index_mut(&mut self, index: ops::RangeFrom<usize>) -> &mut str {
        self.assert_sanity();
        &mut self[..][index]
    }
}

impl ops::IndexMut<ops::RangeFull> for InlineString {
    #[inline]
    fn index_mut(&mut self, _index: ops::RangeFull) -> &mut str {
        self.assert_sanity();
        let length = self.len();
        unsafe { str::from_utf8_unchecked_mut(&mut self.bytes[..length]) }
    }
}

impl ops::Deref for InlineString {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.assert_sanity();
        unsafe { str::from_utf8_unchecked(&self.bytes[..self.len()]) }
    }
}

impl ops::DerefMut for InlineString {
    #[inline]
    fn deref_mut(&mut self) -> &mut str {
        self.assert_sanity();
        let length = self.len();
        unsafe { str::from_utf8_unchecked_mut(&mut self.bytes[..length]) }
    }
}

impl Default for InlineString {
    #[inline]
    fn default() -> InlineString {
        InlineString::new()
    }
}

impl PartialEq<InlineString> for InlineString {
    #[inline]
    fn eq(&self, rhs: &InlineString) -> bool {
        self.assert_sanity();
        rhs.assert_sanity();
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

impl_eq! { InlineString, str }
impl_eq! { InlineString, &'a str }
impl_eq! { borrow::Cow<'a, str>, InlineString }

impl InlineString {
    #[cfg_attr(feature = "nightly", allow(inline_always))]
    #[inline(always)]
    fn assert_sanity(&self) {
        debug_assert!(
            self.length as usize <= INLINE_STRING_CAPACITY,
            "inlinable_string: internal error: length greater than capacity"
        );
        debug_assert!(
            str::from_utf8(&self.bytes[0..self.length as usize]).is_ok(),
            "inlinable_string: internal error: contents are not valid UTF-8!"
        );
    }

    /// Creates a new string buffer initialized with the empty string.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let s = InlineString::new();
    /// ```
    #[inline]
    pub fn new() -> InlineString {
        InlineString {
            length: 0,
            bytes: [0; INLINE_STRING_CAPACITY],
        }
    }

    /// Returns the underlying byte buffer, encoded as UTF-8. Trailing bytes are
    /// zeroed.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let s = InlineString::from("hello");
    /// let bytes = s.into_bytes();
    /// assert_eq!(&bytes[0..5], [104, 101, 108, 108, 111]);
    /// ```
    #[inline]
    pub fn into_bytes(mut self) -> [u8; INLINE_STRING_CAPACITY] {
        self.assert_sanity();
        for i in self.len()..INLINE_STRING_CAPACITY {
            self.bytes[i] = 0;
        }
        self.bytes
    }

    /// Pushes the given string onto this string buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("foo");
    /// s.push_str("bar");
    /// assert_eq!(s, "foobar");
    /// ```
    #[inline]
    pub fn push_str(&mut self, string: &str) -> Result<(), NotEnoughSpaceError> {
        self.assert_sanity();

        let string_len = string.len();
        let new_length = self.len() + string_len;

        if new_length > INLINE_STRING_CAPACITY {
            return Err(NotEnoughSpaceError);
        }

        unsafe {
            ptr::copy_nonoverlapping(
                string.as_ptr(),
                self.bytes.as_mut_ptr().offset(self.length as isize),
                string_len,
            );
        }
        self.length = new_length as u8;

        self.assert_sanity();
        Ok(())
    }

    /// Adds the given character to the end of the string.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("abc");
    /// s.push('1');
    /// s.push('2');
    /// s.push('3');
    /// assert_eq!(s, "abc123");
    /// ```
    #[inline]
    pub fn push(&mut self, ch: char) -> Result<(), NotEnoughSpaceError> {
        self.assert_sanity();

        let char_len = ch.len_utf8();
        let new_length = self.len() + char_len;

        if new_length > INLINE_STRING_CAPACITY {
            return Err(NotEnoughSpaceError);
        }

        {
            let mut slice = &mut self.bytes[self.length as usize..INLINE_STRING_CAPACITY];
            ch.encode_utf8(&mut slice);
        }
        self.length = new_length as u8;

        self.assert_sanity();
        Ok(())
    }

    /// Works with the underlying buffer as a byte slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let s = InlineString::from("hello");
    /// assert_eq!(s.as_bytes(), [104, 101, 108, 108, 111]);
    /// ```
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.assert_sanity();
        &self.bytes[0..self.len()]
    }

    /// Shortens a string to the specified length.
    ///
    /// # Panics
    ///
    /// Panics if `new_len` > current length, or if `new_len` is not a character
    /// boundary.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("hello");
    /// s.truncate(2);
    /// assert_eq!(s, "he");
    /// ```
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        self.assert_sanity();

        assert!(
            self.is_char_boundary(new_len),
            "inlinable_string::InlineString::truncate: new_len is not a character
                 boundary"
        );
        assert!(new_len <= self.len());

        self.length = new_len as u8;
        self.assert_sanity();
    }

    /// Removes the last character from the string buffer and returns it.
    /// Returns `None` if this string buffer is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("foo");
    /// assert_eq!(s.pop(), Some('o'));
    /// assert_eq!(s.pop(), Some('o'));
    /// assert_eq!(s.pop(), Some('f'));
    /// assert_eq!(s.pop(), None);
    /// ```
    #[inline]
    pub fn pop(&mut self) -> Option<char> {
        self.assert_sanity();

        match self.char_indices().rev().next() {
            None => None,
            Some((idx, ch)) => {
                self.length = idx as u8;
                self.assert_sanity();
                Some(ch)
            }
        }
    }

    /// Removes the character from the string buffer at byte position `idx` and
    /// returns it.
    ///
    /// # Panics
    ///
    /// If `idx` does not lie on a character boundary, or if it is out of
    /// bounds, then this function will panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("foo");
    /// assert_eq!(s.remove(0), 'f');
    /// assert_eq!(s.remove(1), 'o');
    /// assert_eq!(s.remove(0), 'o');
    /// ```
    #[inline]
    pub fn remove(&mut self, idx: usize) -> char {
        self.assert_sanity();
        assert!(idx < self.len());

        let ch = self
            .get(idx..)
            .expect(
                "inlinable_string::InlineString::remove: idx does not lie on a
                            character boundary",
            )
            .chars()
            .next()
            .expect("Should be `Some` because `idx < self.len()`");
        let char_len = ch.len_utf8();
        let next = idx + char_len;

        unsafe {
            let ptr = self.bytes.as_mut_ptr();

            ptr::copy(
                ptr.add(next),
                ptr.add(idx),
                self.len() - next,
            );
        }
        self.length -= char_len as u8;

        self.assert_sanity();
        ch
    }

    /// Inserts the given bytes at the given position of the string.
    unsafe fn insert_bytes(&mut self, idx: usize, bytes: &[u8]) -> Result<(), NotEnoughSpaceError> {
        let len = self.len();
        let amt = bytes.len();

        // This subtraction does not overflow because `INLINE_STRING_CAPACITY >= self.len()` holds.
        if amt > INLINE_STRING_CAPACITY - len {
            return Err(NotEnoughSpaceError);
        }

        let ptr = self.bytes.as_mut_ptr().add(idx);

        // Shift the latter part.
        ptr::copy(
            ptr,
            ptr.add(amt),
            len - idx,
        );
        // Copy the bytes into the buffer.
        ptr::copy(bytes.as_ptr(), self.bytes.as_mut_ptr().add(idx), amt);
        // `amt` is less than `u8::MAX` becuase `INLINE_STRING_CAPACITY < u8::MAX` holds.
        self.length += amt as u8;

        Ok(())
    }

    /// Inserts a character into the string buffer at byte position `idx`.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("foo");
    /// s.insert(2, 'f');
    /// assert!(s == "fofo");
    /// ```
    ///
    /// # Panics
    ///
    /// If `idx` does not lie on a character boundary or is out of bounds, then
    /// this function will panic.
    #[inline]
    pub fn insert(&mut self, idx: usize, ch: char) -> Result<(), NotEnoughSpaceError> {
        self.assert_sanity();
        assert!(idx <= self.len());

        let mut bits = [0; 4];
        let bits = ch.encode_utf8(&mut bits).as_bytes();

        unsafe {
            self.insert_bytes(idx, bits)?;
        }

        self.assert_sanity();
        Ok(())
    }

    /// Inserts a string into the string buffer at byte position `idx`.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("foo");
    /// s.insert_str(2, "bar");
    /// assert!(s == "fobaro");
    /// ```
    #[inline]
    pub fn insert_str(&mut self, idx: usize, string: &str) -> Result<(), NotEnoughSpaceError> {
        self.assert_sanity();
        assert!(idx <= self.len());

        unsafe {
            self.insert_bytes(idx, string.as_bytes())?;
        }

        self.assert_sanity();
        Ok(())
    }

    /// Views the internal string buffer as a mutable sequence of bytes.
    ///
    /// # Safety
    ///
    /// This is unsafe because it does not check to ensure that the resulting
    /// string will be valid UTF-8.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("hello");
    /// unsafe {
    ///     let slice = s.as_mut_slice();
    ///     assert!(slice == &[104, 101, 108, 108, 111]);
    ///     slice.reverse();
    /// }
    /// assert_eq!(s, "olleh");
    /// ```
    #[inline]
    pub unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
        self.assert_sanity();
        &mut self.bytes[0..self.length as usize]
    }

    /// Returns the number of bytes in this string.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let a = InlineString::from("foo");
    /// assert_eq!(a.len(), 3);
    /// ```
    #[inline]
    pub fn len(&self) -> usize {
        self.assert_sanity();
        self.length as usize
    }

    /// Returns true if the string contains no bytes
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut v = InlineString::new();
    /// assert!(v.is_empty());
    /// v.push('a');
    /// assert!(!v.is_empty());
    /// ```
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.assert_sanity();
        self.length == 0
    }

    /// Truncates the string, returning it to 0 length.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::InlineString;
    ///
    /// let mut s = InlineString::from("foo");
    /// s.clear();
    /// assert!(s.is_empty());
    /// ```
    #[inline]
    pub fn clear(&mut self) {
        self.assert_sanity();
        self.length = 0;
        self.assert_sanity();
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use super::{InlineString, NotEnoughSpaceError, INLINE_STRING_CAPACITY};

    #[test]
    fn test_push_str() {
        let mut s = InlineString::new();
        assert!(s.push_str("small").is_ok());
        assert_eq!(s, "small");

        let long_str = "this is a really long string that is much larger than
                        INLINE_STRING_CAPACITY and so cannot be stored inline.";
        assert_eq!(s.push_str(long_str), Err(NotEnoughSpaceError));
        assert_eq!(s, "small");
    }

    #[test]
    fn test_push() {
        let mut s = InlineString::new();

        for _ in 0..INLINE_STRING_CAPACITY {
            assert!(s.push('a').is_ok());
        }

        assert_eq!(s.push('a'), Err(NotEnoughSpaceError));
    }

    #[test]
    fn test_insert() {
        let mut s = InlineString::new();

        for _ in 0..INLINE_STRING_CAPACITY {
            assert!(s.insert(0, 'a').is_ok());
        }

        assert_eq!(s.insert(0, 'a'), Err(NotEnoughSpaceError));
    }

    #[test]
    fn test_write() {
        use core::fmt::{Error, Write};

        let mut s = InlineString::new();
        let mut normal_string = String::new();

        for _ in 0..INLINE_STRING_CAPACITY {
            assert!(write!(&mut s, "a").is_ok());
            assert!(write!(&mut normal_string, "a").is_ok());
        }

        assert_eq!(write!(&mut s, "a"), Err(Error));
        assert_eq!(&normal_string[..], &s[..]);
    }
}

#[cfg(test)]
#[cfg(feature = "nightly")]
mod benches {
    use test::Bencher;

    #[bench]
    fn its_fast(_b: &mut Bencher) {}
}
