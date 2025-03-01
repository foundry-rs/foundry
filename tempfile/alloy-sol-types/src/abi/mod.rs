//! Ethereum ABI codec implementation.
//!
//! This module provides the low-level ABI [`Encoder`] and [`Decoder`] structs,
//! along with generic functions for their operation. These utilize an
//! intermediate representation, referred to as tokens. For additional
//! information about tokens, see the [`token`] module documentation.
//!
//! You should not need this module in most cases, as the
//! [`SolType`](crate::SolType) and [`SolValue`](crate::SolValue) traits
//! provide a higher-level and easier to use interface. If you're sure you need
//! the low-level functionality of this module, there are three main interfaces:
//!
//! ### `{encode,decode}`
//!
//! [`encode`] operates on a single token. It wrap this token in a
//! single-element tuple, and passes it to the encoder. Similarly, [`decode`]
//! decodes a single token from a blob by decoding a single-element tuple.
//!
//! Use this interface when ABI-encoding a single token. This is suitable for
//! encoding a type in isolation, or for encoding parameters for single-param
//! functions.
//!
//! ### `{encode,decode}_params`
//!
//! [`encode_params`] operates on a sequence. If the sequence is a tuple, the
//! tuple is inferred to be a set of Solidity function parameters,
//! The corresponding [`decode_params`] reverses this operation, decoding a
//! tuple from a blob.
//!
//! This is used to encode the parameters for a Solidity function.
//!
//! ### `{encode,decode}_sequence`
//!
//! [`encode_sequence`] operates on a sequence of tokens. This sequence is
//! inferred not to be function parameters.
//!
//! This is the least useful one. Most users will not need it.

mod encoder;
pub use encoder::{encode, encode_params, encode_sequence, Encoder};

mod decoder;
pub use decoder::{decode, decode_params, decode_sequence, Decoder, RECURSION_LIMIT};

pub mod token;
pub use token::{Token, TokenSeq};

/// The ABI encoding of an empty byte array (`bytes` or `string`).
pub const EMPTY_BYTES: &[u8; 64] = &alloy_primitives::hex!(
    "0000000000000000000000000000000000000000000000000000000000000020" // offset, points to the next word
    "0000000000000000000000000000000000000000000000000000000000000000" // length, 0
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SolType, SolValue};

    #[test]
    fn empty_bytes() {
        assert_eq!(EMPTY_BYTES.len(), 64);
        assert_eq!(EMPTY_BYTES[..], crate::sol_data::String::abi_encode("")[..]);
        assert_eq!(EMPTY_BYTES[..], crate::sol_data::Bytes::abi_encode(b"")[..]);
        assert_eq!(EMPTY_BYTES[..], "".abi_encode());
        assert_eq!(EMPTY_BYTES[..], b"".abi_encode());
    }
}
