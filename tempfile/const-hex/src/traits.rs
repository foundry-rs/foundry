//! Modified from `hex`.

#![allow(clippy::ptr_as_ptr, clippy::borrow_as_ptr, clippy::missing_errors_doc)]

use core::iter;

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    rc::Rc,
    string::String,
    sync::Arc,
    vec::Vec,
};

/// Encoding values as hex string.
///
/// This trait is implemented for all `T` which implement `AsRef<[u8]>`. This
/// includes `String`, `str`, `Vec<u8>` and `[u8]`.
///
/// # Examples
///
/// ```
/// #![allow(deprecated)]
/// use const_hex::ToHex;
///
/// assert_eq!("Hello world!".encode_hex::<String>(), "48656c6c6f20776f726c6421");
/// assert_eq!("Hello world!".encode_hex_upper::<String>(), "48656C6C6F20776F726C6421");
/// ```
#[cfg_attr(feature = "alloc", doc = "\n[`encode`]: crate::encode")]
#[cfg_attr(not(feature = "alloc"), doc = "\n[`encode`]: crate::encode_to_slice")]
#[deprecated(note = "use `ToHexExt` instead")]
pub trait ToHex {
    /// Encode the hex strict representing `self` into the result.
    /// Lower case letters are used (e.g. `f9b4ca`).
    fn encode_hex<T: iter::FromIterator<char>>(&self) -> T;

    /// Encode the hex strict representing `self` into the result.
    /// Upper case letters are used (e.g. `F9B4CA`).
    fn encode_hex_upper<T: iter::FromIterator<char>>(&self) -> T;
}

/// Encoding values as hex string.
///
/// This trait is implemented for all `T` which implement `AsRef<[u8]>`. This
/// includes `String`, `str`, `Vec<u8>` and `[u8]`.
///
/// # Examples
///
/// ```
/// use const_hex::ToHexExt;
///
/// assert_eq!("Hello world!".encode_hex(), "48656c6c6f20776f726c6421");
/// assert_eq!("Hello world!".encode_hex_upper(), "48656C6C6F20776F726C6421");
/// assert_eq!("Hello world!".encode_hex_with_prefix(), "0x48656c6c6f20776f726c6421");
/// assert_eq!("Hello world!".encode_hex_upper_with_prefix(), "0x48656C6C6F20776F726C6421");
/// ```
#[cfg(feature = "alloc")]
pub trait ToHexExt {
    /// Encode the hex strict representing `self` into the result.
    /// Lower case letters are used (e.g. `f9b4ca`).
    fn encode_hex(&self) -> String;

    /// Encode the hex strict representing `self` into the result.
    /// Upper case letters are used (e.g. `F9B4CA`).
    fn encode_hex_upper(&self) -> String;

    /// Encode the hex strict representing `self` into the result with prefix `0x`.
    /// Lower case letters are used (e.g. `0xf9b4ca`).
    fn encode_hex_with_prefix(&self) -> String;

    /// Encode the hex strict representing `self` into the result with prefix `0X`.
    /// Upper case letters are used (e.g. `0xF9B4CA`).
    fn encode_hex_upper_with_prefix(&self) -> String;
}

struct BytesToHexChars<'a, const UPPER: bool> {
    inner: core::slice::Iter<'a, u8>,
    next: Option<char>,
}

impl<'a, const UPPER: bool> BytesToHexChars<'a, UPPER> {
    fn new(inner: &'a [u8]) -> Self {
        BytesToHexChars {
            inner: inner.iter(),
            next: None,
        }
    }
}

impl<const UPPER: bool> Iterator for BytesToHexChars<'_, UPPER> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next.take() {
            Some(current) => Some(current),
            None => self.inner.next().map(|byte| {
                let (high, low) = crate::byte2hex::<UPPER>(*byte);
                self.next = Some(low as char);
                high as char
            }),
        }
    }
}

#[inline]
fn encode_to_iter<T: iter::FromIterator<char>, const UPPER: bool>(source: &[u8]) -> T {
    BytesToHexChars::<UPPER>::new(source).collect()
}

#[allow(deprecated)]
impl<T: AsRef<[u8]>> ToHex for T {
    #[inline]
    fn encode_hex<U: iter::FromIterator<char>>(&self) -> U {
        encode_to_iter::<_, false>(self.as_ref())
    }

