// Copyright 2015-2020 Parity Technologies
// Copyright 2023-2023 Alloy Contributors
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::{
    abi::{Token, TokenSeq},
    utils, Word,
};
use alloc::vec::Vec;
use core::{mem, ptr};

/// An ABI encoder.
///
/// This is not intended for public consumption. It should be used only by the
/// token types. If you have found yourself here, you probably want to use the
/// high-level [`crate::SolType`] interface (or its dynamic equivalent) instead.
#[derive(Clone, Debug, Default)]
pub struct Encoder {
    buf: Vec<Word>,
    suffix_offset: Vec<usize>,
}

impl Encoder {
    /// Instantiate a new empty encoder.
    #[inline]
    pub const fn new() -> Self {
        Self { buf: Vec::new(), suffix_offset: Vec::new() }
    }

    /// Instantiate a new encoder with a given capacity in words.
    #[inline]
    pub fn with_capacity(size: usize) -> Self {
        Self {
            buf: Vec::with_capacity(size),
            // Note: this has to be non-zero even if it won't get used. The compiler will optimize
            // it out, but it won't for `Vec::new` (??).
            suffix_offset: Vec::with_capacity(4),
        }
    }

    /// Return a reference to the encoded words.
    #[inline]
    pub fn words(&self) -> &[Word] {
        &self.buf
    }

    /// Finish the encoding process, returning the encoded words.
    ///
    /// Use `into_bytes` instead to flatten the words into bytes.
    // https://github.com/rust-lang/rust-clippy/issues/4979
    #[allow(clippy::missing_const_for_fn)]
    #[inline]
    pub fn finish(self) -> Vec<Word> {
        self.buf
    }

    /// Return a reference to the encoded bytes.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `#[repr(transparent)] FixedBytes<N>([u8; N])`
        unsafe { &*(self.words() as *const [Word] as *const [[u8; 32]]) }.as_flattened()
    }

    /// Finish the encoding process, returning the encoded bytes.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        // SAFETY: `#[repr(transparent)] FixedBytes<N>([u8; N])`
        unsafe { mem::transmute::<Vec<Word>, Vec<[u8; 32]>>(self.finish()) }.into_flattened()
    }

    /// Determine the current suffix offset.
    ///
    /// # Panics
    ///
    /// Panics if there is no current suffix offset.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn suffix_offset(&self) -> usize {
        debug_assert!(!self.suffix_offset.is_empty());
        unsafe { *self.suffix_offset.last().unwrap_unchecked() }
    }

    /// Appends a suffix offset.
    #[inline]
    pub fn push_offset(&mut self, words: usize) {
        self.suffix_offset.push(words * 32);
    }

    /// Removes the last offset and returns it.
    #[inline]
    pub fn pop_offset(&mut self) -> Option<usize> {
        self.suffix_offset.pop()
    }

    /// Bump the suffix offset by a given number of words.
    #[inline]
    pub fn bump_offset(&mut self, words: usize) {
        if let Some(last) = self.suffix_offset.last_mut() {
            *last += words * 32;
        }
    }

    /// Append a word to the encoder.
    #[inline]
    pub fn append_word(&mut self, word: Word) {
        self.buf.push(word);
    }

    /// Append a pointer to the current suffix offset.
    ///
    /// # Panics
    ///
    /// Panics if there is no current suffix offset.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn append_indirection(&mut self) {
        self.append_word(utils::pad_usize(self.suffix_offset()));
    }

    /// Append a sequence length.
    #[inline]
    pub fn append_seq_len(&mut self, len: usize) {
        self.append_word(utils::pad_usize(len));
    }

    /// Append a sequence of bytes as a packed sequence with a length prefix.
    #[inline]
    pub fn append_packed_seq(&mut self, bytes: &[u8]) {
        self.append_seq_len(bytes.len());
        self.append_bytes(bytes);
    }

    /// Shortcut for appending a token sequence.
    #[inline]
    pub fn append_head_tail<'a, T: TokenSeq<'a>>(&mut self, token: &T) {
        token.encode_sequence(self);
    }

    /// Append a sequence of bytes, padding to the next word.
    #[inline(always)]
    fn append_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        let n_words = utils::words_for(bytes);
        self.buf.reserve(n_words);
        unsafe {
            // set length before copying
            // this is fine because we reserved above and we don't panic below
            let len = self.buf.len();
            self.buf.set_len(len + n_words);

            // copy
            let cnt = bytes.len();
            let dst = self.buf.as_mut_ptr().add(len).cast::<u8>();
            ptr::copy_nonoverlapping(bytes.as_ptr(), dst, cnt);

            // set remaining bytes to zero if necessary
            let rem = cnt % 32;
            if rem != 0 {
                let pad = 32 - rem;
                ptr::write_bytes(dst.add(cnt), 0, pad);
            }
        }
    }
}

