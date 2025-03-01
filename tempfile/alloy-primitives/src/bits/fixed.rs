use crate::aliases;
use core::{fmt, iter, ops, str};
use derive_more::{Deref, DerefMut, From, Index, IndexMut, IntoIterator};
use hex::FromHex;

/// A byte array of fixed length (`[u8; N]`).
///
/// This type allows us to more tightly control serialization, deserialization.
/// rlp encoding, decoding, and other type-level attributes for fixed-length
/// byte arrays.
///
/// Users looking to prevent type-confusion between byte arrays of different
/// lengths should use the [`wrap_fixed_bytes!`](crate::wrap_fixed_bytes) macro
/// to create a new fixed-length byte array type.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Deref,
    DerefMut,
    From,
    Index,
    IndexMut,
    IntoIterator,
)]
#[cfg_attr(feature = "arbitrary", derive(derive_arbitrary::Arbitrary, proptest_derive::Arbitrary))]
#[cfg_attr(feature = "allocative", derive(allocative::Allocative))]
#[repr(transparent)]
pub struct FixedBytes<const N: usize>(#[into_iterator(owned, ref, ref_mut)] pub [u8; N]);

crate::impl_fb_traits!(FixedBytes<N>, N, const);

impl<const N: usize> Default for FixedBytes<N> {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl<const N: usize> Default for &FixedBytes<N> {
    #[inline]
    fn default() -> Self {
        &FixedBytes::ZERO
    }
}

impl<const N: usize> From<&[u8; N]> for FixedBytes<N> {
    #[inline]
    fn from(bytes: &[u8; N]) -> Self {
        Self(*bytes)
    }
}

impl<const N: usize> From<&mut [u8; N]> for FixedBytes<N> {
    #[inline]
    fn from(bytes: &mut [u8; N]) -> Self {
        Self(*bytes)
    }
}

/// Tries to create a `FixedBytes<N>` by copying from a slice `&[u8]`. Succeeds
/// if `slice.len() == N`.
impl<const N: usize> TryFrom<&[u8]> for FixedBytes<N> {
    type Error = core::array::TryFromSliceError;

    #[inline]
    fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
        <&Self>::try_from(slice).copied()
    }
}

/// Tries to create a `FixedBytes<N>` by copying from a mutable slice `&mut
/// [u8]`. Succeeds if `slice.len() == N`.
impl<const N: usize> TryFrom<&mut [u8]> for FixedBytes<N> {
    type Error = core::array::TryFromSliceError;

    #[inline]
    fn try_from(slice: &mut [u8]) -> Result<Self, Self::Error> {
        Self::try_from(&*slice)
    }
}

/// Tries to create a ref `FixedBytes<N>` by copying from a slice `&[u8]`.
/// Succeeds if `slice.len() == N`.
impl<'a, const N: usize> TryFrom<&'a [u8]> for &'a FixedBytes<N> {
    type Error = core::array::TryFromSliceError;

    #[inline]
    fn try_from(slice: &'a [u8]) -> Result<&'a FixedBytes<N>, Self::Error> {
        // SAFETY: `FixedBytes<N>` is `repr(transparent)` for `[u8; N]`
        <&[u8; N]>::try_from(slice).map(|array_ref| unsafe { core::mem::transmute(array_ref) })
    }
}

/// Tries to create a ref `FixedBytes<N>` by copying from a mutable slice `&mut
/// [u8]`. Succeeds if `slice.len() == N`.
impl<'a, const N: usize> TryFrom<&'a mut [u8]> for &'a mut FixedBytes<N> {
    type Error = core::array::TryFromSliceError;

    #[inline]
    fn try_from(slice: &'a mut [u8]) -> Result<&'a mut FixedBytes<N>, Self::Error> {
        // SAFETY: `FixedBytes<N>` is `repr(transparent)` for `[u8; N]`
        <&mut [u8; N]>::try_from(slice).map(|array_ref| unsafe { core::mem::transmute(array_ref) })
    }
}

// Ideally this would be:
// `impl<const N: usize> From<FixedBytes<N>> for Uint<N * 8>`
// `impl<const N: usize> From<Uint<N / 8>> for FixedBytes<N>`
macro_rules! fixed_bytes_uint_conversions {
    ($($int:ty => $fb:ty),* $(,)?) => {$(
        impl From<$int> for $fb {
            /// Converts a fixed-width unsigned integer into a fixed byte array
            /// by interpreting the bytes as big-endian.
            #[inline]
            fn from(value: $int) -> Self {
                Self(value.to_be_bytes())
            }
        }

        impl From<$fb> for $int {
            /// Converts a fixed byte array into a fixed-width unsigned integer
            /// by interpreting the bytes as big-endian.
            #[inline]
            fn from(value: $fb) -> Self {
                Self::from_be_bytes(value.0)
            }
        }

        const _: () = assert!(<$int>::BITS as usize == <$fb>::len_bytes() * 8);
    )*};
}

fixed_bytes_uint_conversions! {
    u8            => aliases::B8,
    aliases::U8   => aliases::B8,
    i8            => aliases::B8,
    aliases::I8   => aliases::B8,

    u16           => aliases::B16,
    aliases::U16  => aliases::B16,
    i16           => aliases::B16,
    aliases::I16  => aliases::B16,

    u32           => aliases::B32,
    aliases::U32  => aliases::B32,
    i32           => aliases::B32,
    aliases::I32  => aliases::B32,

    u64           => aliases::B64,
    aliases::U64  => aliases::B64,
    i64           => aliases::B64,
    aliases::I64  => aliases::B64,

    u128          => aliases::B128,
    aliases::U128 => aliases::B128,
    i128          => aliases::B128,
    aliases::I128 => aliases::B128,

    aliases::U160 => aliases::B160,
    aliases::I160 => aliases::B160,

    aliases::U256 => aliases::B256,
    aliases::I256 => aliases::B256,

    aliases::U512 => aliases::B512,
    aliases::I512 => aliases::B512,

}

impl<const N: usize> From<FixedBytes<N>> for [u8; N] {
    #[inline]
    fn from(s: FixedBytes<N>) -> Self {
        s.0
    }
}

impl<const N: usize> AsRef<[u8; N]> for FixedBytes<N> {
    #[inline]
    fn as_ref(&self) -> &[u8; N] {
        &self.0
    }
}

impl<const N: usize> AsMut<[u8; N]> for FixedBytes<N> {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8; N] {
        &mut self.0
    }
}

