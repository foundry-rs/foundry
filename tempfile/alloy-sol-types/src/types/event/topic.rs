use crate::{abi::token::WordToken, sol_data::*, SolType};
use alloc::vec::Vec;
use alloy_primitives::keccak256;

/// A Solidity event topic.
///
/// These types implement a special encoding used only in Solidity indexed event
/// parameters.
///
/// For more details, see the [Solidity reference][ref].
///
/// [ref]: https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters
///
/// # Implementer's Guide
///
/// It should not be necessary to implement this trait manually. Instead, use
/// the [`sol!`](crate::sol!) procedural macro to parse Solidity syntax into
/// types that implement this trait.
pub trait EventTopic: SolType {
    /// The number of bytes this type occupies in another topic's preimage,
    /// usually a multiple of 32.
    ///
    /// This should be used in conjunction with [`encode_topic_preimage`] to
    /// construct the preimage of a complex topic.
    ///
    /// [`encode_topic_preimage`]: EventTopic::encode_topic_preimage
    fn topic_preimage_length(rust: &Self::RustType) -> usize;

    /// Encodes this type as preimage bytes which are then hashed in
    /// complex types' [`encode_topic`][EventTopic::encode_topic].
    ///
    /// See the [Solidity ABI spec][ref] for more details.
    ///
    /// [ref]: https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters
    fn encode_topic_preimage(rust: &Self::RustType, out: &mut Vec<u8>);

    /// Indexed event parameter encoding.
    ///
    /// Note that this is different from [`encode_topic_preimage`] and
    /// [`SolType::abi_encode`]. See the [Solidity ABI spec][ref] for more
    /// details.
    ///
    /// [`encode_topic_preimage`]: EventTopic::encode_topic_preimage
    /// [ref]: https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters
    fn encode_topic(rust: &Self::RustType) -> WordToken;
}

// Single word types: encoded as just the single word
macro_rules! word_impl {
    () => {
        #[inline]
        fn topic_preimage_length(_: &Self::RustType) -> usize {
            32
        }

        #[inline]
        fn encode_topic_preimage(rust: &Self::RustType, out: &mut Vec<u8>) {
            out.extend($crate::private::SolTypeValue::<Self>::stv_to_tokens(rust).0);
        }

        #[inline]
        fn encode_topic(rust: &Self::RustType) -> WordToken {
            $crate::private::SolTypeValue::<Self>::stv_to_tokens(rust)
        }
    };
}

impl EventTopic for Address {
    word_impl!();
}

impl EventTopic for Function {
    word_impl!();
}

impl EventTopic for Bool {
    word_impl!();
}

impl<const BITS: usize> EventTopic for Int<BITS>
where
    IntBitCount<BITS>: SupportedInt,
{
    word_impl!();
}

impl<const BITS: usize> EventTopic for Uint<BITS>
where
    IntBitCount<BITS>: SupportedInt,
{
    word_impl!();
}

impl<const N: usize> EventTopic for FixedBytes<N>
where
    ByteCount<N>: SupportedFixedBytes,
{
    word_impl!();
}

// Bytes-like types - preimage encoding: bytes padded to 32; hash: the bytes
macro_rules! bytes_impl {
    () => {
        #[inline]
        fn topic_preimage_length(rust: &Self::RustType) -> usize {
            crate::utils::next_multiple_of_32(rust.len())
        }

        #[inline]
        fn encode_topic_preimage(rust: &Self::RustType, out: &mut Vec<u8>) {
            encode_topic_bytes(rust.as_ref(), out);
        }

        #[inline]
        fn encode_topic(rust: &Self::RustType) -> WordToken {
            WordToken(keccak256(rust))
        }
    };
}

impl EventTopic for String {
    bytes_impl!();
}

impl EventTopic for Bytes {
    bytes_impl!();
}

// Complex types - preimage encoding and hash: iter each element
macro_rules! array_impl {
    ($ty:ident) => {
        #[inline]
        fn topic_preimage_length(rust: &Self::RustType) -> usize {
            rust.iter().map($ty::topic_preimage_length).sum()
        }

        #[inline]
        fn encode_topic_preimage(rust: &Self::RustType, out: &mut Vec<u8>) {
            out.reserve(Self::topic_preimage_length(rust));
            for t in rust {
                $ty::encode_topic_preimage(t, out);
            }
        }

        #[inline]
        fn encode_topic(rust: &Self::RustType) -> WordToken {
            let mut out = Vec::new();
            Self::encode_topic_preimage(rust, &mut out);
            WordToken(keccak256(out))
        }
    };
}

impl<T: EventTopic> EventTopic for Array<T> {
    array_impl!(T);
}

impl<T: EventTopic, const N: usize> EventTopic for FixedArray<T, N> {
    array_impl!(T);
}

macro_rules! tuple_impls {
    ($count:literal $($ty:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($ty: EventTopic,)+> EventTopic for ($($ty,)+) {
            #[inline]
            fn topic_preimage_length(rust: &Self::RustType) -> usize {
                let ($($ty,)+) = rust;
                0usize $( + <$ty>::topic_preimage_length($ty) )+
            }

            #[inline]
            fn encode_topic_preimage(rust: &Self::RustType, out: &mut Vec<u8>) {
                let b @ ($($ty,)+) = rust;
                out.reserve(Self::topic_preimage_length(b));
                $(
                    <$ty>::encode_topic_preimage($ty, out);
                )+
            }

            #[inline]
            fn encode_topic(rust: &Self::RustType) -> WordToken {
                let mut out = Vec::new();
                Self::encode_topic_preimage(rust, &mut out);
                WordToken(keccak256(out))
            }
        }
    };
}

impl EventTopic for () {
    #[inline]
    fn topic_preimage_length(_: &Self::RustType) -> usize {
        0
    }

    #[inline]
    fn encode_topic_preimage(_: &Self::RustType, _: &mut Vec<u8>) {}

    #[inline]
    fn encode_topic(_: &Self::RustType) -> WordToken {
        WordToken::default()
    }
}

all_the_tuples!(tuple_impls);

fn encode_topic_bytes(sl: &[u8], out: &mut Vec<u8>) {
    let padding = 32 - sl.len() % 32;
    out.reserve(sl.len() + padding);
    out.extend_from_slice(sl);
    out.extend(core::iter::repeat(0).take(padding));
}
