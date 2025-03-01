// Copyright 2015-2020 Parity Technologies
// Copyright 2023-2023 Alloy Contributors

// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Ethereum ABI tokens.
//!
//! See [`Token`] for more details.

use crate::{
    abi::{Decoder, Encoder},
    Result, Word,
};
use alloc::vec::Vec;
use alloy_primitives::{utils::vec_try_with_capacity, Bytes, FixedBytes, I256, U256};
use core::fmt;

#[allow(unknown_lints, unnameable_types)]
mod sealed {
    pub trait Sealed {}
    impl Sealed for super::WordToken {}
    impl Sealed for () {}
    impl<T, const N: usize> Sealed for super::FixedSeqToken<T, N> {}
    impl<T> Sealed for super::DynSeqToken<T> {}
    impl Sealed for super::PackedSeqToken<'_> {}
}
use sealed::Sealed;

/// Ethereum ABI tokens.
///
/// Tokens are an intermediate state between ABI-encoded blobs, and Rust types.
///
/// ABI encoding uses 5 types:
/// - [`WordToken`]: Single EVM words (a 32-byte string)
/// - [`FixedSeqToken`]: Sequences with a fixed length `T[M]`
/// - [`DynSeqToken`]: Sequences with a dynamic length `T[]`
/// - [`PackedSeqToken`]: Dynamic-length byte arrays `bytes` or `string`
/// - Tuples `(T, U, V, ...)` (implemented for arity `0..=24`)
///
/// A token with a lifetime borrows its data from elsewhere. During decoding,
/// it borrows its data from the decoder. During encoding, it borrows its data
/// from the Rust value being encoded.
///
/// This trait allows us to encode and decode data with minimal copying. It may
/// also be used to enable zero-copy decoding of data, or fast transformation of
/// encoded blobs without full decoding.
///
/// This trait is sealed and cannot be implemented for types outside of this
/// crate. It is implemented only for the types listed above.
pub trait Token<'de>: Sealed + Sized {
    /// True if the token represents a dynamically-sized type.
    const DYNAMIC: bool;

    /// Decode a token from a decoder.
    fn decode_from(dec: &mut Decoder<'de>) -> Result<Self>;

    /// Calculate the number of head words.
    fn head_words(&self) -> usize;

    /// Calculate the number of tail words.
    fn tail_words(&self) -> usize;

    /// Calculate the total number of head and tail words.
    #[inline]
    fn total_words(&self) -> usize {
        self.head_words() + self.tail_words()
    }

    /// Append head words to the encoder.
    fn head_append(&self, enc: &mut Encoder);

    /// Append tail words to the encoder.
    fn tail_append(&self, enc: &mut Encoder);
}

/// A token composed of a sequence of other tokens.
///
/// This functions is an extension trait for [`Token`], and is only
/// implemented by [`FixedSeqToken`], [`DynSeqToken`], [`PackedSeqToken`], and
/// tuples of [`Token`]s (including [`WordToken`]).
pub trait TokenSeq<'a>: Token<'a> {
    /// True for tuples only.
    const IS_TUPLE: bool = false;

    /// ABI-encode the token sequence into the encoder.
    fn encode_sequence(&self, enc: &mut Encoder);

    /// ABI-decode the token sequence from the encoder.
    fn decode_sequence(dec: &mut Decoder<'a>) -> Result<Self>;
}

/// A single EVM word - T for any value type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WordToken(pub Word);

impl<T> From<&T> for WordToken
where
    T: Clone,
    Self: From<T>,
{
    #[inline]
    fn from(value: &T) -> Self {
        Self::from(value.clone())
    }
}

impl<T> From<&mut T> for WordToken
where
    T: Clone,
    Self: From<T>,
{
    #[inline]
    fn from(value: &mut T) -> Self {
        Self::from(value.clone())
    }
}

impl From<Word> for WordToken {
    #[inline]
    fn from(value: Word) -> Self {
        Self(value)
    }
}

impl From<WordToken> for Word {
    #[inline]
    fn from(value: WordToken) -> Self {
        value.0
    }
}

impl From<bool> for WordToken {
    #[inline]
    fn from(value: bool) -> Self {
        U256::from(value as u64).into()
    }
}

impl From<U256> for WordToken {
    #[inline]
    fn from(value: U256) -> Self {
        Self(value.into())
    }
}

impl From<I256> for WordToken {
    #[inline]
    fn from(value: I256) -> Self {
        Self(value.into())
    }
}

impl From<WordToken> for [u8; 32] {
    #[inline]
    fn from(value: WordToken) -> [u8; 32] {
        value.0.into()
    }
}

impl From<[u8; 32]> for WordToken {
    #[inline]
    fn from(value: [u8; 32]) -> Self {
        Self(value.into())
    }
}

impl AsRef<Word> for WordToken {
    #[inline]
    fn as_ref(&self) -> &Word {
        &self.0
    }
}

impl AsRef<[u8]> for WordToken {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0 .0
    }
}

