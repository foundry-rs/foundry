use crate::{
    abi::token::{PackedSeqToken, Token, TokenSeq, WordToken},
    types::interface::RevertReason,
    Result, SolType, Word,
};
use alloc::{string::String, vec::Vec};
use alloy_primitives::U256;
use core::{borrow::Borrow, fmt};

/// A Solidity custom error.
///
/// # Implementer's Guide
///
/// It should not be necessary to implement this trait manually. Instead, use
/// the [`sol!`](crate::sol!) procedural macro to parse Solidity syntax into
/// types that implement this trait.
pub trait SolError: Sized {
    /// The underlying tuple type which represents the error's members.
    ///
    /// If the error has no arguments, this will be the unit type `()`
    type Parameters<'a>: SolType<Token<'a> = Self::Token<'a>>;

    /// The corresponding [`TokenSeq`] type.
    type Token<'a>: TokenSeq<'a>;

    /// The error's ABI signature.
    const SIGNATURE: &'static str;

    /// The error selector: `keccak256(SIGNATURE)[0..4]`
    const SELECTOR: [u8; 4];

    /// Convert from the tuple type used for ABI encoding and decoding.
    fn new(tuple: <Self::Parameters<'_> as SolType>::RustType) -> Self;

    /// Convert to the token type used for EIP-712 encoding and decoding.
    fn tokenize(&self) -> Self::Token<'_>;

    /// The size of the error params when encoded in bytes, **without** the
    /// selector.
    #[inline]
    fn abi_encoded_size(&self) -> usize {
        if let Some(size) = <Self::Parameters<'_> as SolType>::ENCODED_SIZE {
            return size;
        }

        // `total_words` includes the first dynamic offset which we ignore.
        let offset = <<Self::Parameters<'_> as SolType>::Token<'_> as Token>::DYNAMIC as usize * 32;
        (self.tokenize().total_words() * Word::len_bytes()).saturating_sub(offset)
    }

    /// ABI decode this call's arguments from the given slice, **without** its
    /// selector.
    #[inline]
    fn abi_decode_raw(data: &[u8], validate: bool) -> Result<Self> {
        <Self::Parameters<'_> as SolType>::abi_decode_sequence(data, validate).map(Self::new)
    }

    /// ABI decode this error's arguments from the given slice, **with** the
    /// selector.
    #[inline]
    fn abi_decode(data: &[u8], validate: bool) -> Result<Self> {
        let data = data
            .strip_prefix(&Self::SELECTOR)
            .ok_or_else(|| crate::Error::type_check_fail_sig(data, Self::SIGNATURE))?;
        Self::abi_decode_raw(data, validate)
    }

    /// ABI encode the error to the given buffer **without** its selector.
    #[inline]
    fn abi_encode_raw(&self, out: &mut Vec<u8>) {
        out.reserve(self.abi_encoded_size());
        out.extend(crate::abi::encode_sequence(&self.tokenize()));
    }

    /// ABI encode the error to the given buffer **with** its selector.
    #[inline]
    fn abi_encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.abi_encoded_size());
        out.extend(&Self::SELECTOR);
        self.abi_encode_raw(&mut out);
        out
    }
}

/// Represents a standard Solidity revert. These are thrown by `revert(reason)`
/// or `require(condition, reason)` statements in Solidity.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Revert {
    /// The reason string, provided by the Solidity contract.
    pub reason: String,
}

impl fmt::Debug for Revert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Revert").field(&self.reason).finish()
    }
}

impl fmt::Display for Revert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("revert: ")?;
        f.write_str(self.reason())
    }
}

impl core::error::Error for Revert {}

impl AsRef<str> for Revert {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.reason
    }
}

impl Borrow<str> for Revert {
    #[inline]
    fn borrow(&self) -> &str {
        &self.reason
    }
}

impl From<Revert> for String {
    #[inline]
    fn from(value: Revert) -> Self {
        value.reason
    }
}

impl From<String> for Revert {
    #[inline]
    fn from(reason: String) -> Self {
        Self { reason }
    }
}

impl From<&str> for Revert {
    #[inline]
    fn from(value: &str) -> Self {
        Self { reason: value.into() }
    }
}

