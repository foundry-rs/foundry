use crate::{
    abi::{self, Token, TokenSeq},
    private::SolTypeValue,
    Result, Word,
};
use alloc::{borrow::Cow, vec::Vec};

/// A Solidity type.
///
/// This trait is implemented by types that contain ABI encoding and decoding
/// info for Solidity types. Types may be combined to express arbitrarily
/// complex Solidity types.
///
/// These types are zero cost representations of Solidity types. They do not
/// exist at runtime. They **only** contain information about the type, they do
/// not carry any data.
///
/// # Implementer's Guide
///
/// It should not be necessary to implement this trait manually. Instead, use
/// the [`sol!`] procedural macro to parse Solidity syntax into types that
/// implement this trait.
///
/// # Examples
///
/// Basic usage:
///
/// ```
/// use alloy_sol_types::{sol_data::*, SolType};
///
/// type Uint256DynamicArray = Array<Uint<256>>;
/// assert_eq!(Uint256DynamicArray::sol_type_name(), "uint256[]");
///
/// type Erc20FunctionArgs = (Address, Uint<256>);
/// assert_eq!(Erc20FunctionArgs::sol_type_name(), "(address,uint256)");
///
/// type LargeComplexType = (FixedArray<Array<Bool>, 2>, (FixedBytes<13>, String));
/// assert_eq!(LargeComplexType::sol_type_name(), "(bool[][2],(bytes13,string))");
/// ```
///
/// The previous example can be entirely replicated with the [`sol!`] macro:
///
/// ```
/// use alloy_sol_types::{sol, SolType};
///
/// type Uint256DynamicArray = sol!(uint256[]);
/// assert_eq!(Uint256DynamicArray::sol_type_name(), "uint256[]");
///
/// type Erc20FunctionArgs = sol!((address, uint256));
/// assert_eq!(Erc20FunctionArgs::sol_type_name(), "(address,uint256)");
///
/// type LargeComplexType = sol!((bool[][2],(bytes13,string)));
/// assert_eq!(LargeComplexType::sol_type_name(), "(bool[][2],(bytes13,string))");
/// ```
///
/// For more complex usage, it's recommended to use the
/// [`SolValue`](crate::SolValue) trait for primitive types, and the `Sol*`
/// traits for other types created with [`sol!`]:
///
/// ```
/// use alloy_primitives::Address;
/// use alloy_sol_types::{sol, SolCall, SolStruct, SolValue};
///
/// sol! {
///     struct MyStruct {
///         bool a;
///         uint64 b;
///         address c;
///     }
///
///     enum MyEnum {
///         A,
///         B,
///         C,
///     }
///
///     function myFunction(MyStruct my_struct, MyEnum my_enum);
/// }
///
/// // `SolValue`
/// let my_bool = true;
/// let _ = my_bool.abi_encode();
///
/// let my_struct = MyStruct { a: true, b: 1, c: Address::ZERO };
/// let _ = my_struct.abi_encode();
///
/// let my_enum = MyEnum::A;
/// let _ = my_enum.abi_encode();
///
/// // `SolCall`
/// let my_function_call = myFunctionCall { my_struct, my_enum };
/// let _ = my_function_call.abi_encode();
/// ```
///
/// [`sol!`]: crate::sol
pub trait SolType: Sized {
    /// The corresponding Rust type.
    type RustType: SolTypeValue<Self> + 'static;

    /// The corresponding [ABI token type](Token).
    ///
    /// This is the intermediate representation of the type that is used for
    /// ABI encoding and decoding.
    type Token<'a>: Token<'a>;

    /// The name of this type in Solidity.
    const SOL_NAME: &'static str;

    /// The statically-known ABI-encoded size of the type.
    ///
    /// If this is not known at compile time, this should be `None`, which indicates that the
    /// encoded size is dynamic.
    const ENCODED_SIZE: Option<usize>;

    /// The statically-known Non-standard Packed Mode ABI-encoded size of the type.
    ///
    /// If this is not known at compile time, this should be `None`, which indicates that the
    /// encoded size is dynamic.
    const PACKED_ENCODED_SIZE: Option<usize>;

    /// Whether the ABI-encoded size is dynamic.
    ///
    /// There should be no need to override the default implementation.
    const DYNAMIC: bool = Self::ENCODED_SIZE.is_none();