impl<'a> Token<'a> for WordToken {
    const DYNAMIC: bool = false;

    #[inline]
    fn decode_from(dec: &mut Decoder<'a>) -> Result<Self> {
        dec.take_word().copied().map(Self)
    }

    #[inline]
    fn head_words(&self) -> usize {
        1
    }

    #[inline]
    fn tail_words(&self) -> usize {
        0
    }

    #[inline]
    fn head_append(&self, enc: &mut Encoder) {
        enc.append_word(self.0);
    }

    #[inline]
    fn tail_append(&self, _enc: &mut Encoder) {}
}

impl WordToken {
    /// Create a new word token from a word.
    #[inline]
    pub const fn new(array: [u8; 32]) -> Self {
        Self(FixedBytes(array))
    }

    /// Returns a reference to the word as a slice.
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        &self.0 .0
    }
}

/// A Fixed Sequence - `T[N]`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedSeqToken<T, const N: usize>(pub [T; N]);

impl<T, const N: usize> TryFrom<Vec<T>> for FixedSeqToken<T, N> {
    type Error = <[T; N] as TryFrom<Vec<T>>>::Error;

    #[inline]
    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        <[T; N]>::try_from(value).map(Self)
    }
}

impl<T, const N: usize> From<[T; N]> for FixedSeqToken<T, N> {
    #[inline]
    fn from(value: [T; N]) -> Self {
        Self(value)
    }
}

impl<T, const N: usize> AsRef<[T; N]> for FixedSeqToken<T, N> {
    #[inline]
    fn as_ref(&self) -> &[T; N] {
        &self.0
    }
}

impl<'de, T: Token<'de>, const N: usize> Token<'de> for FixedSeqToken<T, N> {
    const DYNAMIC: bool = T::DYNAMIC;

    #[inline]
    fn decode_from(dec: &mut Decoder<'de>) -> Result<Self> {
        if Self::DYNAMIC {
            dec.take_indirection().and_then(|mut child| Self::decode_sequence(&mut child))
        } else {
            Self::decode_sequence(dec)
        }
    }

    #[inline]
    fn head_words(&self) -> usize {
        if Self::DYNAMIC {
            // offset
            1
        } else {
            // elements
            self.0.iter().map(T::total_words).sum()
        }
    }

    #[inline]
    fn tail_words(&self) -> usize {
        if Self::DYNAMIC {
            // elements
            self.0.iter().map(T::total_words).sum()
        } else {
            0
        }
    }

    #[inline]
    fn head_append(&self, enc: &mut Encoder) {
        if Self::DYNAMIC {
            enc.append_indirection();
        } else {
            for inner in &self.0 {
                inner.head_append(enc);
            }
        }
    }

    #[inline]
    fn tail_append(&self, enc: &mut Encoder) {
        if Self::DYNAMIC {
            self.encode_sequence(enc);
        }
    }
}

impl<'de, T: Token<'de>, const N: usize> TokenSeq<'de> for FixedSeqToken<T, N> {
    fn encode_sequence(&self, enc: &mut Encoder) {
        enc.push_offset(self.0.iter().map(T::head_words).sum());

        for inner in &self.0 {
            inner.head_append(enc);
            enc.bump_offset(inner.tail_words());
        }
        for inner in &self.0 {
            inner.tail_append(enc);
        }

        enc.pop_offset();
    }

    #[inline]
    fn decode_sequence(dec: &mut Decoder<'de>) -> Result<Self> {
        crate::impl_core::try_from_fn(|_| T::decode_from(dec)).map(Self)
    }
}

impl<T, const N: usize> FixedSeqToken<T, N> {
    /// Take the backing array, consuming the token.
    // https://github.com/rust-lang/rust-clippy/issues/4979
    #[allow(clippy::missing_const_for_fn)]
    #[inline]
    pub fn into_array(self) -> [T; N] {
        self.0
    }

    /// Returns a reference to the array.
    #[inline]
    pub const fn as_array(&self) -> &[T; N] {
        &self.0
    }

    /// Returns a reference to the array as a slice.
    #[inline]
    pub const fn as_slice(&self) -> &[T] {
        &self.0
    }
}

/// A Dynamic Sequence - `T[]`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynSeqToken<T>(pub Vec<T>);

impl<T> From<Vec<T>> for DynSeqToken<T> {
    #[inline]
    fn from(value: Vec<T>) -> Self {
        Self(value)
    }
}

impl<T> AsRef<[T]> for DynSeqToken<T> {
    #[inline]
    fn as_ref(&self) -> &[T] {
        self.0.as_ref()
    }
}