impl SolError for Revert {
    type Parameters<'a> = (crate::sol_data::String,);
    type Token<'a> = (PackedSeqToken<'a>,);

    const SIGNATURE: &'static str = "Error(string)";
    const SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

    #[inline]
    fn new(tuple: <Self::Parameters<'_> as SolType>::RustType) -> Self {
        Self { reason: tuple.0 }
    }

    #[inline]
    fn tokenize(&self) -> Self::Token<'_> {
        (PackedSeqToken::from(self.reason.as_bytes()),)
    }

    #[inline]
    fn abi_encoded_size(&self) -> usize {
        64 + crate::utils::next_multiple_of_32(self.reason.len())
    }
}

impl Revert {
    /// Returns the revert reason string, or `"<empty>"` if empty.
    #[inline]
    pub fn reason(&self) -> &str {
        if self.reason.is_empty() {
            "<empty>"
        } else {
            &self.reason
        }
    }
}

/// A [Solidity panic].
///
/// These are thrown by `assert(condition)` and by internal Solidity checks,
/// such as arithmetic overflow or array bounds checks.
///
/// The list of all known panic codes can be found in the [PanicKind] enum.
///
/// [Solidity panic]: https://docs.soliditylang.org/en/latest/control-structures.html#panic-via-assert-and-error-via-require
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Panic {
    /// The [Solidity panic code].
    ///
    /// [Solidity panic code]: https://docs.soliditylang.org/en/latest/control-structures.html#panic-via-assert-and-error-via-require
    pub code: U256,
}

impl AsRef<U256> for Panic {
    #[inline]
    fn as_ref(&self) -> &U256 {
        &self.code
    }
}

impl Borrow<U256> for Panic {
    #[inline]
    fn borrow(&self) -> &U256 {
        &self.code
    }
}

impl From<PanicKind> for Panic {
    #[inline]
    fn from(value: PanicKind) -> Self {
        Self { code: U256::from(value as u64) }
    }
}

impl From<u64> for Panic {
    #[inline]
    fn from(value: u64) -> Self {
        Self { code: U256::from(value) }
    }
}

impl From<Panic> for U256 {
    #[inline]
    fn from(value: Panic) -> Self {
        value.code
    }
}

impl From<U256> for Panic {
    #[inline]
    fn from(value: U256) -> Self {
        Self { code: value }
    }
}

impl fmt::Debug for Panic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_tuple("Panic");
        if let Some(kind) = self.kind() {
            debug.field(&kind);
        } else {
            debug.field(&self.code);
        }
        debug.finish()
    }
}

impl fmt::Display for Panic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("panic: ")?;

        let kind = self.kind();
        let msg = kind.map(PanicKind::as_str).unwrap_or("unknown code");
        f.write_str(msg)?;

        f.write_str(" (0x")?;
        if let Some(kind) = kind {
            write!(f, "{:02x}", kind as u32)
        } else {
            write!(f, "{:x}", self.code)
        }?;
        f.write_str(")")
    }
}

impl core::error::Error for Panic {}

impl SolError for Panic {
    type Parameters<'a> = (crate::sol_data::Uint<256>,);
    type Token<'a> = (WordToken,);

    const SIGNATURE: &'static str = "Panic(uint256)";
    const SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

    #[inline]
    fn new(tuple: <Self::Parameters<'_> as SolType>::RustType) -> Self {
        Self { code: tuple.0 }
    }

    #[inline]
    fn tokenize(&self) -> Self::Token<'_> {
        (WordToken::from(self.code),)
    }

    #[inline]
    fn abi_encoded_size(&self) -> usize {
        32
    }
}

impl Panic {
    /// Returns the [PanicKind] if this panic code is a known Solidity panic, as
    /// described [in the Solidity documentation][ref].
    ///
    /// [ref]: https://docs.soliditylang.org/en/latest/control-structures.html#panic-via-assert-and-error-via-require
    pub fn kind(&self) -> Option<PanicKind> {
        // use try_from to avoid copying by using the `&` impl
        u32::try_from(&self.code).ok().and_then(PanicKind::from_number)
    }
}

