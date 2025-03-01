// Copyright 2015, The inlinable_string crate Developers. See the COPYRIGHT file
// at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! A trait that exists to abstract string operations over any number of
//! concrete string type implementations.
//!
//! See the [crate level documentation](./../index.html) for more.

use alloc::borrow::{Borrow, Cow};
use alloc::vec::Vec;
use alloc::string::{String, FromUtf16Error, FromUtf8Error};
use core::cmp::PartialEq;
use core::fmt::Display;

/// A trait that exists to abstract string operations over any number of
/// concrete string type implementations.
///
/// See the [crate level documentation](./../index.html) for more.
pub trait StringExt<'a>:
    Borrow<str>
    + Display
    + PartialEq<str>
    + PartialEq<&'a str>
    + PartialEq<String>
    + PartialEq<Cow<'a, str>>
{
    /// Creates a new string buffer initialized with the empty string.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let s = InlinableString::new();
    /// ```
    fn new() -> Self
    where
        Self: Sized;

    /// Creates a new string buffer with the given capacity. The string will be
    /// able to hold at least `capacity` bytes without reallocating. If
    /// `capacity` is less than or equal to `INLINE_STRING_CAPACITY`, the string
    /// will not heap allocate.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let s = InlinableString::with_capacity(10);
    /// ```
    fn with_capacity(capacity: usize) -> Self
    where
        Self: Sized;

    /// Returns the vector as a string buffer, if possible, taking care not to
    /// copy it.
    ///
    /// # Failure
    ///
    /// If the given vector is not valid UTF-8, then the original vector and the
    /// corresponding error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let hello_vec = vec![104, 101, 108, 108, 111];
    /// let s = InlinableString::from_utf8(hello_vec).unwrap();
    /// assert_eq!(s, "hello");
    ///
    /// let invalid_vec = vec![240, 144, 128];
    /// let s = InlinableString::from_utf8(invalid_vec).err().unwrap();
    /// let err = s.utf8_error();
    /// assert_eq!(s.into_bytes(), [240, 144, 128]);
    /// ```
    fn from_utf8(vec: Vec<u8>) -> Result<Self, FromUtf8Error>
    where
        Self: Sized;

    /// Converts a vector of bytes to a new UTF-8 string.
    /// Any invalid UTF-8 sequences are replaced with U+FFFD REPLACEMENT CHARACTER.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let input = b"Hello \xF0\x90\x80World";
    /// let output = InlinableString::from_utf8_lossy(input);
    /// assert_eq!(output, "Hello \u{FFFD}World");
    /// ```
    fn from_utf8_lossy(v: &'a [u8]) -> Cow<'a, str>
    where
        Self: Sized,
    {
        String::from_utf8_lossy(v)
    }

    /// Decode a UTF-16 encoded vector `v` into a `InlinableString`, returning `None`
    /// if `v` contains any invalid data.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// // 𝄞music
    /// let mut v = &mut [0xD834, 0xDD1E, 0x006d, 0x0075,
    ///                   0x0073, 0x0069, 0x0063];
    /// assert_eq!(InlinableString::from_utf16(v).unwrap(),
    ///            InlinableString::from("𝄞music"));
    ///
    /// // 𝄞mu<invalid>ic
    /// v[4] = 0xD800;
    /// assert!(InlinableString::from_utf16(v).is_err());
    /// ```
    fn from_utf16(v: &[u16]) -> Result<Self, FromUtf16Error>
    where
        Self: Sized;

    /// Decode a UTF-16 encoded vector `v` into a string, replacing
    /// invalid data with the replacement character (U+FFFD).
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// // 𝄞mus<invalid>ic<invalid>
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075,
    ///           0x0073, 0xDD1E, 0x0069, 0x0063,
    ///           0xD834];
    ///
    /// assert_eq!(InlinableString::from_utf16_lossy(v),
    ///            InlinableString::from("𝄞mus\u{FFFD}ic\u{FFFD}"));
    /// ```
    fn from_utf16_lossy(v: &[u16]) -> Self
    where
        Self: Sized;

    /// Creates a new `InlinableString` from a length, capacity, and pointer.
    ///
    /// # Safety
    ///
    /// This is _very_ unsafe because:
    ///
    /// * We call `String::from_raw_parts` to get a `Vec<u8>`. Therefore, this
    ///   function inherits all of its unsafety, see [its
    ///   documentation](https://doc.rust-lang.org/nightly/collections/vec/struct.Vec.html#method.from_raw_parts)
    ///   for the invariants it expects, they also apply to this function.
    ///
    /// * We assume that the `Vec` contains valid UTF-8.
    unsafe fn from_raw_parts(buf: *mut u8, length: usize, capacity: usize) -> Self
    where
        Self: Sized;

    /// Converts a vector of bytes to a new `InlinableString` without checking
    /// if it contains valid UTF-8.
    ///
    /// # Safety
    ///
    /// This is unsafe because it assumes that the UTF-8-ness of the vector has
    /// already been validated.
    unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Self
    where
        Self: Sized;

    /// Returns the underlying byte buffer, encoded as UTF-8.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let s = InlinableString::from("hello");
    /// let bytes = s.into_bytes();
    /// assert_eq!(bytes, [104, 101, 108, 108, 111]);
    /// ```
    fn into_bytes(self) -> Vec<u8>;

    /// Pushes the given string onto this string buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// s.push_str("bar");
    /// assert_eq!(s, "foobar");
    /// ```
    fn push_str(&mut self, string: &str);

    /// Returns the number of bytes that this string buffer can hold without
    /// reallocating.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let s = InlinableString::with_capacity(10);
    /// assert!(s.capacity() >= 10);
    /// ```
    fn capacity(&self) -> usize;

    /// Reserves capacity for at least `additional` more bytes to be inserted
    /// in the given `InlinableString`. The collection may reserve more space to avoid
    /// frequent reallocations.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity overflows `usize`.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::new();
    /// s.reserve(10);
    /// assert!(s.capacity() >= 10);
    /// ```
    fn reserve(&mut self, additional: usize);

    /// Reserves the minimum capacity for exactly `additional` more bytes to be
    /// inserted in the given `InlinableString`. Does nothing if the capacity is already
    /// sufficient.
    ///
    /// Note that the allocator may give the collection more space than it
    /// requests. Therefore capacity can not be relied upon to be precisely
    /// minimal. Prefer `reserve` if future insertions are expected.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity overflows `usize`.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::new();
    /// s.reserve_exact(10);
    /// assert!(s.capacity() >= 10);
    /// ```
    fn reserve_exact(&mut self, additional: usize);

    /// Shrinks the capacity of this string buffer to match its length. If the
    /// string's length is less than `INLINE_STRING_CAPACITY` and the string is
    /// heap-allocated, then it is demoted to inline storage.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// s.reserve(100);
    /// assert!(s.capacity() >= 100);
    /// s.shrink_to_fit();
    /// assert_eq!(s.capacity(), inlinable_string::INLINE_STRING_CAPACITY);
    /// ```
    fn shrink_to_fit(&mut self);

    /// Adds the given character to the end of the string.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("abc");
    /// s.push('1');
    /// s.push('2');
    /// s.push('3');
    /// assert_eq!(s, "abc123");
    /// ```
    fn push(&mut self, ch: char);

    /// Works with the underlying buffer as a byte slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let s = InlinableString::from("hello");
    /// assert_eq!(s.as_bytes(), [104, 101, 108, 108, 111]);
    /// ```
    fn as_bytes(&self) -> &[u8];

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
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("hello");
    /// s.truncate(2);
    /// assert_eq!(s, "he");
    /// ```
    fn truncate(&mut self, new_len: usize);

    /// Removes the last character from the string buffer and returns it.
    /// Returns `None` if this string buffer is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// assert_eq!(s.pop(), Some('o'));
    /// assert_eq!(s.pop(), Some('o'));
    /// assert_eq!(s.pop(), Some('f'));
    /// assert_eq!(s.pop(), None);
    /// ```
    fn pop(&mut self) -> Option<char>;

    /// Removes the character from the string buffer at byte position `idx` and
    /// returns it.
    ///
    /// # Warning
    ///
    /// This is an O(n) operation as it requires copying every element in the
    /// buffer.
    ///
    /// # Panics
    ///
    /// If `idx` does not lie on a character boundary, or if it is out of
    /// bounds, then this function will panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// assert_eq!(s.remove(0), 'f');
    /// assert_eq!(s.remove(1), 'o');
    /// assert_eq!(s.remove(0), 'o');
    /// ```
    fn remove(&mut self, idx: usize) -> char;

    /// Inserts a character into the string buffer at byte position `idx`.
    ///
    /// # Warning
    ///
    /// This is an O(n) operation as it requires copying every element in the
    /// buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// s.insert(2, 'f');
    /// assert!(s == "fofo");
    /// ```
    ///
    /// # Panics
    ///
    /// If `idx` does not lie on a character boundary or is out of bounds, then
    /// this function will panic.
    fn insert(&mut self, idx: usize, ch: char);

    /// Inserts a string into the string buffer at byte position `idx`.
    ///
    /// # Warning
    ///
    /// This is an O(n) operation as it requires copying every element in the
    /// buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// s.insert_str(2, "bar");
    /// assert!(s == "fobaro");
    /// ```
    ///
    /// # Panics
    ///
    /// If `idx` does not lie on a character boundary or is out of bounds, then
    /// this function will panic.
    fn insert_str(&mut self, idx: usize, string: &str);

    /// Views the string buffer as a mutable sequence of bytes.
    ///
    /// # Safety
    ///
    /// This is unsafe because it does not check to ensure that the resulting
    /// string will be valid UTF-8.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("hello");
    /// unsafe {
    ///     let slice = s.as_mut_slice();
    ///     assert!(slice == &[104, 101, 108, 108, 111]);
    ///     slice.reverse();
    /// }
    /// assert_eq!(s, "olleh");
    /// ```
    unsafe fn as_mut_slice(&mut self) -> &mut [u8];

    /// Returns the number of bytes in this string.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let a = InlinableString::from("foo");
    /// assert_eq!(a.len(), 3);
    /// ```
    fn len(&self) -> usize;

    /// Returns true if the string contains no bytes
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut v = InlinableString::new();
    /// assert!(v.is_empty());
    /// v.push('a');
    /// assert!(!v.is_empty());
    /// ```
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Truncates the string, returning it to 0 length.
    ///
    /// # Examples
    ///
    /// ```
    /// use inlinable_string::{InlinableString, StringExt};
    ///
    /// let mut s = InlinableString::from("foo");
    /// s.clear();
    /// assert!(s.is_empty());
    /// ```
    #[inline]
    fn clear(&mut self) {
        self.truncate(0);
    }
}

impl<'a> StringExt<'a> for String {
    #[inline]
    fn new() -> Self {
        String::new()
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        String::with_capacity(capacity)
    }

    #[inline]
    fn from_utf8(vec: Vec<u8>) -> Result<Self, FromUtf8Error> {
        String::from_utf8(vec)
    }

    #[inline]
    fn from_utf16(v: &[u16]) -> Result<Self, FromUtf16Error> {
        String::from_utf16(v)
    }

    #[inline]
    fn from_utf16_lossy(v: &[u16]) -> Self {
        String::from_utf16_lossy(v)
    }

    #[inline]
    unsafe fn from_raw_parts(buf: *mut u8, length: usize, capacity: usize) -> Self {
        String::from_raw_parts(buf, length, capacity)
    }

    #[inline]
    unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Self {
        String::from_utf8_unchecked(bytes)
    }

    #[inline]
    fn into_bytes(self) -> Vec<u8> {
        String::into_bytes(self)
    }

    #[inline]
    fn push_str(&mut self, string: &str) {
        String::push_str(self, string)
    }

    #[inline]
    fn capacity(&self) -> usize {
        String::capacity(self)
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        String::reserve(self, additional)
    }

    #[inline]
    fn reserve_exact(&mut self, additional: usize) {
        String::reserve_exact(self, additional)
    }

    #[inline]
    fn shrink_to_fit(&mut self) {
        String::shrink_to_fit(self)
    }

    #[inline]
    fn push(&mut self, ch: char) {
        String::push(self, ch)
    }

    #[inline]
    fn as_bytes(&self) -> &[u8] {
        String::as_bytes(self)
    }

    #[inline]
    fn truncate(&mut self, new_len: usize) {
        String::truncate(self, new_len)
    }

    #[inline]
    fn pop(&mut self) -> Option<char> {
        String::pop(self)
    }

    #[inline]
    fn remove(&mut self, idx: usize) -> char {
        String::remove(self, idx)
    }

    #[inline]
    fn insert(&mut self, idx: usize, ch: char) {
        String::insert(self, idx, ch)
    }

    #[inline]
    fn insert_str(&mut self, idx: usize, string: &str) {
        String::insert_str(self, idx, string)
    }

    #[inline]
    unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut *(self.as_mut_str() as *mut str as *mut [u8])
    }

    #[inline]
    fn len(&self) -> usize {
        String::len(self)
    }
}