impl<'de, T: Token<'de>> Token<'de> for DynSeqToken<T> {
    const DYNAMIC: bool = true;

    #[inline]
    fn decode_from(dec: &mut Decoder<'de>) -> Result<Self> {
        let mut child = dec.take_indirection()?;
        let len = child.take_offset()?;
        // This appears to be an unclarity in the Solidity spec. The spec
        // specifies that offsets are relative to the first word of
        // `enc(X)`. But known-good test vectors are relative to the
        // word AFTER the array size
        let mut child = child.raw_child()?;
        let mut tokens = vec_try_with_capacity(len)?;
        for _ in 0..len {
            tokens.push(T::decode_from(&mut child)?);
        }
        Ok(Self(tokens))
    }

    #[inline]
    fn head_words(&self) -> usize {
        // offset
        1
    }

    #[inline]
    fn tail_words(&self) -> usize {
        // length + elements
        1 + self.0.iter().map(T::total_words).sum::<usize>()
    }

    #[inline]
    fn head_append(&self, enc: &mut Encoder) {
        enc.append_indirection();
    }

    #[inline]
    fn tail_append(&self, enc: &mut Encoder) {
        enc.append_seq_len(self.0.len());
        self.encode_sequence(enc);
    }
}

impl<'de, T: Token<'de>> TokenSeq<'de> for DynSeqToken<T> {
    fn encode_sequence(&self, enc: &mut Encoder) {
        enc.push_offset(self.0.iter().map(T::head_words).sum());

        for inner in &self.0 {
            inner.head_append(enc);
            enc.bump_offset(inner.tail_words());
        }
        for inner in &self.0 {
            inner.tail_append(enc);
        }

        enc.pop_offset();
    }

    #[inline]
    fn decode_sequence(dec: &mut Decoder<'de>) -> Result<Self> {
        Self::decode_from(dec)
    }
}

impl<T> DynSeqToken<T> {
    /// Returns a reference to the backing slice.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}

/// A Packed Sequence - `bytes` or `string`
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PackedSeqToken<'a>(pub &'a [u8]);

impl fmt::Debug for PackedSeqToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PackedSeqToken").field(&hex::encode_prefixed(self.0)).finish()
    }
}

impl<'a> From<&'a [u8]> for PackedSeqToken<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self(value)
    }
}

impl<'a> From<&'a Vec<u8>> for PackedSeqToken<'a> {
    fn from(value: &'a Vec<u8>) -> Self {
        Self(value.as_slice())
    }
}

impl AsRef<[u8]> for PackedSeqToken<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl<'de: 'a, 'a> Token<'de> for PackedSeqToken<'a> {
    const DYNAMIC: bool = true;

    #[inline]
    fn decode_from(dec: &mut Decoder<'de>) -> Result<Self> {
        let mut child = dec.take_indirection()?;
        let len = child.take_offset()?;
        let bytes = child.peek_len(len)?;
        Ok(PackedSeqToken(bytes))
    }

    #[inline]
    fn head_words(&self) -> usize {
        // offset
        1
    }

    #[inline]
    fn tail_words(&self) -> usize {
        // length + words(data)
        1 + crate::utils::words_for(self.0)
    }

    #[inline]
    fn head_append(&self, enc: &mut Encoder) {
        enc.append_indirection();
    }

    #[inline]
    fn tail_append(&self, enc: &mut Encoder) {
        enc.append_packed_seq(self.0);
    }
}

impl PackedSeqToken<'_> {
    /// Instantiate a new [`Vec`] by copying the underlying slice.
    // https://github.com/rust-lang/rust-clippy/issues/4979
    #[allow(clippy::missing_const_for_fn)]
    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Instantiate a new [`Bytes`] by copying the underlying slice.
    pub fn into_bytes(self) -> Bytes {
        Bytes::copy_from_slice(self.0)
    }

    /// Returns a reference to the slice.
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        self.0
    }
}