impl<const N: usize> AsRef<[u8]> for FixedBytes<N> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl<const N: usize> AsMut<[u8]> for FixedBytes<N> {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl<const N: usize> fmt::Debug for FixedBytes<N> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_hex::<false>(f, true)
    }
}

impl<const N: usize> fmt::Display for FixedBytes<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If the alternate flag is NOT set, we write the full hex.
        if N <= 4 || !f.alternate() {
            return self.fmt_hex::<false>(f, true);
        }

        // If the alternate flag is set, we use middle-out compression.
        const SEP_LEN: usize = '…'.len_utf8();
        let mut buf = [0; 2 + 4 + SEP_LEN + 4];
        buf[0] = b'0';
        buf[1] = b'x';
        hex::encode_to_slice(&self.0[0..2], &mut buf[2..6]).unwrap();
        '…'.encode_utf8(&mut buf[6..]);
        hex::encode_to_slice(&self.0[N - 2..N], &mut buf[6 + SEP_LEN..]).unwrap();

        // SAFETY: always valid UTF-8
        f.write_str(unsafe { str::from_utf8_unchecked(&buf) })
    }
}

impl<const N: usize> fmt::LowerHex for FixedBytes<N> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_hex::<false>(f, f.alternate())
    }
}

impl<const N: usize> fmt::UpperHex for FixedBytes<N> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_hex::<true>(f, f.alternate())
    }
}

impl<const N: usize> ops::BitAnd for FixedBytes<N> {
    type Output = Self;

    #[inline]
    fn bitand(mut self, rhs: Self) -> Self::Output {
        self &= rhs;
        self
    }
}

impl<const N: usize> ops::BitAndAssign for FixedBytes<N> {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        // Note: `slice::Iter` has better codegen than `array::IntoIter`
        iter::zip(self, &rhs).for_each(|(a, b)| *a &= *b);
    }
}

impl<const N: usize> ops::BitOr for FixedBytes<N> {
    type Output = Self;