#[cfg(test)]
mod std_string_stringext_sanity_tests {
    // Sanity tests for std::string::String's StringExt implementation.

    use alloc::string::String;
    use super::StringExt;

    #[test]
    fn test_new() {
        let s = <String as StringExt>::new();
        assert!(StringExt::is_empty(&s));
    }

    #[test]
    fn test_with_capacity() {
        let s = <String as StringExt>::with_capacity(10);
        assert!(StringExt::capacity(&s) >= 10);
    }

    #[test]
    fn test_from_utf8() {
        let s = <String as StringExt>::from_utf8(vec![104, 101, 108, 108, 111]);
        assert_eq!(s.unwrap(), "hello");
    }

    #[test]
    fn test_from_utf16() {
        let v = &mut [0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0x0069, 0x0063];
        let s = <String as StringExt>::from_utf16(v);
        assert_eq!(s.unwrap(), "𝄞music");
    }

    #[test]
    fn test_from_utf16_lossy() {
        let input = b"Hello \xF0\x90\x80World";
        let output = <String as StringExt>::from_utf8_lossy(input);
        assert_eq!(output, "Hello \u{FFFD}World");
    }

    #[test]
    fn test_into_bytes() {
        let s = String::from("hello");
        let bytes = StringExt::into_bytes(s);
        assert_eq!(bytes, [104, 101, 108, 108, 111]);
    }