    #[inline]
    fn encode_hex_upper<U: iter::FromIterator<char>>(&self) -> U {
        encode_to_iter::<_, true>(self.as_ref())
    }
}

#[cfg(feature = "alloc")]
impl<T: AsRef<[u8]>> ToHexExt for T {
    #[inline]
    fn encode_hex(&self) -> String {
        crate::encode(self)
    }

    #[inline]
    fn encode_hex_upper(&self) -> String {
        crate::encode_upper(self)
    }

    #[inline]
    fn encode_hex_with_prefix(&self) -> String {
        crate::encode_prefixed(self)
    }

    #[inline]
    fn encode_hex_upper_with_prefix(&self) -> String {
        crate::encode_upper_prefixed(self)
    }
}

/// Types that can be decoded from a hex string.
///
/// This trait is implemented for `Vec<u8>` and small `u8`-arrays.
///
/// # Example
///
/// ```
/// use const_hex::FromHex;
///
/// let buffer = <[u8; 12]>::from_hex("48656c6c6f20776f726c6421")?;
/// assert_eq!(buffer, *b"Hello world!");
/// # Ok::<(), const_hex::FromHexError>(())
/// ```
pub trait FromHex: Sized {
    /// The associated error which can be returned from parsing.
    type Error;

    /// Creates an instance of type `Self` from the given hex string, or fails
    /// with a custom error type.
    ///
    /// Both, upper and lower case characters are valid and can even be
    /// mixed (e.g. `f9b4ca`, `F9B4CA` and `f9B4Ca` are all valid strings).
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>;
}

#[cfg(feature = "alloc")]
impl<T: FromHex> FromHex for Box<T> {
    type Error = T::Error;

    #[inline]
    fn from_hex<U: AsRef<[u8]>>(hex: U) -> Result<Self, Self::Error> {
        FromHex::from_hex(hex.as_ref()).map(Self::new)
    }
}

#[cfg(feature = "alloc")]
impl<T> FromHex for Cow<'_, T>
where
    T: ToOwned + ?Sized,
    T::Owned: FromHex,
{
    type Error = <T::Owned as FromHex>::Error;

    #[inline]
    fn from_hex<U: AsRef<[u8]>>(hex: U) -> Result<Self, Self::Error> {
        FromHex::from_hex(hex.as_ref()).map(Cow::Owned)
    }
}

#[cfg(feature = "alloc")]
impl<T: FromHex> FromHex for Rc<T> {
    type Error = T::Error;

    #[inline]
    fn from_hex<U: AsRef<[u8]>>(hex: U) -> Result<Self, Self::Error> {
        FromHex::from_hex(hex.as_ref()).map(Self::new)
    }
}

#[cfg(feature = "alloc")]
impl<T: FromHex> FromHex for Arc<T> {
    type Error = T::Error;

    #[inline]
    fn from_hex<U: AsRef<[u8]>>(hex: U) -> Result<Self, Self::Error> {
        FromHex::from_hex(hex.as_ref()).map(Self::new)
    }
}

#[cfg(feature = "alloc")]
impl FromHex for Vec<u8> {
    type Error = crate::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        crate::decode(hex.as_ref())
    }
}

#[cfg(feature = "alloc")]
impl FromHex for Vec<i8> {
    type Error = crate::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        // SAFETY: transmuting `u8` to `i8` is safe.
        crate::decode(hex.as_ref()).map(|vec| unsafe { core::mem::transmute::<Vec<u8>, Self>(vec) })
    }
}

#[cfg(feature = "alloc")]
impl FromHex for Box<[u8]> {
    type Error = crate::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        <Vec<u8>>::from_hex(hex).map(Vec::into_boxed_slice)
    }
}

#[cfg(feature = "alloc")]
impl FromHex for Box<[i8]> {
    type Error = crate::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        <Vec<i8>>::from_hex(hex).map(Vec::into_boxed_slice)
    }
}

impl<const N: usize> FromHex for [u8; N] {
    type Error = crate::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        crate::decode_to_array(hex.as_ref())
    }
}

impl<const N: usize> FromHex for [i8; N] {
    type Error = crate::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        // SAFETY: casting `[u8]` to `[i8]` is safe.
        crate::decode_to_array(hex.as_ref())
            .map(|buf| unsafe { *(&buf as *const [u8; N] as *const [i8; N]) })
    }
}