    /// Returns the name of this type in Solidity.
    #[deprecated(since = "0.6.3", note = "use `SOL_NAME` instead")]
    #[inline]
    fn sol_type_name() -> Cow<'static, str> {
        Self::SOL_NAME.into()
    }

    /// Calculate the ABI-encoded size of the data, counting both head and tail
    /// words. For a single-word type this will always be 32.
    #[inline]
    fn abi_encoded_size<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> usize {
        rust.stv_abi_encoded_size()
    }

    /// Returns `true` if the given token can be detokenized with this type.
    fn valid_token(token: &Self::Token<'_>) -> bool;

    /// Returns an error if the given token cannot be detokenized with this
    /// type.
    #[inline]
    fn type_check(token: &Self::Token<'_>) -> Result<()> {
        if Self::valid_token(token) {
            Ok(())
        } else {
            Err(crate::Error::type_check_fail_token::<Self>(token))
        }
    }

    /// Detokenize this type's value from the given token.
    ///
    /// See the [`abi::token`] module for more information.
    fn detokenize(token: Self::Token<'_>) -> Self::RustType;

    /// Tokenizes the given value into this type's token.
    ///
    /// See the [`abi::token`] module for more information.
    #[inline]
    fn tokenize<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> Self::Token<'_> {
        rust.stv_to_tokens()
    }

    /// Encode this data according to EIP-712 `encodeData` rules, and hash it
    /// if necessary.
    ///
    /// Implementer's note: All single-word types are encoded as their word.
    /// All multi-word types are encoded as the hash the concatenated data
    /// words for each element
    ///
    /// <https://eips.ethereum.org/EIPS/eip-712#definition-of-encodedata>
    #[inline]
    fn eip712_data_word<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> Word {
        rust.stv_eip712_data_word()
    }

    /// Returns the length of this value when ABI-encoded in Non-standard Packed Mode.
    ///
    /// See [`abi_encode_packed`][SolType::abi_encode_packed] for more details.
    #[inline]
    fn abi_packed_encoded_size<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> usize {
        rust.stv_abi_packed_encoded_size()
    }

    /// Non-standard Packed Mode ABI encoding.
    ///
    /// See [`abi_encode_packed`][SolType::abi_encode_packed] for more details.
    #[inline]
    fn abi_encode_packed_to<E: ?Sized + SolTypeValue<Self>>(rust: &E, out: &mut Vec<u8>) {
        rust.stv_abi_encode_packed_to(out)
    }

    /// Non-standard Packed Mode ABI encoding.
    ///
    /// This is different from normal ABI encoding:
    /// - types shorter than 32 bytes are concatenated directly, without padding or sign extension;
    /// - dynamic types are encoded in-place and without the length;
    /// - array elements are padded, but still encoded in-place.
    ///
    /// More information can be found in the [Solidity docs](https://docs.soliditylang.org/en/latest/abi-spec.html#non-standard-packed-mode).
    #[inline]
    fn abi_encode_packed<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::abi_packed_encoded_size(rust));
        Self::abi_encode_packed_to(rust, &mut out);
        out
    }

    /// Tokenizes and ABI-encodes the given value by wrapping it in a
    /// single-element sequence.
    ///
    /// See the [`abi`] module for more information.
    #[inline]
    fn abi_encode<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> Vec<u8> {
        abi::encode(&rust.stv_to_tokens())
    }

    /// Tokenizes and ABI-encodes the given value as function parameters.
    ///
    /// See the [`abi`] module for more information.
    #[inline]
    fn abi_encode_params<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> Vec<u8>
    where
        for<'a> Self::Token<'a>: TokenSeq<'a>,
    {
        abi::encode_params(&rust.stv_to_tokens())
    }

    /// Tokenizes and ABI-encodes the given value as a sequence.
    ///
    /// See the [`abi`] module for more information.
    #[inline]
    fn abi_encode_sequence<E: ?Sized + SolTypeValue<Self>>(rust: &E) -> Vec<u8>
    where
        for<'a> Self::Token<'a>: TokenSeq<'a>,
    {
        abi::encode_sequence(&rust.stv_to_tokens())
    }

    /// Decodes this type's value from an ABI blob by interpreting it as a
    /// single-element sequence.
    ///
    /// See the [`abi`] module for more information.
    #[inline]
    fn abi_decode(data: &[u8], validate: bool) -> Result<Self::RustType> {
        abi::decode::<Self::Token<'_>>(data, validate)
            .and_then(validate_and_detokenize::<Self>(validate))
    }

    /// Decodes this type's value from an ABI blob by interpreting it as
    /// function parameters.
    ///
    /// See the [`abi`] module for more information.
    #[inline]
    fn abi_decode_params<'de>(data: &'de [u8], validate: bool) -> Result<Self::RustType>
    where
        Self::Token<'de>: TokenSeq<'de>,
    {
        abi::decode_params::<Self::Token<'_>>(data, validate)
            .and_then(validate_and_detokenize::<Self>(validate))
    }

    /// Decodes this type's value from an ABI blob by interpreting it as a
    /// sequence.
    ///
    /// See the [`abi`] module for more information.
    #[inline]
    fn abi_decode_sequence<'de>(data: &'de [u8], validate: bool) -> Result<Self::RustType>
    where
        Self::Token<'de>: TokenSeq<'de>,
    {
        abi::decode_sequence::<Self::Token<'_>>(data, validate)
            .and_then(validate_and_detokenize::<Self>(validate))
    }
}

#[inline]
fn validate_and_detokenize<T: SolType>(
    validate: bool,
) -> impl FnOnce(T::Token<'_>) -> Result<T::RustType> {
    move |token| {
        if validate {
            T::type_check(&token)?;
        }
        Ok(T::detokenize(token))
    }
}