    #[test]
    fn test_push_str() {
        let mut s = String::from("hello");
        StringExt::push_str(&mut s, " world");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_capacity() {
        let s = <String as StringExt>::with_capacity(100);
        assert!(String::capacity(&s) >= 100);
    }

    #[test]
    fn test_reserve() {
        let mut s = <String as StringExt>::new();
        StringExt::reserve(&mut s, 100);
        assert!(String::capacity(&s) >= 100);
    }

    #[test]
    fn test_reserve_exact() {
        let mut s = <String as StringExt>::new();
        StringExt::reserve_exact(&mut s, 100);
        assert!(String::capacity(&s) >= 100);
    }

    #[test]
    fn test_shrink_to_fit() {
        let mut s = <String as StringExt>::with_capacity(100);
        StringExt::push_str(&mut s, "foo");
        StringExt::shrink_to_fit(&mut s);
        assert_eq!(String::capacity(&s), 3);
    }

    #[test]
    fn test_push() {
        let mut s = String::new();
        StringExt::push(&mut s, 'a');
        assert_eq!(s, "a");
    }

    #[test]
    fn test_truncate() {
        let mut s = String::from("foo");
        StringExt::truncate(&mut s, 1);
        assert_eq!(s, "f");
    }

    #[test]
    fn test_pop() {
        let mut s = String::from("foo");
        assert_eq!(StringExt::pop(&mut s), Some('o'));
        assert_eq!(StringExt::pop(&mut s), Some('o'));
        assert_eq!(StringExt::pop(&mut s), Some('f'));
        assert_eq!(StringExt::pop(&mut s), None);
    }
}