    #[inline]
    fn bitor(mut self, rhs: Self) -> Self::Output {
        self |= rhs;
        self
    }
}

impl<const N: usize> ops::BitOrAssign for FixedBytes<N> {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        // Note: `slice::Iter` has better codegen than `array::IntoIter`
        iter::zip(self, &rhs).for_each(|(a, b)| *a |= *b);
    }
}

impl<const N: usize> ops::BitXor for FixedBytes<N> {
    type Output = Self;

    #[inline]
    fn bitxor(mut self, rhs: Self) -> Self::Output {
        self ^= rhs;
        self
    }
}

impl<const N: usize> ops::BitXorAssign for FixedBytes<N> {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Self) {
        // Note: `slice::Iter` has better codegen than `array::IntoIter`
        iter::zip(self, &rhs).for_each(|(a, b)| *a ^= *b);
    }
}

impl<const N: usize> ops::Not for FixedBytes<N> {
    type Output = Self;

    #[inline]
    fn not(mut self) -> Self::Output {
        self.iter_mut().for_each(|byte| *byte = !*byte);
        self
    }
}

impl<const N: usize> str::FromStr for FixedBytes<N> {
    type Err = hex::FromHexError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

#[cfg(feature = "rand")]
impl<const N: usize> rand::distributions::Distribution<FixedBytes<N>>
    for rand::distributions::Standard
{
    #[inline]
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> FixedBytes<N> {
        FixedBytes::random_with(rng)
    }
}

impl<const N: usize> FixedBytes<N> {
    /// Array of Zero bytes.
    pub const ZERO: Self = Self([0u8; N]);

    /// Wraps the given byte array in [`FixedBytes`].
    #[inline]
    pub const fn new(bytes: [u8; N]) -> Self {
        Self(bytes)
    }

    /// Creates a new [`FixedBytes`] with the last byte set to `x`.
    #[inline]
    pub const fn with_last_byte(x: u8) -> Self {
        let mut bytes = [0u8; N];
        if N > 0 {
            bytes[N - 1] = x;
        }
        Self(bytes)
    }

    /// Creates a new [`FixedBytes`] where all bytes are set to `byte`.
    #[inline]
    pub const fn repeat_byte(byte: u8) -> Self {
        Self([byte; N])
    }

    /// Returns the size of this byte array (`N`).
    #[inline(always)]
    pub const fn len_bytes() -> usize {
        N
    }

    /// Creates a new [`FixedBytes`] with cryptographically random content.
    ///
    /// # Panics
    ///
    /// Panics if the underlying call to
    /// [`getrandom_uninit`](getrandom::getrandom_uninit) fails.
    #[cfg(feature = "getrandom")]
    #[inline]
    #[track_caller]
    pub fn random() -> Self {
        Self::try_random().unwrap()
    }

    /// Tries to create a new [`FixedBytes`] with cryptographically random
    /// content.
    ///
    /// # Errors
    ///
    /// This function only propagates the error from the underlying call to
    /// [`getrandom_uninit`](getrandom::getrandom_uninit).
    #[cfg(feature = "getrandom")]
    #[inline]
    pub fn try_random() -> Result<Self, getrandom::Error> {
        let mut bytes = Self::ZERO;
        bytes.try_randomize()?;
        Ok(bytes)
    }

    /// Creates a new [`FixedBytes`] with the given random number generator.
    #[cfg(feature = "rand")]
    #[inline]
    #[doc(alias = "random_using")]
    pub fn random_with<R: rand::Rng + ?Sized>(rng: &mut R) -> Self {
        let mut bytes = Self::ZERO;
        bytes.randomize_with(rng);
        bytes
    }

    /// Fills this [`FixedBytes`] with cryptographically random content.
    ///
    /// # Panics
    ///
    /// Panics if the underlying call to
    /// [`getrandom_uninit`](getrandom::getrandom_uninit) fails.
    #[cfg(feature = "getrandom")]
    #[inline]
    #[track_caller]
    pub fn randomize(&mut self) {
        self.try_randomize().unwrap()
    }