/// Represents a [Solidity panic].
/// Same as the [Solidity definition].
///
/// [Solidity panic]: https://docs.soliditylang.org/en/latest/control-structures.html#panic-via-assert-and-error-via-require
/// [Solidity definition]: https://github.com/ethereum/solidity/blob/9eaa5cebdb1458457135097efdca1a3573af17c8/libsolutil/ErrorCodes.h#L25-L37
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum PanicKind {
    // Docs extracted from the Solidity definition and documentation, linked above.
    /// Generic / unspecified error.
    ///
    /// Generic compiler inserted panics.
    #[default]
    Generic = 0x00,
    /// Used by the `assert()` builtin.
    ///
    /// Thrown when you call `assert` with an argument that evaluates to
    /// `false`.
    Assert = 0x01,
    /// Arithmetic underflow or overflow.
    ///
    /// Thrown when an arithmetic operation results in underflow or overflow
    /// outside of an `unchecked { ... }` block.
    UnderOverflow = 0x11,
    /// Division or modulo by zero.
    ///
    /// Thrown when you divide or modulo by zero (e.g. `5 / 0` or `23 % 0`).
    DivisionByZero = 0x12,
    /// Enum conversion error.
    ///
    /// Thrown when you convert a value that is too big or negative into an enum
    /// type.
    EnumConversionError = 0x21,
    /// Invalid encoding in storage.
    ///
    /// Thrown when you access a storage byte array that is incorrectly encoded.
    StorageEncodingError = 0x22,
    /// Empty array pop.
    ///
    /// Thrown when you call `.pop()` on an empty array.
    EmptyArrayPop = 0x31,
    /// Array out of bounds access.
    ///
    /// Thrown when you access an array, `bytesN` or an array slice at an
    /// out-of-bounds or negative index (i.e. `x[i]` where `i >= x.length` or
    /// `i < 0`).
    ArrayOutOfBounds = 0x32,
    /// Resource error (too large allocation or too large array).
    ///
    /// Thrown when you allocate too much memory or create an array that is too
    /// large.
    ResourceError = 0x41,
    /// Calling invalid internal function.
    ///
    /// Thrown when you call a zero-initialized variable of internal function
    /// type.
    InvalidInternalFunction = 0x51,
}

impl fmt::Display for PanicKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PanicKind {
    /// Returns the panic code for the given number if it is a known one.
    pub const fn from_number(value: u32) -> Option<Self> {
        match value {
            0x00 => Some(Self::Generic),
            0x01 => Some(Self::Assert),
            0x11 => Some(Self::UnderOverflow),
            0x12 => Some(Self::DivisionByZero),
            0x21 => Some(Self::EnumConversionError),
            0x22 => Some(Self::StorageEncodingError),
            0x31 => Some(Self::EmptyArrayPop),
            0x32 => Some(Self::ArrayOutOfBounds),
            0x41 => Some(Self::ResourceError),
            0x51 => Some(Self::InvalidInternalFunction),
            _ => None,
        }
    }

    /// Returns the panic code's string representation.
    pub const fn as_str(self) -> &'static str {
        // modified from the original Solidity comments:
        // https://github.com/ethereum/solidity/blob/9eaa5cebdb1458457135097efdca1a3573af17c8/libsolutil/ErrorCodes.h#L25-L37
        match self {
            Self::Generic => "generic/unspecified error",
            Self::Assert => "assertion failed",
            Self::UnderOverflow => "arithmetic underflow or overflow",
            Self::DivisionByZero => "division or modulo by zero",
            Self::EnumConversionError => "failed to convert value into enum type",
            Self::StorageEncodingError => "storage byte array incorrectly encoded",
            Self::EmptyArrayPop => "called `.pop()` on an empty array",
            Self::ArrayOutOfBounds => "array out-of-bounds access",
            Self::ResourceError => "memory allocation error",
            Self::InvalidInternalFunction => "called an invalid internal function",
        }
    }
}

