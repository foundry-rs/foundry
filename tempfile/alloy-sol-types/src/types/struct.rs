//! This module contains the [`SolStruct`] trait, which is used to implement
//! Solidity structs logic, particularly for EIP-712 encoding/decoding.

use super::SolType;
use crate::Eip712Domain;
use alloc::{borrow::Cow, string::String, vec::Vec};
use alloy_primitives::{keccak256, B256};

/// A Solidity struct.
///
/// This trait is used to implement ABI and EIP-712 encoding and decoding.
///
/// # Implementer's Guide
///
/// It should not be necessary to implement this trait manually. Instead, use
/// the [`sol!`](crate::sol!) procedural macro to parse Solidity syntax into
/// types that implement this trait.
///
/// # Note
///
/// Special attention should be paid to [`eip712_encode_type`] for complex
/// Solidity types. Nested Solidity structs **must** properly encode their type.
///
/// To be clear, a struct with a nested struct must encode the nested struct's
/// type as well.
///
/// See [EIP-712#definition-of-encodetype][ref] for more details.
///
/// [`eip712_encode_type`]: SolStruct::eip712_encode_type
/// [ref]: https://eips.ethereum.org/EIPS/eip-712#definition-of-encodetype
pub trait SolStruct: SolType<RustType = Self> {
    /// The struct name.
    ///
    /// Used in [`eip712_encode_type`][SolStruct::eip712_encode_type].
    const NAME: &'static str;

    /// Returns component EIP-712 types. These types are used to construct
    /// the `encodeType` string. These are the types of the struct's fields,
    /// and should not include the root type.
    fn eip712_components() -> Vec<Cow<'static, str>>;

    /// Return the root EIP-712 type. This type is used to construct the
    /// `encodeType` string.
    fn eip712_root_type() -> Cow<'static, str>;

    /// The EIP-712-encoded type string.
    ///
    /// See [EIP-712 `encodeType`](https://eips.ethereum.org/EIPS/eip-712#definition-of-encodetype).
    fn eip712_encode_type() -> Cow<'static, str> {
        fn eip712_encode_types(
            root_type: Cow<'static, str>,
            mut components: Vec<Cow<'static, str>>,
        ) -> Cow<'static, str> {
            if components.is_empty() {
                return root_type;
            }

            components.sort_unstable();
            components.dedup();

            let mut s = String::with_capacity(
                root_type.len() + components.iter().map(|s| s.len()).sum::<usize>(),
            );
            s.push_str(&root_type);
            for component in components {
                s.push_str(&component);
            }
            Cow::Owned(s)
        }

        eip712_encode_types(Self::eip712_root_type(), Self::eip712_components())
    }

    /// Calculates the [EIP-712 `typeHash`](https://eips.ethereum.org/EIPS/eip-712#rationale-for-typehash)
    /// for this struct.
    ///
    /// This is defined as the Keccak-256 hash of the
    /// [`encodeType`](Self::eip712_encode_type) string.
    #[inline]
    fn eip712_type_hash(&self) -> B256 {
        keccak256(Self::eip712_encode_type().as_bytes())
    }

    /// Encodes this domain using [EIP-712 `encodeData`](https://eips.ethereum.org/EIPS/eip-712#definition-of-encodedata).
    fn eip712_encode_data(&self) -> Vec<u8>;

    /// Hashes this struct according to [EIP-712 `hashStruct`](https://eips.ethereum.org/EIPS/eip-712#definition-of-hashstruct).
    #[inline]
    fn eip712_hash_struct(&self) -> B256 {
        let mut hasher = alloy_primitives::Keccak256::new();
        hasher.update(self.eip712_type_hash());
        hasher.update(self.eip712_encode_data());
        hasher.finalize()
    }

    /// Does something.
    ///
    /// See [EIP-712 `signTypedData`](https://eips.ethereum.org/EIPS/eip-712#specification-of-the-eth_signtypeddata-json-rpc).
    #[inline]
    fn eip712_signing_hash(&self, domain: &Eip712Domain) -> B256 {
        let mut digest_input = [0u8; 2 + 32 + 32];
        digest_input[0] = 0x19;
        digest_input[1] = 0x01;
        digest_input[2..34].copy_from_slice(&domain.hash_struct()[..]);
        digest_input[34..66].copy_from_slice(&self.eip712_hash_struct()[..]);
        keccak256(digest_input)
    }
}