macro_rules! tuple_impls {
    ($count:literal $($ty:ident),+) => {
        impl<'de, $($ty: Token<'de>,)+> Sealed for ($($ty,)+) {}

        #[allow(non_snake_case)]
        impl<'de, $($ty: Token<'de>,)+> Token<'de> for ($($ty,)+) {
            const DYNAMIC: bool = $( <$ty as Token>::DYNAMIC )||+;

            #[inline]
            fn decode_from(dec: &mut Decoder<'de>) -> Result<Self> {
                // The first element in a dynamic tuple is an offset to the tuple's data;
                // for a static tuples, the data begins right away
                if Self::DYNAMIC {
                    dec.take_indirection().and_then(|mut child| Self::decode_sequence(&mut child))
                } else {
                    Self::decode_sequence(dec)
                }
            }

            #[inline]
            fn head_words(&self) -> usize {
                if Self::DYNAMIC {
                    // offset
                    1
                } else {
                    // elements
                    let ($($ty,)+) = self;
                    0 $( + $ty.total_words() )+
                }
            }

            #[inline]
            fn tail_words(&self) -> usize {
                if Self::DYNAMIC {
                    // elements
                    let ($($ty,)+) = self;
                    0 $( + $ty.total_words() )+
                } else {
                    0
                }
            }

            #[inline]
            fn head_append(&self, enc: &mut Encoder) {
                if Self::DYNAMIC {
                    enc.append_indirection();
                } else {
                    let ($($ty,)+) = self;
                    $(
                        $ty.head_append(enc);
                    )+
                }
            }

            #[inline]
            fn tail_append(&self, enc: &mut Encoder) {
                if Self::DYNAMIC {
                    self.encode_sequence(enc);
                }
            }
        }

        #[allow(non_snake_case)]
        impl<'de, $($ty: Token<'de>,)+> TokenSeq<'de> for ($($ty,)+) {
            const IS_TUPLE: bool = true;

            fn encode_sequence(&self, enc: &mut Encoder) {
                let ($($ty,)+) = self;
                enc.push_offset(0 $( + $ty.head_words() )+);

                $(
                    $ty.head_append(enc);
                    enc.bump_offset($ty.tail_words());
                )+

                $(
                    $ty.tail_append(enc);
                )+

                enc.pop_offset();
            }

            #[inline]
            fn decode_sequence(dec: &mut Decoder<'de>) -> Result<Self> {
                Ok(($(
                    match <$ty as Token>::decode_from(dec) {
                        Ok(t) => t,
                        Err(e) => return Err(e),
                    },
                )+))
            }
        }
    };
}

impl<'de> Token<'de> for () {
    const DYNAMIC: bool = false;

    #[inline]
    fn decode_from(_dec: &mut Decoder<'de>) -> Result<Self> {
        Ok(())
    }

    #[inline]
    fn head_words(&self) -> usize {
        0
    }

    #[inline]
    fn tail_words(&self) -> usize {
        0
    }

    #[inline]
    fn head_append(&self, _enc: &mut Encoder) {}

    #[inline]
    fn tail_append(&self, _enc: &mut Encoder) {}
}

impl<'de> TokenSeq<'de> for () {
    const IS_TUPLE: bool = true;

    #[inline]
    fn encode_sequence(&self, _enc: &mut Encoder) {}

    #[inline]
    fn decode_sequence(_dec: &mut Decoder<'de>) -> Result<Self> {
        Ok(())
    }
}

all_the_tuples!(tuple_impls);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sol_data, SolType};
    use alloy_primitives::B256;

    macro_rules! assert_type_check {
        ($sol:ty, $token:expr $(,)?) => {
            assert!(<$sol>::type_check($token).is_ok())
        };
    }

    macro_rules! assert_not_type_check {
        ($sol:ty, $token:expr $(,)?) => {
            assert!(<$sol>::type_check($token).is_err())
        };
    }

    #[test]
    fn test_type_check() {
        assert_type_check!(
            (sol_data::Uint<256>, sol_data::Bool),
            &(WordToken(B256::default()), WordToken(B256::default())),
        );

        // TODO(tests): more like this where we test type check internal logic
        assert_not_type_check!(sol_data::Uint<8>, &Word::repeat_byte(0x11).into());
        assert_not_type_check!(sol_data::Bool, &B256::repeat_byte(0x11).into());
        assert_not_type_check!(sol_data::FixedBytes<31>, &B256::repeat_byte(0x11).into());

        assert_type_check!(
            (sol_data::Uint<32>, sol_data::Bool),
            &(WordToken(B256::default()), WordToken(B256::default())),
        );

        assert_type_check!(
            sol_data::Array<sol_data::Bool>,
            &DynSeqToken(vec![WordToken(B256::default()), WordToken(B256::default()),]),
        );

        assert_type_check!(
            sol_data::Array<sol_data::Bool>,
            &DynSeqToken(vec![WordToken(B256::default()), WordToken(B256::default()),]),
        );
        assert_type_check!(
            sol_data::Array<sol_data::Address>,
            &DynSeqToken(vec![WordToken(B256::default()), WordToken(B256::default()),]),
        );

        assert_type_check!(
            sol_data::FixedArray<sol_data::Bool, 2>,
            &FixedSeqToken::<_, 2>([
                WordToken(B256::default()),
                WordToken(B256::default()),
            ]),
        );

        assert_type_check!(
            sol_data::FixedArray<sol_data::Address, 2>,
            &FixedSeqToken::<_, 2>([
                WordToken(B256::default()),
                WordToken(B256::default()),
            ]),
        );
    }
}