/// Decodes and retrieves the reason for a revert from the provided output data.
///
/// This function attempts to decode the provided output data as a generic contract error
/// or a UTF-8 string (for Vyper reverts) using the `RevertReason::decode` method.
///
/// If successful, it returns the decoded revert reason wrapped in an `Option`.
///
/// If both attempts fail, it returns `None`.
pub fn decode_revert_reason(out: &[u8]) -> Option<String> {
    RevertReason::decode(out).map(|x| x.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sol, types::interface::SolInterface};
    use alloc::string::ToString;
    use alloy_primitives::{address, hex, keccak256};

    #[test]
    fn revert_encoding() {
        let revert = Revert::from("test");
        let encoded = revert.abi_encode();
        let decoded = Revert::abi_decode(&encoded, true).unwrap();
        assert_eq!(encoded.len(), revert.abi_encoded_size() + 4);
        assert_eq!(encoded.len(), 100);
        assert_eq!(revert, decoded);
    }

    #[test]
    fn panic_encoding() {
        let panic = Panic { code: U256::ZERO };
        assert_eq!(panic.kind(), Some(PanicKind::Generic));
        let encoded = panic.abi_encode();
        let decoded = Panic::abi_decode(&encoded, true).unwrap();

        assert_eq!(encoded.len(), panic.abi_encoded_size() + 4);
        assert_eq!(encoded.len(), 36);
        assert_eq!(panic, decoded);
    }

    #[test]
    fn selectors() {
        assert_eq!(
            Revert::SELECTOR,
            &keccak256(b"Error(string)")[..4],
            "Revert selector is incorrect"
        );
        assert_eq!(
            Panic::SELECTOR,
            &keccak256(b"Panic(uint256)")[..4],
            "Panic selector is incorrect"
        );
    }

    #[test]
    fn decode_solidity_revert_reason() {
        let revert = Revert::from("test_revert_reason");
        let encoded = revert.abi_encode();
        let decoded = decode_revert_reason(&encoded).unwrap();
        assert_eq!(decoded, revert.to_string());
    }

    #[test]
    fn decode_uniswap_revert() {
        // Solc 0.5.X/0.5.16 adds a random 0x80 byte which makes reserialization check fail.
        // https://github.com/Uniswap/v2-core/blob/ee547b17853e71ed4e0101ccfd52e70d5acded58/contracts/UniswapV2Pair.sol#L178
        // https://github.com/paradigmxyz/evm-inspectors/pull/12
        let bytes = hex!("08c379a000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000024556e697377617056323a20494e53554646494349454e545f494e5055545f414d4f554e5400000000000000000000000000000000000000000000000000000080");

        Revert::abi_decode(&bytes, true).unwrap_err();

        let decoded = Revert::abi_decode(&bytes, false).unwrap();
        assert_eq!(decoded.reason, "UniswapV2: INSUFFICIENT_INPUT_AMOUNT");

        let decoded = decode_revert_reason(&bytes).unwrap();
        assert_eq!(decoded, "revert: UniswapV2: INSUFFICIENT_INPUT_AMOUNT");
    }

    #[test]
    fn decode_random_revert_reason() {
        let revert_reason = String::from("test_revert_reason");
        let decoded = decode_revert_reason(revert_reason.as_bytes()).unwrap();
        assert_eq!(decoded, "test_revert_reason");
    }

    #[test]
    fn decode_non_utf8_revert_reason() {
        let revert_reason = [0xFF];
        let decoded = decode_revert_reason(&revert_reason);
        assert_eq!(decoded, None);
    }

    // https://github.com/alloy-rs/core/issues/382
    #[test]
    fn decode_solidity_no_interface() {
        sol! {
            interface C {
                #[derive(Debug, PartialEq)]
                error SenderAddressError(address);
            }
        }

        let data = hex!("8758782b000000000000000000000000a48388222c7ee7daefde5d0b9c99319995c4a990");
        assert_eq!(decode_revert_reason(&data), None);

        let C::CErrors::SenderAddressError(decoded) = C::CErrors::abi_decode(&data, true).unwrap();
        assert_eq!(
            decoded,
            C::SenderAddressError { _0: address!("0xa48388222c7ee7daefde5d0b9c99319995c4a990") }
        );
    }
}
