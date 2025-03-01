use crate::FixedBytes;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::{
    borrow::Borrow,
    fmt,
    ops::{Deref, DerefMut, RangeBounds},
};

#[cfg(feature = "rlp")]
mod rlp;

#[cfg(feature = "serde")]
mod serde;

/// Wrapper type around [`bytes::Bytes`] to support "0x" prefixed hex strings.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Bytes(pub bytes::Bytes);

impl Default for &Bytes {
    #[inline]
    fn default() -> Self {
        static EMPTY: Bytes = Bytes::new();
        &EMPTY
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(self, f)
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(self, f)
    }
}

impl fmt::LowerHex for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&hex::encode_prefixed(self.as_ref()))
    }
}

impl fmt::UpperHex for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&hex::encode_upper_prefixed(self.as_ref()))
    }
}

impl Deref for Bytes {
    type Target = bytes::Bytes;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Bytes {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<[u8]> for Bytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Borrow<[u8]> for Bytes {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl FromIterator<u8> for Bytes {
    #[inline]
    fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
        Self(bytes::Bytes::from_iter(iter))
    }
}

impl<'a> FromIterator<&'a u8> for Bytes {
    #[inline]
    fn from_iter<T: IntoIterator<Item = &'a u8>>(iter: T) -> Self {
        Self(iter.into_iter().copied().collect::<bytes::Bytes>())
    }
}

impl IntoIterator for Bytes {
    type Item = u8;
    type IntoIter = bytes::buf::IntoIter<bytes::Bytes>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Bytes {
    type Item = &'a u8;
    type IntoIter = core::slice::Iter<'a, u8>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<bytes::Bytes> for Bytes {
    #[inline]
    fn from(value: bytes::Bytes) -> Self {
        Self(value)
    }
}

impl From<Bytes> for bytes::Bytes {
    #[inline]
    fn from(value: Bytes) -> Self {
        value.0
    }
}

impl From<Vec<u8>> for Bytes {
    #[inline]
    fn from(value: Vec<u8>) -> Self {
        Self(value.into())
    }
}

impl<const N: usize> From<FixedBytes<N>> for Bytes {
    #[inline]
    fn from(value: FixedBytes<N>) -> Self {
        value.to_vec().into()
    }
}

impl<const N: usize> From<&'static FixedBytes<N>> for Bytes {
    #[inline]
    fn from(value: &'static FixedBytes<N>) -> Self {
        Self::from_static(value.as_slice())
    }
}

impl<const N: usize> From<[u8; N]> for Bytes {
    #[inline]
    fn from(value: [u8; N]) -> Self {
        value.to_vec().into()
    }
}

impl<const N: usize> From<&'static [u8; N]> for Bytes {
    #[inline]
    fn from(value: &'static [u8; N]) -> Self {
        Self::from_static(value)
    }
}

impl From<&'static [u8]> for Bytes {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        Self::from_static(value)
    }
}

impl From<&'static str> for Bytes {
    #[inline]
    fn from(value: &'static str) -> Self {
        Self::from_static(value.as_bytes())
    }
}

impl From<Box<[u8]>> for Bytes {
    #[inline]
    fn from(value: Box<[u8]>) -> Self {
        Self(value.into())
    }
}

impl From<String> for Bytes {
    #[inline]
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

impl From<Bytes> for Vec<u8> {
    #[inline]
    fn from(value: Bytes) -> Self {
        value.0.into()
    }
}

impl PartialEq<[u8]> for Bytes {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        self[..] == *other
    }
}

impl PartialEq<Bytes> for [u8] {
    #[inline]
    fn eq(&self, other: &Bytes) -> bool {
        *self == other[..]
    }
}

impl PartialEq<Vec<u8>> for Bytes {
    #[inline]
    fn eq(&self, other: &Vec<u8>) -> bool {
        self[..] == other[..]
    }
}

impl PartialEq<Bytes> for Vec<u8> {
    #[inline]
    fn eq(&self, other: &Bytes) -> bool {
        *other == *self
    }
}

impl PartialEq<bytes::Bytes> for Bytes {
    #[inline]
    fn eq(&self, other: &bytes::Bytes) -> bool {
        other == self.as_ref()
    }
}

impl core::str::FromStr for Bytes {
    type Err = hex::FromHexError;

    #[inline]
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        hex::decode(value).map(Into::into)
    }
}

impl hex::FromHex for Bytes {
    type Error = hex::FromHexError;