    /// Tries to fill this [`FixedBytes`] with cryptographically random content.
    ///
    /// # Errors
    ///
    /// This function only propagates the error from the underlying call to
    /// [`getrandom_uninit`](getrandom::getrandom_uninit).
    #[inline]
    #[cfg(feature = "getrandom")]
    pub fn try_randomize(&mut self) -> Result<(), getrandom::Error> {
        getrandom::getrandom(&mut self.0)
    }

    /// Fills this [`FixedBytes`] with the given random number generator.
    #[cfg(feature = "rand")]
    #[doc(alias = "randomize_using")]
    pub fn randomize_with<R: rand::Rng + ?Sized>(&mut self, rng: &mut R) {
        rng.fill_bytes(&mut self.0);
    }

    /// Concatenate two `FixedBytes`.
    ///
    /// Due to constraints in the language, the user must specify the value of
    /// the output size `Z`.
    ///
    /// # Panics
    ///
    /// Panics if `Z` is not equal to `N + M`.
    pub const fn concat_const<const M: usize, const Z: usize>(
        self,
        other: FixedBytes<M>,
    ) -> FixedBytes<Z> {
        assert!(N + M == Z, "Output size `Z` must equal the sum of the input sizes `N` and `M`");

        let mut result = [0u8; Z];
        let mut i = 0;
        while i < Z {
            result[i] = if i >= N { other.0[i - N] } else { self.0[i] };
            i += 1;
        }
        FixedBytes(result)
    }

    /// Create a new [`FixedBytes`] from the given slice `src`.
    ///
    /// For a fallible version, use the `TryFrom<&[u8]>` implementation.
    ///
    /// # Note
    ///
    /// The given bytes are interpreted in big endian order.
    ///
    /// # Panics
    ///
    /// If the length of `src` and the number of bytes in `Self` do not match.
    #[inline]
    #[track_caller]
    pub fn from_slice(src: &[u8]) -> Self {
        match Self::try_from(src) {
            Ok(x) => x,
            Err(_) => panic!("cannot convert a slice of length {} to FixedBytes<{N}>", src.len()),
        }
    }

    /// Create a new [`FixedBytes`] from the given slice `src`, left-padding it
    /// with zeroes if necessary.
    ///
    /// # Note
    ///
    /// The given bytes are interpreted in big endian order.
    ///
    /// # Panics
    ///
    /// Panics if `src.len() > N`.
    #[inline]
    #[track_caller]
    pub fn left_padding_from(value: &[u8]) -> Self {
        let len = value.len();
        assert!(len <= N, "slice is too large. Expected <={N} bytes, got {len}");
        let mut bytes = Self::ZERO;
        bytes[N - len..].copy_from_slice(value);
        bytes
    }

    /// Create a new [`FixedBytes`] from the given slice `src`, right-padding it
    /// with zeroes if necessary.
    ///
    /// # Note
    ///
    /// The given bytes are interpreted in big endian order.
    ///
    /// # Panics
    ///
    /// Panics if `src.len() > N`.
    #[inline]
    #[track_caller]
    pub fn right_padding_from(value: &[u8]) -> Self {
        let len = value.len();
        assert!(len <= N, "slice is too large. Expected <={N} bytes, got {len}");
        let mut bytes = Self::ZERO;
        bytes[..len].copy_from_slice(value);
        bytes
    }

    /// Returns a slice containing the entire array. Equivalent to `&s[..]`.
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Returns a mutable slice containing the entire array. Equivalent to
    /// `&mut s[..]`.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }

    /// Returns `true` if all bits set in `self` are also set in `b`.
    #[inline]
    pub fn covers(&self, other: &Self) -> bool {
        (*self & *other) == *other
    }

    /// Returns `true` if all bits set in `self` are also set in `b`.
    pub const fn const_covers(self, other: Self) -> bool {
        // (self & other) == other
        other.const_eq(&self.bit_and(other))
    }