/// ABI-encodes a single token.
///
/// You are probably looking for
/// [`SolValue::abi_encode`](crate::SolValue::abi_encode) if
/// you are not intending to use raw tokens.
///
/// See the [`abi`](super) module for more information.
#[inline(always)]
pub fn encode<'a, T: Token<'a>>(token: &T) -> Vec<u8> {
    encode_sequence::<(T,)>(tuple_from_ref(token))
}

/// ABI-encodes a tuple as ABI function params, suitable for passing to a
/// function.
///
/// You are probably looking for
/// [`SolValue::abi_encode_params`](crate::SolValue::abi_encode_params) if
/// you are not intending to use raw tokens.
///
/// See the [`abi`](super) module for more information.
#[inline(always)]
pub fn encode_params<'a, T: TokenSeq<'a>>(token: &T) -> Vec<u8> {
    let encode = const {
        if T::IS_TUPLE {
            encode_sequence
        } else {
            encode
        }
    };
    encode(token)
}

/// ABI-encodes a token sequence.
///
/// You are probably looking for
/// [`SolValue::abi_encode_sequence`](crate::SolValue::abi_encode_sequence) if
/// you are not intending to use raw tokens.
///
/// See the [`abi`](super) module for more information.
#[inline]
pub fn encode_sequence<'a, T: TokenSeq<'a>>(token: &T) -> Vec<u8> {
    let mut enc = Encoder::with_capacity(token.total_words());
    enc.append_head_tail(token);
    enc.into_bytes()
}

/// Converts a reference to `T` into a reference to a tuple of length 1 (without
/// copying).
///
/// Same as [`core::array::from_ref`].
#[inline(always)]
const fn tuple_from_ref<T>(s: &T) -> &(T,) {
    // SAFETY: Converting `&T` to `&(T,)` is sound.
    unsafe { &*(s as *const T).cast::<(T,)>() }
}

#[cfg(test)]
mod tests {
    use crate::{sol_data, SolType};
    use alloc::{borrow::ToOwned, string::ToString, vec::Vec};
    use alloy_primitives::{address, bytes, hex, Address, U256};
    use alloy_sol_macro::sol;