    #[inline]
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        hex::decode(hex).map(Self::from)
    }
}

impl bytes::Buf for Bytes {
    #[inline]
    fn remaining(&self) -> usize {
        self.0.len()
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        self.0.chunk()
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        self.0.advance(cnt)
    }

    #[inline]
    fn copy_to_bytes(&mut self, len: usize) -> bytes::Bytes {
        self.0.copy_to_bytes(len)
    }
}

impl Bytes {
    /// Creates a new empty `Bytes`.
    ///
    /// This will not allocate and the returned `Bytes` handle will be empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_primitives::Bytes;
    ///
    /// let b = Bytes::new();
    /// assert_eq!(&b[..], b"");
    /// ```
    #[inline]
    pub const fn new() -> Self {
        Self(bytes::Bytes::new())
    }

    /// Creates a new `Bytes` from a static slice.
    ///
    /// The returned `Bytes` will point directly to the static slice. There is
    /// no allocating or copying.
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_primitives::Bytes;
    ///
    /// let b = Bytes::from_static(b"hello");
    /// assert_eq!(&b[..], b"hello");
    /// ```
    #[inline]
    pub const fn from_static(bytes: &'static [u8]) -> Self {
        Self(bytes::Bytes::from_static(bytes))
    }

    /// Creates a new `Bytes` instance from a slice by copying it.
    #[inline]
    pub fn copy_from_slice(data: &[u8]) -> Self {
        Self(bytes::Bytes::copy_from_slice(data))
    }

    /// Returns a slice of self for the provided range.
    #[inline]
    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        Self(self.0.slice(range))
    }

    /// Returns a slice of self that is equivalent to the given `subset`.
    #[inline]
    pub fn slice_ref(&self, subset: &[u8]) -> Self {
        Self(self.0.slice_ref(subset))
    }

    /// Splits the bytes into two at the given index.
    #[must_use = "consider Bytes::truncate if you don't need the other half"]
    #[inline]
    pub fn split_off(&mut self, at: usize) -> Self {
        Self(self.0.split_off(at))
    }

    /// Splits the bytes into two at the given index.
    #[must_use = "consider Bytes::advance if you don't need the other half"]
    #[inline]
    pub fn split_to(&mut self, at: usize) -> Self {
        Self(self.0.split_to(at))
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for Bytes {
    #[inline]
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        u.arbitrary_iter()?.collect::<arbitrary::Result<Vec<u8>>>().map(Into::into)
    }

    #[inline]
    fn arbitrary_take_rest(u: arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self(u.take_rest().to_vec().into()))
    }

    #[inline]
    fn size_hint(_depth: usize) -> (usize, Option<usize>) {
        (0, None)
    }
}

#[cfg(feature = "arbitrary")]
impl proptest::arbitrary::Arbitrary for Bytes {
    type Parameters = proptest::arbitrary::ParamsFor<Vec<u8>>;
    type Strategy = proptest::arbitrary::Mapped<Vec<u8>, Self>;

    #[inline]
    fn arbitrary() -> Self::Strategy {
        use proptest::strategy::Strategy;
        proptest::arbitrary::any::<Vec<u8>>().prop_map(|vec| Self(vec.into()))
    }

    #[inline]
    fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Strategy;
        proptest::arbitrary::any_with::<Vec<u8>>(args).prop_map(|vec| Self(vec.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let expected = Bytes::from_static(&[0x12, 0x13, 0xab, 0xcd]);
        assert_eq!("1213abcd".parse::<Bytes>().unwrap(), expected);
        assert_eq!("0x1213abcd".parse::<Bytes>().unwrap(), expected);
        assert_eq!("1213ABCD".parse::<Bytes>().unwrap(), expected);
        assert_eq!("0x1213ABCD".parse::<Bytes>().unwrap(), expected);
    }

    #[test]
    fn format() {
        let b = Bytes::from_static(&[1, 35, 69, 103, 137, 171, 205, 239]);
        assert_eq!(format!("{b}"), "0x0123456789abcdef");
        assert_eq!(format!("{b:x}"), "0x0123456789abcdef");
        assert_eq!(format!("{b:?}"), "0x0123456789abcdef");
        assert_eq!(format!("{b:#?}"), "0x0123456789abcdef");
        assert_eq!(format!("{b:#x}"), "0x0123456789abcdef");
        assert_eq!(format!("{b:X}"), "0x0123456789ABCDEF");
        assert_eq!(format!("{b:#X}"), "0x0123456789ABCDEF");
    }
}