    /// Compile-time equality. NOT constant-time equality.
    pub const fn const_eq(&self, other: &Self) -> bool {
        let mut i = 0;
        while i < N {
            if self.0[i] != other.0[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Returns `true` if no bits are set.
    #[inline]
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    /// Returns `true` if no bits are set.
    #[inline]
    pub const fn const_is_zero(&self) -> bool {
        self.const_eq(&Self::ZERO)
    }

    /// Computes the bitwise AND of two `FixedBytes`.
    pub const fn bit_and(self, rhs: Self) -> Self {
        let mut ret = Self::ZERO;
        let mut i = 0;
        while i < N {
            ret.0[i] = self.0[i] & rhs.0[i];
            i += 1;
        }
        ret
    }

    /// Computes the bitwise OR of two `FixedBytes`.
    pub const fn bit_or(self, rhs: Self) -> Self {
        let mut ret = Self::ZERO;
        let mut i = 0;
        while i < N {
            ret.0[i] = self.0[i] | rhs.0[i];
            i += 1;
        }
        ret
    }

    /// Computes the bitwise XOR of two `FixedBytes`.
    pub const fn bit_xor(self, rhs: Self) -> Self {
        let mut ret = Self::ZERO;
        let mut i = 0;
        while i < N {
            ret.0[i] = self.0[i] ^ rhs.0[i];
            i += 1;
        }
        ret
    }

    fn fmt_hex<const UPPER: bool>(&self, f: &mut fmt::Formatter<'_>, prefix: bool) -> fmt::Result {
        let mut buf = hex::Buffer::<N, true>::new();
        let s = if UPPER { buf.format_upper(self) } else { buf.format(self) };
        // SAFETY: The buffer is guaranteed to be at least 2 bytes in length.
        f.write_str(unsafe { s.get_unchecked((!prefix as usize) * 2..) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_fmt {
        ($($fmt:literal, $hex:literal => $expected:literal;)+) => {$(
            assert_eq!(
                format!($fmt, fixed_bytes!($hex)),
                $expected
            );
        )+};
    }

    #[test]
    fn concat_const() {
        const A: FixedBytes<2> = fixed_bytes!("0x0123");
        const B: FixedBytes<2> = fixed_bytes!("0x4567");
        const EXPECTED: FixedBytes<4> = fixed_bytes!("0x01234567");
        const ACTUAL: FixedBytes<4> = A.concat_const(B);

        assert_eq!(ACTUAL, EXPECTED);
    }

    #[test]
    fn display() {
        test_fmt! {
            "{}", "0123456789abcdef" => "0x0123456789abcdef";
            "{:#}", "0123" => "0x0123";
            "{:#}", "01234567" => "0x01234567";
            "{:#}", "0123456789" => "0x0123…6789";
        }
    }

    #[test]
    fn debug() {
        test_fmt! {
            "{:?}", "0123456789abcdef" => "0x0123456789abcdef";
            "{:#?}", "0123456789abcdef" => "0x0123456789abcdef";
        }
    }

    #[test]
    fn lower_hex() {
        test_fmt! {
            "{:x}", "0123456789abcdef" => "0123456789abcdef";
            "{:#x}", "0123456789abcdef" => "0x0123456789abcdef";
        }
    }

    #[test]
    fn upper_hex() {
        test_fmt! {
            "{:X}", "0123456789abcdef" => "0123456789ABCDEF";
            "{:#X}", "0123456789abcdef" => "0x0123456789ABCDEF";
        }
    }

    #[test]
    fn left_padding_from() {
        assert_eq!(FixedBytes::<4>::left_padding_from(&[0x01, 0x23]), fixed_bytes!("0x00000123"));

        assert_eq!(
            FixedBytes::<4>::left_padding_from(&[0x01, 0x23, 0x45, 0x67]),
            fixed_bytes!("0x01234567")
        );
    }

    #[test]
    #[should_panic(expected = "slice is too large. Expected <=4 bytes, got 5")]
    fn left_padding_from_too_large() {
        FixedBytes::<4>::left_padding_from(&[0x01, 0x23, 0x45, 0x67, 0x89]);
    }

    #[test]
    fn right_padding_from() {
        assert_eq!(FixedBytes::<4>::right_padding_from(&[0x01, 0x23]), fixed_bytes!("0x01230000"));

        assert_eq!(
            FixedBytes::<4>::right_padding_from(&[0x01, 0x23, 0x45, 0x67]),
            fixed_bytes!("0x01234567")
        );
    }

    #[test]
    #[should_panic(expected = "slice is too large. Expected <=4 bytes, got 5")]
    fn right_padding_from_too_large() {
        FixedBytes::<4>::right_padding_from(&[0x01, 0x23, 0x45, 0x67, 0x89]);
    }
}