    #[test]
    fn encode_address() {
        let address = Address::from([0x11u8; 20]);
        let expected = hex!("0000000000000000000000001111111111111111111111111111111111111111");
        let encoded = sol_data::Address::abi_encode(&address);
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Address::abi_encoded_size(&address));
    }

    #[test]
    fn encode_dynamic_array_of_addresses() {
        type MyTy = sol_data::Array<sol_data::Address>;
        let data = vec![Address::from([0x11u8; 20]), Address::from([0x22u8; 20])];
        let encoded = MyTy::abi_encode(&data);
        let expected = hex!(
            "
			0000000000000000000000000000000000000000000000000000000000000020
			0000000000000000000000000000000000000000000000000000000000000002
			0000000000000000000000001111111111111111111111111111111111111111
			0000000000000000000000002222222222222222222222222222222222222222
		"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_fixed_array_of_addresses() {
        type MyTy = sol_data::FixedArray<sol_data::Address, 2>;

        let addresses = [Address::from([0x11u8; 20]), Address::from([0x22u8; 20])];

        let encoded = MyTy::abi_encode(&addresses);
        let encoded_params = MyTy::abi_encode_params(&addresses);
        let expected = hex!(
            "
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    	"
        );
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&addresses));
    }

    #[test]
    fn encode_two_addresses() {
        type MyTy = (sol_data::Address, sol_data::Address);
        let addresses = (Address::from([0x11u8; 20]), Address::from([0x22u8; 20]));

        let encoded = MyTy::abi_encode_sequence(&addresses);
        let encoded_params = MyTy::abi_encode_params(&addresses);
        let expected = hex!(
            "
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&addresses));
    }

    #[test]
    fn encode_fixed_array_of_dynamic_array_of_addresses() {
        type MyTy = sol_data::FixedArray<sol_data::Array<sol_data::Address>, 2>;
        let data = [
            vec![Address::from([0x11u8; 20]), Address::from([0x22u8; 20])],
            vec![Address::from([0x33u8; 20]), Address::from([0x44u8; 20])],
        ];

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000040
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000003333333333333333333333333333333333333333
    		0000000000000000000000004444444444444444444444444444444444444444
    	"
        );
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);

        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_array_of_fixed_array_of_addresses() {
        type TwoAddrs = sol_data::FixedArray<sol_data::Address, 2>;
        type MyTy = sol_data::Array<TwoAddrs>;

        let data = vec![
            [Address::from([0x11u8; 20]), Address::from([0x22u8; 20])],
            [Address::from([0x33u8; 20]), Address::from([0x44u8; 20])],
        ];

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    		0000000000000000000000003333333333333333333333333333333333333333
    		0000000000000000000000004444444444444444444444444444444444444444
    	"
        );
        // a DynSeq at top level ALWAYS has extra indirection
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_array_of_dynamic_arrays() {
        type MyTy = sol_data::Array<sol_data::Array<sol_data::Address>>;

        let data = vec![vec![Address::from([0x11u8; 20])], vec![Address::from([0x22u8; 20])]];

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000000000000000000000000000000000000000000040
    		0000000000000000000000000000000000000000000000000000000000000080
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000002222222222222222222222222222222222222222
    	"
        );
        // a DynSeq at top level ALWAYS has extra indirection
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_array_of_dynamic_arrays2() {
        type MyTy = sol_data::Array<sol_data::Array<sol_data::Address>>;

        let data = vec![
            vec![Address::from([0x11u8; 20]), Address::from([0x22u8; 20])],
            vec![Address::from([0x33u8; 20]), Address::from([0x44u8; 20])],
        ];
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000000000000000000000000000000000000000000040
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000003333333333333333333333333333333333333333
    		0000000000000000000000004444444444444444444444444444444444444444
    	"
        );
        // a DynSeq at top level ALWAYS has extra indirection
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_fixed_array_of_fixed_arrays() {
        type MyTy = sol_data::FixedArray<sol_data::FixedArray<sol_data::Address, 2>, 2>;

        let fixed = [
            [Address::from([0x11u8; 20]), Address::from([0x22u8; 20])],
            [Address::from([0x33u8; 20]), Address::from([0x44u8; 20])],
        ];

        let encoded = MyTy::abi_encode_sequence(&fixed);
        let encoded_params = MyTy::abi_encode_params(&fixed);
        let expected = hex!(
            "
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    		0000000000000000000000003333333333333333333333333333333333333333
    		0000000000000000000000004444444444444444444444444444444444444444
    	"
        );
        // a non-dynamic FixedSeq at top level NEVER has extra indirection
        assert_eq!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&fixed));
    }

    #[test]
    fn encode_fixed_array_of_static_tuples_followed_by_dynamic_type() {
        type Tup = (sol_data::Uint<256>, sol_data::Uint<256>, sol_data::Address);
        type Fixed = sol_data::FixedArray<Tup, 2>;
        type MyTy = (Fixed, sol_data::String);

        let data = (
            [
                (U256::from(93523141), U256::from(352332135), Address::from([0x44u8; 20])),
                (U256::from(12411), U256::from(451), Address::from([0x22u8; 20])),
            ],
            "gavofyork".to_string(),
        );

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000005930cc5
    		0000000000000000000000000000000000000000000000000000000015002967
    		0000000000000000000000004444444444444444444444444444444444444444
    		000000000000000000000000000000000000000000000000000000000000307b
    		00000000000000000000000000000000000000000000000000000000000001c3
    		0000000000000000000000002222222222222222222222222222222222222222
    		00000000000000000000000000000000000000000000000000000000000000e0
    		0000000000000000000000000000000000000000000000000000000000000009
    		6761766f66796f726b0000000000000000000000000000000000000000000000
    	"
        );

        let encoded = MyTy::abi_encode(&data);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_ne!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_empty_array() {
        type MyTy0 = sol_data::Array<sol_data::Address>;

        let data: Vec<Address> = vec![];

        // Empty arrays
        let encoded = MyTy0::abi_encode_params(&data);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000000
    	    "
        );

        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), MyTy0::abi_encoded_size(&data));

        type MyTy = (sol_data::Array<sol_data::Address>, sol_data::Array<sol_data::Address>);
        let data: (Vec<Address>, Vec<Address>) = (vec![], vec![]);

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000040
    		0000000000000000000000000000000000000000000000000000000000000060
    		0000000000000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000000
    	    "
        );

        // Empty arrays
        let encoded = MyTy::abi_encode(&data);
        assert_ne!(encoded, expected);

        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));

        type MyTy2 = (
            sol_data::Array<sol_data::Array<sol_data::Address>>,
            sol_data::Array<sol_data::Array<sol_data::Address>>,
        );

        let data: (Vec<Vec<Address>>, Vec<Vec<Address>>) = (vec![vec![]], vec![vec![]]);

        // Nested empty arrays
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000040
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000000
    	"
        );
        // A Dynamic FixedSeq may be a top-level sequence to `encode` or may
        // itself be an item in a top-level sequence. Which is to say, it could
        // be (as `abi_encode(T)` or `abi_encode((T,))`). This test was `abi_encode(T)`
        let encoded = MyTy2::abi_encode(&data);
        assert_ne!(encoded, expected);
        let encoded_params = MyTy2::abi_encode_params(&data);

        assert_eq!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy2::abi_encoded_size(&data));
    }

    #[test]
    fn encode_empty_bytes() {
        let bytes = Vec::<u8>::new();

        let encoded = sol_data::Bytes::abi_encode(&bytes);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000000
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Bytes::abi_encoded_size(&bytes));
    }

    #[test]
    fn encode_bytes() {
        let bytes = vec![0x12, 0x34];

        let encoded = sol_data::Bytes::abi_encode(&bytes);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000002
    		1234000000000000000000000000000000000000000000000000000000000000
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Bytes::abi_encoded_size(&bytes));
    }

    #[test]
    fn encode_fixed_bytes() {
        let encoded = sol_data::FixedBytes::<2>::abi_encode(&[0x12, 0x34]);
        let expected = hex!("1234000000000000000000000000000000000000000000000000000000000000");
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::FixedBytes::<2>::abi_encoded_size(&[0x12, 0x34]));
    }

    #[test]
    fn encode_empty_string() {
        let s = "";
        let encoded = sol_data::String::abi_encode(s);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000000
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::String::abi_encoded_size(&s));
    }

    #[test]
    fn encode_string() {
        let s = "gavofyork".to_string();
        let encoded = sol_data::String::abi_encode(&s);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000009
    		6761766f66796f726b0000000000000000000000000000000000000000000000
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::String::abi_encoded_size(&s));
    }

    #[test]
    fn encode_bytes2() {
        let bytes = hex!("10000000000000000000000000000000000000000000000000000000000002").to_vec();
        let encoded = sol_data::Bytes::abi_encode(&bytes);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		000000000000000000000000000000000000000000000000000000000000001f
    		1000000000000000000000000000000000000000000000000000000000000200
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Bytes::abi_encoded_size(&bytes));
    }

    #[test]
    fn encode_bytes3() {
        let bytes = hex!(
            "
    		1000000000000000000000000000000000000000000000000000000000000000
    		1000000000000000000000000000000000000000000000000000000000000000
    	"
        );
        let encoded = sol_data::Bytes::abi_encode(&bytes);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000040
    		1000000000000000000000000000000000000000000000000000000000000000
    		1000000000000000000000000000000000000000000000000000000000000000
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Bytes::abi_encoded_size(&bytes));
    }

    #[test]
    fn encode_two_bytes() {
        type MyTy = (sol_data::Bytes, sol_data::Bytes);

        let bytes = (
            hex!("10000000000000000000000000000000000000000000000000000000000002").to_vec(),
            hex!("0010000000000000000000000000000000000000000000000000000000000002").to_vec(),
        );
        let encoded = MyTy::abi_encode(&bytes);
        let encoded_params = MyTy::abi_encode_params(&bytes);
        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000040
    		0000000000000000000000000000000000000000000000000000000000000080
    		000000000000000000000000000000000000000000000000000000000000001f
    		1000000000000000000000000000000000000000000000000000000000000200
    		0000000000000000000000000000000000000000000000000000000000000020
    		0010000000000000000000000000000000000000000000000000000000000002
    	"
        );
        // A Dynamic FixedSeq may be a top-level sequence to `encode` or may
        // itself be an item in a top-level sequence. Which is to say, it could
        // be (as `abi_encode(T)` or `abi_encode((T,))`). This test was `abi_encode(T)`
        assert_ne!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&bytes));
    }

    #[test]
    fn encode_uint() {
        let uint = 4;
        let encoded = sol_data::Uint::<8>::abi_encode(&uint);
        let expected = hex!("0000000000000000000000000000000000000000000000000000000000000004");
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Uint::<8>::abi_encoded_size(&uint));
    }

    #[test]
    fn encode_int() {
        let int = 4;
        let encoded = sol_data::Int::<8>::abi_encode(&int);
        let expected = hex!("0000000000000000000000000000000000000000000000000000000000000004");
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Int::<8>::abi_encoded_size(&int));
    }

    #[test]
    fn encode_bool() {
        let encoded = sol_data::Bool::abi_encode(&true);
        let expected = hex!("0000000000000000000000000000000000000000000000000000000000000001");
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Bool::abi_encoded_size(&true));
    }

    #[test]
    fn encode_bool2() {
        let encoded = sol_data::Bool::abi_encode(&false);
        let expected = hex!("0000000000000000000000000000000000000000000000000000000000000000");
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), sol_data::Bool::abi_encoded_size(&false));
    }

    #[test]
    fn comprehensive_test() {
        type MyTy = (sol_data::Uint<8>, sol_data::Bytes, sol_data::Uint<8>, sol_data::Bytes);

        let bytes = hex!(
            "
    		131a3afc00d1b1e3461b955e53fc866dcf303b3eb9f4c16f89e388930f48134b
    		131a3afc00d1b1e3461b955e53fc866dcf303b3eb9f4c16f89e388930f48134b
    	"
        );

        let data = (5, bytes, 3, bytes);

        let encoded = MyTy::abi_encode(&data);
        let encoded_params = MyTy::abi_encode_params(&data);

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000005
    		0000000000000000000000000000000000000000000000000000000000000080
    		0000000000000000000000000000000000000000000000000000000000000003
    		00000000000000000000000000000000000000000000000000000000000000e0
    		0000000000000000000000000000000000000000000000000000000000000040
    		131a3afc00d1b1e3461b955e53fc866dcf303b3eb9f4c16f89e388930f48134b
    		131a3afc00d1b1e3461b955e53fc866dcf303b3eb9f4c16f89e388930f48134b
    		0000000000000000000000000000000000000000000000000000000000000040
    		131a3afc00d1b1e3461b955e53fc866dcf303b3eb9f4c16f89e388930f48134b
    		131a3afc00d1b1e3461b955e53fc866dcf303b3eb9f4c16f89e388930f48134b
    	"
        );
        // A Dynamic FixedSeq may be a top-level sequence to `encode` or may
        // itself be an item in a top-level sequence. Which is to say, it could
        // be (as `abi_encode(T)` or `abi_encode((T,))`). This test was `abi_encode(T)`
        assert_ne!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn comprehensive_test2() {
        type MyTy = (
            sol_data::Bool,
            sol_data::String,
            sol_data::Uint<8>,
            sol_data::Uint<8>,
            sol_data::Uint<8>,
            sol_data::Array<sol_data::Uint<8>>,
        );

        let data = (true, "gavofyork".to_string(), 2, 3, 4, vec![5, 6, 7]);

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000001
    		00000000000000000000000000000000000000000000000000000000000000c0
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000000000000000000000000000000000000000000003
    		0000000000000000000000000000000000000000000000000000000000000004
    		0000000000000000000000000000000000000000000000000000000000000100
    		0000000000000000000000000000000000000000000000000000000000000009
    		6761766f66796f726b0000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000003
    		0000000000000000000000000000000000000000000000000000000000000005
    		0000000000000000000000000000000000000000000000000000000000000006
    		0000000000000000000000000000000000000000000000000000000000000007
    	"
        );
        // A Dynamic FixedSeq may be a top-level sequence to `encode` or may
        // itself be an item in a top-level sequence. Which is to say, it could
        // be (as `abi_encode(T)` or `abi_encode((T,))`). This test was `abi_encode(T)`
        let encoded = MyTy::abi_encode(&data);
        assert_ne!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_array_of_bytes() {
        type MyTy = sol_data::Array<sol_data::Bytes>;
        let data = vec![hex!(
            "019c80031b20d5e69c8093a571162299032018d913930d93ab320ae5ea44a4218a274f00d607"
        )
        .to_vec()];

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000026
    		019c80031b20d5e69c8093a571162299032018d913930d93ab320ae5ea44a421
    		8a274f00d6070000000000000000000000000000000000000000000000000000
    	"
        );
        // a DynSeq at top level ALWAYS has extra indirection
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_array_of_bytes2() {
        type MyTy = sol_data::Array<sol_data::Bytes>;

        let data = vec![
            hex!("4444444444444444444444444444444444444444444444444444444444444444444444444444")
                .to_vec(),
            hex!("6666666666666666666666666666666666666666666666666666666666666666666666666666")
                .to_vec(),
        ];

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000000000000000000000000000000000000000000040
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000000000000000000000000000000000000000000026
    		4444444444444444444444444444444444444444444444444444444444444444
    		4444444444440000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000026
    		6666666666666666666666666666666666666666666666666666666666666666
    		6666666666660000000000000000000000000000000000000000000000000000
    	"
        );
        // a DynSeq at top level ALWAYS has extra indirection
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_static_tuple_of_addresses() {
        type MyTy = (sol_data::Address, sol_data::Address);
        let data = (Address::from([0x11u8; 20]), Address::from([0x22u8; 20]));

        let encoded = MyTy::abi_encode_sequence(&data);
        let encoded_params = MyTy::abi_encode_params(&data);

        let expected = hex!(
            "
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    	"
        );
        assert_eq!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_tuple() {
        type MyTy = (sol_data::String, sol_data::String);
        let data = ("gavofyork".to_string(), "gavofyork".to_string());

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000040
    		0000000000000000000000000000000000000000000000000000000000000080
    		0000000000000000000000000000000000000000000000000000000000000009
    		6761766f66796f726b0000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000009
    		6761766f66796f726b0000000000000000000000000000000000000000000000
    	"
        );
        // a dynamic FixedSeq at top level should start with indirection
        // when not param encoded.
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_ne!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_tuple_of_bytes2() {
        type MyTy = (sol_data::Bytes, sol_data::Bytes);

        let data = (
            hex!("4444444444444444444444444444444444444444444444444444444444444444444444444444")
                .to_vec(),
            hex!("6666666666666666666666666666666666666666666666666666666666666666666666666666")
                .to_vec(),
        );

        let encoded = MyTy::abi_encode(&data);
        let encoded_params = MyTy::abi_encode_params(&data);

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000040
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000000000000000000000000000000000000000000026
    		4444444444444444444444444444444444444444444444444444444444444444
    		4444444444440000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000026
    		6666666666666666666666666666666666666666666666666666666666666666
    		6666666666660000000000000000000000000000000000000000000000000000
    	"
        );
        // a dynamic FixedSeq at top level should start with indirection
        // when not param encoded.
        assert_eq!(encoded, expected);
        assert_ne!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_complex_tuple() {
        type MyTy = (sol_data::Uint<256>, sol_data::String, sol_data::Address, sol_data::Address);

        let data = (
            U256::from_be_bytes::<32>([0x11u8; 32]),
            "gavofyork".to_owned(),
            Address::from([0x11u8; 20]),
            Address::from([0x22u8; 20]),
        );

        let expected = hex!(
            "
            0000000000000000000000000000000000000000000000000000000000000020
            1111111111111111111111111111111111111111111111111111111111111111
            0000000000000000000000000000000000000000000000000000000000000080
            0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    		0000000000000000000000000000000000000000000000000000000000000009
    		6761766f66796f726b0000000000000000000000000000000000000000000000
    	"
        );
        // a dynamic FixedSeq at top level should start with indirection
        // when not param encoded.
        let encoded = MyTy::abi_encode(&data);
        assert_eq!(encoded, expected);
        let encoded_params = MyTy::abi_encode_params(&data);
        assert_ne!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_nested_tuple() {
        type MyTy = (
            sol_data::String,
            sol_data::Bool,
            sol_data::String,
            (sol_data::String, sol_data::String, (sol_data::String, sol_data::String)),
        );

        let data = (
            "test".to_string(),
            true,
            "cyborg".to_string(),
            ("night".to_string(), "day".to_string(), ("weee".to_string(), "funtests".to_string())),
        );

        let encoded = MyTy::abi_encode(&data);
        let encoded_params = MyTy::abi_encode_sequence(&data);

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000080
    		0000000000000000000000000000000000000000000000000000000000000001
    		00000000000000000000000000000000000000000000000000000000000000c0
    		0000000000000000000000000000000000000000000000000000000000000100
    		0000000000000000000000000000000000000000000000000000000000000004
    		7465737400000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000006
    		6379626f72670000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000060
    		00000000000000000000000000000000000000000000000000000000000000a0
    		00000000000000000000000000000000000000000000000000000000000000e0
    		0000000000000000000000000000000000000000000000000000000000000005
    		6e69676874000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000003
    		6461790000000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000040
    		0000000000000000000000000000000000000000000000000000000000000080
    		0000000000000000000000000000000000000000000000000000000000000004
    		7765656500000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000008
    		66756e7465737473000000000000000000000000000000000000000000000000
    	"
        );
        // a dynamic FixedSeq at top level should start with indirection
        // when not param encoded
        assert_eq!(encoded, expected);
        assert_ne!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_params_containing_dynamic_tuple() {
        type MyTy = (
            sol_data::Address,
            (sol_data::Bool, sol_data::String, sol_data::String),
            sol_data::Address,
            sol_data::Address,
            sol_data::Bool,
        );
        let data = (
            Address::from([0x22u8; 20]),
            (true, "spaceship".to_owned(), "cyborg".to_owned()),
            Address::from([0x33u8; 20]),
            Address::from([0x44u8; 20]),
            false,
        );

        let encoded_single = MyTy::abi_encode(&data);
        let encoded = MyTy::abi_encode_sequence(&data);

        let expected = hex!(
            "
    		0000000000000000000000002222222222222222222222222222222222222222
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000003333333333333333333333333333333333333333
    		0000000000000000000000004444444444444444444444444444444444444444
    		0000000000000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000000000000000000000000000000000000000000060
    		00000000000000000000000000000000000000000000000000000000000000a0
    		0000000000000000000000000000000000000000000000000000000000000009
    		7370616365736869700000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000006
    		6379626f72670000000000000000000000000000000000000000000000000000
    	"
        );
        // A Dynamic FixedSeq may be a top-level sequence to `encode` or may
        // itself be an item in a top-level sequence. Which is to say, it could
        // be (as `abi_encode(T)` or `abi_encode((T,))`). This test was `abi_encode(T)`
        assert_ne!(encoded_single, expected);
        assert_eq!(encoded, expected);
        assert_eq!(encoded.len() + 32, encoded_single.len());
        assert_eq!(encoded_single.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_params_containing_static_tuple() {
        type MyTy = (
            sol_data::Address,
            (sol_data::Address, sol_data::Bool, sol_data::Bool),
            sol_data::Address,
            sol_data::Address,
        );

        let data = (
            Address::from([0x11u8; 20]),
            (Address::from([0x22u8; 20]), true, false),
            Address::from([0x33u8; 20]),
            Address::from([0x44u8; 20]),
        );

        let encoded = MyTy::abi_encode_sequence(&data);
        let encoded_params = MyTy::abi_encode_params(&data);

        let expected = hex!(
            "
    		0000000000000000000000001111111111111111111111111111111111111111
    		0000000000000000000000002222222222222222222222222222222222222222
    		0000000000000000000000000000000000000000000000000000000000000001
    		0000000000000000000000000000000000000000000000000000000000000000
    		0000000000000000000000003333333333333333333333333333333333333333
    		0000000000000000000000004444444444444444444444444444444444444444
    	"
        );

        // a static FixedSeq should NEVER indirect
        assert_eq!(encoded, expected);
        assert_eq!(encoded_params, expected);
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    #[test]
    fn encode_dynamic_tuple_with_nested_static_tuples() {
        type MyTy = (((sol_data::Bool, sol_data::Uint<16>),), sol_data::Array<sol_data::Uint<16>>);

        let data = (((false, 0x777),), vec![0x42, 0x1337]);

        let encoded = MyTy::abi_encode(&data);
        let encoded_params = MyTy::abi_encode_params(&data);

        let expected = hex!(
            "
    		0000000000000000000000000000000000000000000000000000000000000020
    		0000000000000000000000000000000000000000000000000000000000000000
    		0000000000000000000000000000000000000000000000000000000000000777
    		0000000000000000000000000000000000000000000000000000000000000060
    		0000000000000000000000000000000000000000000000000000000000000002
    		0000000000000000000000000000000000000000000000000000000000000042
    		0000000000000000000000000000000000000000000000000000000000001337
    	"
        );
        // a dynamic FixedSeq at top level should start with indirection
        // when not param encoded
        assert_eq!(encoded, expected);
        assert_ne!(encoded_params, expected);
        assert_eq!(encoded_params.len() + 32, encoded.len());
        assert_eq!(encoded.len(), MyTy::abi_encoded_size(&data));
    }

    // https://github.com/foundry-rs/foundry/issues/7280
    #[test]
    fn encode_empty_bytes_array_in_tuple() {
        type MyTy = sol! { (bytes, address, bytes[]) };

        let data = (
            Vec::from(bytes!("09736b79736b79736b79026f7300")),
            address!("0xB7b54cd129e6D8B24e6AE652a473449B273eE3E4"),
            Vec::<Vec<u8>>::new(),
        );

        let encoded_params = MyTy::abi_encode_params(&data);

        let expected = hex!(
            "
            0000000000000000000000000000000000000000000000000000000000000060
            000000000000000000000000B7b54cd129e6D8B24e6AE652a473449B273eE3E4
            00000000000000000000000000000000000000000000000000000000000000a0
            000000000000000000000000000000000000000000000000000000000000000e
            09736b79736b79736b79026f7300000000000000000000000000000000000000
            0000000000000000000000000000000000000000000000000000000000000000
    	"
        );
        assert_eq!(encoded_params, expected);
    }
}
