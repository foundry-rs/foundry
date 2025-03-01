//! [EIP-1898]: https://eips.ethereum.org/EIPS/eip-1898

use alloy_primitives::{hex::FromHexError, ruint::ParseError, BlockHash, B256, U64};
use alloy_rlp::{bytes, Decodable, Encodable, Error as RlpError};
use core::{
    fmt::{self, Formatter},
    num::ParseIntError,
    str::FromStr,
};

/// A helper struct to store the block number/hash and its parent hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub struct BlockWithParent {
    /// Parent hash.
    pub parent: B256,
    /// Block number/hash.
    pub block: BlockNumHash,
}

impl BlockWithParent {
    /// Creates a new [`BlockWithParent`] instance.
    pub const fn new(parent: B256, block: BlockNumHash) -> Self {
        Self { parent, block }
    }
}

/// A block hash which may have a boolean `requireCanonical` field.
///
/// - If false, a RPC call should raise if a block matching the hash is not found.
/// - If true, a RPC call should additionally raise if the block is not in the canonical chain.
///
/// <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1898.md#specification>
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename = "camelCase"))]
pub struct RpcBlockHash {
    /// A block hash
    pub block_hash: BlockHash,
    /// Whether the block must be a canonical block
    pub require_canonical: Option<bool>,
}

impl RpcBlockHash {
    /// Returns a [`RpcBlockHash`] from a [`B256`].
    #[doc(alias = "from_block_hash")]
    pub const fn from_hash(block_hash: B256, require_canonical: Option<bool>) -> Self {
        Self { block_hash, require_canonical }
    }
}

impl From<B256> for RpcBlockHash {
    fn from(value: B256) -> Self {
        Self::from_hash(value, None)
    }
}

impl From<RpcBlockHash> for B256 {
    fn from(value: RpcBlockHash) -> Self {
        value.block_hash
    }
}

impl AsRef<B256> for RpcBlockHash {
    fn as_ref(&self) -> &B256 {
        &self.block_hash
    }
}

impl fmt::Display for RpcBlockHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self { block_hash, require_canonical } = self;
        if *require_canonical == Some(true) {
            f.write_str("canonical ")?;
        }
        write!(f, "hash {block_hash}")
    }
}

impl fmt::Debug for RpcBlockHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.require_canonical {
            Some(require_canonical) => f
                .debug_struct("RpcBlockHash")
                .field("block_hash", &self.block_hash)
                .field("require_canonical", &require_canonical)
                .finish(),
            None => fmt::Debug::fmt(&self.block_hash, f),
        }
    }
}

/// A block Number (or tag - "latest", "earliest", "pending")
///
/// This enum allows users to specify a block in a flexible manner.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BlockNumberOrTag {
    /// Latest block
    #[default]
    Latest,
    /// Finalized block accepted as canonical
    Finalized,
    /// Safe head block
    Safe,
    /// Earliest block (genesis)
    Earliest,
    /// Pending block (not yet part of the blockchain)
    Pending,
    /// Block by number from canonical chain
    Number(u64),
}

impl BlockNumberOrTag {
    /// Returns the numeric block number if explicitly set
    pub const fn as_number(&self) -> Option<u64> {
        match *self {
            Self::Number(num) => Some(num),
            _ => None,
        }
    }

    /// Returns `true` if a numeric block number is set
    pub const fn is_number(&self) -> bool {
        matches!(self, Self::Number(_))
    }

    /// Returns `true` if it's "latest"
    pub const fn is_latest(&self) -> bool {
        matches!(self, Self::Latest)
    }

    /// Returns `true` if it's "finalized"
    pub const fn is_finalized(&self) -> bool {
        matches!(self, Self::Finalized)
    }

    /// Returns `true` if it's "safe"
    pub const fn is_safe(&self) -> bool {
        matches!(self, Self::Safe)
    }

    /// Returns `true` if it's "pending"
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    /// Returns `true` if it's "earliest"
    pub const fn is_earliest(&self) -> bool {
        matches!(self, Self::Earliest)
    }
}

impl From<u64> for BlockNumberOrTag {
    fn from(num: u64) -> Self {
        Self::Number(num)
    }
}

impl From<U64> for BlockNumberOrTag {
    fn from(num: U64) -> Self {
        num.to::<u64>().into()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for BlockNumberOrTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {
            Self::Number(x) => serializer.serialize_str(&format!("0x{x:x}")),
            Self::Latest => serializer.serialize_str("latest"),
            Self::Finalized => serializer.serialize_str("finalized"),
            Self::Safe => serializer.serialize_str("safe"),
            Self::Earliest => serializer.serialize_str("earliest"),
            Self::Pending => serializer.serialize_str("pending"),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for BlockNumberOrTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = alloc::string::String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for BlockNumberOrTag {
    type Err = ParseBlockNumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "latest" => Self::Latest,
            "finalized" => Self::Finalized,
            "safe" => Self::Safe,
            "earliest" => Self::Earliest,
            "pending" => Self::Pending,
            s => {
                if let Some(hex_val) = s.strip_prefix("0x") {
                    u64::from_str_radix(hex_val, 16)?.into()
                } else {
                    return Err(HexStringMissingPrefixError::default().into());
                }
            }
        })
    }
}

impl fmt::Display for BlockNumberOrTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Number(x) => write!(f, "0x{x:x}"),
            Self::Latest => f.pad("latest"),
            Self::Finalized => f.pad("finalized"),
            Self::Safe => f.pad("safe"),
            Self::Earliest => f.pad("earliest"),
            Self::Pending => f.pad("pending"),
        }
    }
}

impl fmt::Debug for BlockNumberOrTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Error thrown when parsing a [BlockNumberOrTag] from a string.
#[derive(Debug)]
pub enum ParseBlockNumberError {
    /// Failed to parse hex value
    ParseIntErr(ParseIntError),
    /// Failed to parse hex value
    ParseErr(ParseError),
    /// Block numbers should be 0x-prefixed
    MissingPrefix(HexStringMissingPrefixError),
}

/// Error variants when parsing a [BlockNumberOrTag]
impl core::error::Error for ParseBlockNumberError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::ParseIntErr(err) => Some(err),
            Self::MissingPrefix(err) => Some(err),
            Self::ParseErr(_) => None,
        }
    }
}

impl fmt::Display for ParseBlockNumberError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseIntErr(err) => write!(f, "{err}"),
            Self::ParseErr(err) => write!(f, "{err}"),
            Self::MissingPrefix(err) => write!(f, "{err}"),
        }
    }
}

impl From<ParseIntError> for ParseBlockNumberError {
    fn from(err: ParseIntError) -> Self {
        Self::ParseIntErr(err)
    }
}

impl From<ParseError> for ParseBlockNumberError {
    fn from(err: ParseError) -> Self {
        Self::ParseErr(err)
    }
}

impl From<HexStringMissingPrefixError> for ParseBlockNumberError {
    fn from(err: HexStringMissingPrefixError) -> Self {
        Self::MissingPrefix(err)
    }
}

/// Thrown when a 0x-prefixed hex string was expected
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct HexStringMissingPrefixError;

impl fmt::Display for HexStringMissingPrefixError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("hex string without 0x prefix")
    }
}

impl core::error::Error for HexStringMissingPrefixError {}

/// A Block Identifier.
/// <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1898.md>
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlockId {
    /// A block hash and an optional bool that defines if it's canonical
    Hash(RpcBlockHash),
    /// A block number
    Number(BlockNumberOrTag),
}

impl BlockId {
    /// Returns the block hash if it is [BlockId::Hash]
    pub const fn as_block_hash(&self) -> Option<BlockHash> {
        match self {
            Self::Hash(hash) => Some(hash.block_hash),
            Self::Number(_) => None,
        }
    }

    /// Returns the block number if it is [`BlockId::Number`] and not a tag
    pub const fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(x) => x.as_number(),
            _ => None,
        }
    }

    /// Returns true if this is [BlockNumberOrTag::Latest]
    pub const fn is_latest(&self) -> bool {
        matches!(self, Self::Number(BlockNumberOrTag::Latest))
    }

    /// Returns true if this is [BlockNumberOrTag::Pending]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Number(BlockNumberOrTag::Pending))
    }

    /// Returns true if this is [BlockNumberOrTag::Safe]
    pub const fn is_safe(&self) -> bool {
        matches!(self, Self::Number(BlockNumberOrTag::Safe))
    }

    /// Returns true if this is [BlockNumberOrTag::Finalized]
    pub const fn is_finalized(&self) -> bool {
        matches!(self, Self::Number(BlockNumberOrTag::Finalized))
    }

    /// Returns true if this is [BlockNumberOrTag::Earliest]
    pub const fn is_earliest(&self) -> bool {
        matches!(self, Self::Number(BlockNumberOrTag::Earliest))
    }

    /// Returns true if this is [BlockNumberOrTag::Number]
    pub const fn is_number(&self) -> bool {
        matches!(self, Self::Number(BlockNumberOrTag::Number(_)))
    }
    /// Returns true if this is [BlockId::Hash]
    pub const fn is_hash(&self) -> bool {
        matches!(self, Self::Hash(_))
    }

    /// Creates a new "pending" tag instance.
    pub const fn pending() -> Self {
        Self::Number(BlockNumberOrTag::Pending)
    }

    /// Creates a new "latest" tag instance.
    pub const fn latest() -> Self {
        Self::Number(BlockNumberOrTag::Latest)
    }

    /// Creates a new "earliest" tag instance.
    pub const fn earliest() -> Self {
        Self::Number(BlockNumberOrTag::Earliest)
    }

    /// Creates a new "finalized" tag instance.
    pub const fn finalized() -> Self {
        Self::Number(BlockNumberOrTag::Finalized)
    }

    /// Creates a new "safe" tag instance.
    pub const fn safe() -> Self {
        Self::Number(BlockNumberOrTag::Safe)
    }

    /// Creates a new block number instance.
    pub const fn number(num: u64) -> Self {
        Self::Number(BlockNumberOrTag::Number(num))
    }

    /// Create a new block hash instance.
    pub const fn hash(block_hash: BlockHash) -> Self {
        Self::Hash(RpcBlockHash { block_hash, require_canonical: None })
    }

    /// Create a new block hash instance that requires the block to be canonical.
    pub const fn hash_canonical(block_hash: BlockHash) -> Self {
        Self::Hash(RpcBlockHash { block_hash, require_canonical: Some(true) })
    }
}

impl Default for BlockId {
    fn default() -> Self {
        BlockNumberOrTag::Latest.into()
    }
}

impl From<u64> for BlockId {
    fn from(num: u64) -> Self {
        BlockNumberOrTag::Number(num).into()
    }
}

impl From<U64> for BlockId {
    fn from(value: U64) -> Self {
        value.to::<u64>().into()
    }
}

impl From<BlockNumberOrTag> for BlockId {
    fn from(num: BlockNumberOrTag) -> Self {
        Self::Number(num)
    }
}

impl From<HashOrNumber> for BlockId {
    fn from(block: HashOrNumber) -> Self {
        match block {
            HashOrNumber::Hash(hash) => hash.into(),
            HashOrNumber::Number(num) => num.into(),
        }
    }
}

impl From<B256> for BlockId {
    fn from(block_hash: B256) -> Self {
        RpcBlockHash { block_hash, require_canonical: None }.into()
    }
}

impl From<(B256, Option<bool>)> for BlockId {
    fn from(hash_can: (B256, Option<bool>)) -> Self {
        RpcBlockHash { block_hash: hash_can.0, require_canonical: hash_can.1 }.into()
    }
}

impl From<RpcBlockHash> for BlockId {
    fn from(value: RpcBlockHash) -> Self {
        Self::Hash(value)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for BlockId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        match self {
            Self::Hash(RpcBlockHash { block_hash, require_canonical }) => {
                let mut s = serializer.serialize_struct("BlockIdEip1898", 1)?;
                s.serialize_field("blockHash", block_hash)?;
                if let Some(require_canonical) = require_canonical {
                    s.serialize_field("requireCanonical", require_canonical)?;
                }
                s.end()
            }
            Self::Number(num) => num.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for BlockId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BlockIdVisitor;

        impl<'de> serde::de::Visitor<'de> for BlockIdVisitor {
            type Value = BlockId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("Block identifier following EIP-1898")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                // Since there is no way to clearly distinguish between a DATA parameter and a QUANTITY parameter. A str is therefor deserialized into a Block Number: <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1898.md>
                // However, since the hex string should be a QUANTITY, we can safely assume that if the len is 66 bytes, it is in fact a hash, ref <https://github.com/ethereum/go-ethereum/blob/ee530c0d5aa70d2c00ab5691a89ab431b73f8165/rpc/types.go#L184-L184>
                if v.len() == 66 {
                    Ok(v.parse::<B256>().map_err(serde::de::Error::custom)?.into())
                } else {
                    // quantity hex string or tag
                    Ok(BlockId::Number(v.parse().map_err(serde::de::Error::custom)?))
                }
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut number = None;
                let mut block_hash = None;
                let mut require_canonical = None;
                while let Some(key) = map.next_key::<alloc::string::String>()? {
                    match key.as_str() {
                        "blockNumber" => {
                            if number.is_some() || block_hash.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockNumber"));
                            }
                            if require_canonical.is_some() {
                                return Err(serde::de::Error::custom(
                                    "Non-valid require_canonical field",
                                ));
                            }
                            number = Some(map.next_value::<BlockNumberOrTag>()?)
                        }
                        "blockHash" => {
                            if number.is_some() || block_hash.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockHash"));
                            }

                            block_hash = Some(map.next_value::<B256>()?);
                        }
                        "requireCanonical" => {
                            if number.is_some() || require_canonical.is_some() {
                                return Err(serde::de::Error::duplicate_field("requireCanonical"));
                            }

                            require_canonical = Some(map.next_value::<bool>()?)
                        }
                        key => {
                            return Err(serde::de::Error::unknown_field(
                                key,
                                &["blockNumber", "blockHash", "requireCanonical"],
                            ))
                        }
                    }
                }

                #[allow(clippy::option_if_let_else)]
                if let Some(number) = number {
                    Ok(number.into())
                } else if let Some(block_hash) = block_hash {
                    Ok((block_hash, require_canonical).into())
                } else {
                    Err(serde::de::Error::custom(
                        "Expected `blockNumber` or `blockHash` with `requireCanonical` optionally",
                    ))
                }
            }
        }

        deserializer.deserialize_any(BlockIdVisitor)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(hash) => hash.fmt(f),
            Self::Number(num) => num.fmt(f),
        }
    }
}

impl fmt::Debug for BlockId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(hash) => hash.fmt(f),
            Self::Number(num) => num.fmt(f),
        }
    }
}

/// Error thrown when parsing a [BlockId] from a string.
#[derive(Debug)]
pub enum ParseBlockIdError {
    /// Failed to parse a block id from a number.
    ParseIntError(ParseIntError),
    /// Failed to parse hex number
    ParseError(ParseError),
    /// Failed to parse a block id as a hex string.
    FromHexError(FromHexError),
}

impl fmt::Display for ParseBlockIdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseIntError(err) => write!(f, "{err}"),
            Self::ParseError(err) => write!(f, "{err}"),
            Self::FromHexError(err) => write!(f, "{err}"),
        }
    }
}

impl core::error::Error for ParseBlockIdError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::ParseIntError(err) => Some(err),
            Self::FromHexError(err) => Some(err),
            Self::ParseError(_) => None,
        }
    }
}

impl From<ParseIntError> for ParseBlockIdError {
    fn from(err: ParseIntError) -> Self {
        Self::ParseIntError(err)
    }
}

impl From<FromHexError> for ParseBlockIdError {
    fn from(err: FromHexError) -> Self {
        Self::FromHexError(err)
    }
}

impl FromStr for BlockId {
    type Err = ParseBlockIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("0x") {
            return match s.len() {
                66 => B256::from_str(s).map(Into::into).map_err(ParseBlockIdError::FromHexError),
                _ => U64::from_str(s).map(Into::into).map_err(ParseBlockIdError::ParseError),
            };
        }

        match s {
            "latest" | "finalized" | "safe" | "earliest" | "pending" => {
                Ok(BlockNumberOrTag::from_str(s).unwrap().into())
            }
            _ => s.parse::<u64>().map_err(ParseBlockIdError::ParseIntError).map(Into::into),
        }
    }
}

/// A number and a hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub struct NumHash {
    /// The number
    pub number: u64,
    /// The hash.
    pub hash: B256,
}

/// Block number and hash of the forked block.
pub type ForkBlock = NumHash;

/// A block number and a hash
pub type BlockNumHash = NumHash;

impl NumHash {
    /// Creates a new `NumHash` from a number and hash.
    pub const fn new(number: u64, hash: B256) -> Self {
        Self { number, hash }
    }

    /// Consumes `Self` and returns the number and hash
    pub const fn into_components(self) -> (u64, B256) {
        (self.number, self.hash)
    }

    /// Returns whether or not the block matches the given [HashOrNumber].
    pub fn matches_block_or_num(&self, block: &HashOrNumber) -> bool {
        match block {
            HashOrNumber::Hash(hash) => self.hash == *hash,
            HashOrNumber::Number(number) => self.number == *number,
        }
    }
}

impl From<(u64, B256)> for NumHash {
    fn from(val: (u64, B256)) -> Self {
        Self { number: val.0, hash: val.1 }
    }
}

impl From<(B256, u64)> for NumHash {
    fn from(val: (B256, u64)) -> Self {
        Self { hash: val.0, number: val.1 }
    }
}

/// Either a hash _or_ a block number
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub enum HashOrNumber {
    /// The hash
    Hash(B256),
    /// The number
    Number(u64),
}

/// A block hash _or_ a block number
pub type BlockHashOrNumber = HashOrNumber;

// === impl HashOrNumber ===

impl HashOrNumber {
    /// Returns the block number if it is a [`HashOrNumber::Number`].
    #[inline]
    pub const fn as_number(self) -> Option<u64> {
        match self {
            Self::Hash(_) => None,
            Self::Number(num) => Some(num),
        }
    }

    /// Returns the block hash if it is a [`HashOrNumber::Hash`].
    #[inline]
    pub const fn as_hash(self) -> Option<B256> {
        match self {
            Self::Hash(hash) => Some(hash),
            Self::Number(_) => None,
        }
    }
}

impl From<B256> for HashOrNumber {
    fn from(value: B256) -> Self {
        Self::Hash(value)
    }
}

impl From<&B256> for HashOrNumber {
    fn from(value: &B256) -> Self {
        (*value).into()
    }
}

impl From<u64> for HashOrNumber {
    fn from(value: u64) -> Self {
        Self::Number(value)
    }
}

impl From<U64> for HashOrNumber {
    fn from(value: U64) -> Self {
        value.to::<u64>().into()
    }
}

impl From<RpcBlockHash> for HashOrNumber {
    fn from(value: RpcBlockHash) -> Self {
        Self::Hash(value.into())
    }
}

/// Allows for RLP encoding of either a hash or a number
impl Encodable for HashOrNumber {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        match self {
            Self::Hash(block_hash) => block_hash.encode(out),
            Self::Number(block_number) => block_number.encode(out),
        }
    }
    fn length(&self) -> usize {
        match self {
            Self::Hash(block_hash) => block_hash.length(),
            Self::Number(block_number) => block_number.length(),
        }
    }
}

/// Allows for RLP decoding of a hash or number
impl Decodable for HashOrNumber {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let header: u8 = *buf.first().ok_or(RlpError::InputTooShort)?;
        // if the byte string is exactly 32 bytes, decode it into a Hash
        // 0xa0 = 0x80 (start of string) + 0x20 (32, length of string)
        if header == 0xa0 {
            // strip the first byte, parsing the rest of the string.
            // If the rest of the string fails to decode into 32 bytes, we'll bubble up the
            // decoding error.
            Ok(B256::decode(buf)?.into())
        } else {
            // a block number when encoded as bytes ranges from 0 to any number of bytes - we're
            // going to accept numbers which fit in less than 64 bytes.
            // Any data larger than this which is not caught by the Hash decoding should error and
            // is considered an invalid block number.
            Ok(u64::decode(buf)?.into())
        }
    }
}

impl fmt::Display for HashOrNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(hash) => write!(f, "{}", hash),
            Self::Number(num) => write!(f, "{}", num),
        }
    }
}

/// Error thrown when parsing a [HashOrNumber] from a string.
#[derive(Debug)]
pub struct ParseBlockHashOrNumberError {
    input: alloc::string::String,
    parse_int_error: ParseIntError,
    hex_error: alloy_primitives::hex::FromHexError,
}

impl fmt::Display for ParseBlockHashOrNumberError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to parse {:?} as a number: {} or hash: {}",
            self.input, self.parse_int_error, self.hex_error
        )
    }
}

impl core::error::Error for ParseBlockHashOrNumberError {}

impl FromStr for HashOrNumber {
    type Err = ParseBlockHashOrNumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use alloc::string::ToString;

        match u64::from_str(s) {
            Ok(val) => Ok(val.into()),
            Err(parse_int_error) => match B256::from_str(s) {
                Ok(val) => Ok(val.into()),
                Err(hex_error) => Err(ParseBlockHashOrNumberError {
                    input: s.to_string(),
                    parse_int_error,
                    hex_error,
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, vec::Vec};
    use alloy_primitives::b256;

    const HASH: B256 = b256!("1a15e3c30cf094a99826869517b16d185d45831d3a494f01030b0001a9d3ebb9");

    #[test]
    fn block_id_from_str() {
        assert_eq!("0x0".parse::<BlockId>().unwrap(), BlockId::number(0));
        assert_eq!("0x24A931".parse::<BlockId>().unwrap(), BlockId::number(2402609));
        assert_eq!(
            "0x1a15e3c30cf094a99826869517b16d185d45831d3a494f01030b0001a9d3ebb9"
                .parse::<BlockId>()
                .unwrap(),
            HASH.into()
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn compact_block_number_serde() {
        let num: BlockNumberOrTag = 1u64.into();
        let serialized = serde_json::to_string(&num).unwrap();
        assert_eq!(serialized, "\"0x1\"");
    }

    #[test]
    fn block_id_as_u64() {
        assert_eq!(BlockId::number(123).as_u64(), Some(123));
        assert_eq!(BlockId::number(0).as_u64(), Some(0));
        assert_eq!(BlockId::earliest().as_u64(), None);
        assert_eq!(BlockId::latest().as_u64(), None);
        assert_eq!(BlockId::pending().as_u64(), None);
        assert_eq!(BlockId::safe().as_u64(), None);
        assert_eq!(BlockId::hash(BlockHash::ZERO).as_u64(), None);
        assert_eq!(BlockId::hash_canonical(BlockHash::ZERO).as_u64(), None);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn can_parse_eip1898_block_ids() {
        let num = serde_json::json!(
            { "blockNumber": "0x0" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Number(0u64)));

        let num = serde_json::json!(
            { "blockNumber": "pending" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Pending));

        let num = serde_json::json!(
            { "blockNumber": "latest" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Latest));

        let num = serde_json::json!(
            { "blockNumber": "finalized" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Finalized));

        let num = serde_json::json!(
            { "blockNumber": "safe" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Safe));

        let num = serde_json::json!(
            { "blockNumber": "earliest" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Earliest));

        let num = serde_json::json!("0x0");
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Number(0u64)));

        let num = serde_json::json!("pending");
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Pending));

        let num = serde_json::json!("latest");
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Latest));

        let num = serde_json::json!("finalized");
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Finalized));

        let num = serde_json::json!("safe");
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Safe));

        let num = serde_json::json!("earliest");
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(id, BlockId::Number(BlockNumberOrTag::Earliest));

        let num = serde_json::json!(
            { "blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3" }
        );
        let id = serde_json::from_value::<BlockId>(num).unwrap();
        assert_eq!(
            id,
            BlockId::Hash(
                "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3"
                    .parse::<B256>()
                    .unwrap()
                    .into()
            )
        );
    }

    #[test]
    fn display_rpc_block_hash() {
        let hash = RpcBlockHash::from_hash(HASH, Some(true));

        assert_eq!(
            hash.to_string(),
            "canonical hash 0x1a15e3c30cf094a99826869517b16d185d45831d3a494f01030b0001a9d3ebb9"
        );

        let hash = RpcBlockHash::from_hash(HASH, None);

        assert_eq!(
            hash.to_string(),
            "hash 0x1a15e3c30cf094a99826869517b16d185d45831d3a494f01030b0001a9d3ebb9"
        );
    }

    #[test]
    fn display_block_id() {
        let id = BlockId::hash(HASH);

        assert_eq!(
            id.to_string(),
            "hash 0x1a15e3c30cf094a99826869517b16d185d45831d3a494f01030b0001a9d3ebb9"
        );

        let id = BlockId::hash_canonical(HASH);

        assert_eq!(
            id.to_string(),
            "canonical hash 0x1a15e3c30cf094a99826869517b16d185d45831d3a494f01030b0001a9d3ebb9"
        );

        let id = BlockId::number(100000);

        assert_eq!(id.to_string(), "0x186a0");

        let id = BlockId::latest();

        assert_eq!(id.to_string(), "latest");

        let id = BlockId::safe();

        assert_eq!(id.to_string(), "safe");

        let id = BlockId::finalized();

        assert_eq!(id.to_string(), "finalized");

        let id = BlockId::earliest();

        assert_eq!(id.to_string(), "earliest");

        let id = BlockId::pending();

        assert_eq!(id.to_string(), "pending");
    }

    #[test]
    fn test_block_number_or_tag() {
        // Test Latest variant
        let latest = BlockNumberOrTag::Latest;
        assert_eq!(latest.as_number(), None);
        assert!(latest.is_latest());
        assert!(!latest.is_number());
        assert!(!latest.is_finalized());
        assert!(!latest.is_safe());
        assert!(!latest.is_pending());
        assert!(!latest.is_earliest());

        // Test Finalized variant
        let finalized = BlockNumberOrTag::Finalized;
        assert_eq!(finalized.as_number(), None);
        assert!(finalized.is_finalized());
        assert!(!finalized.is_latest());
        assert!(!finalized.is_number());
        assert!(!finalized.is_safe());
        assert!(!finalized.is_pending());
        assert!(!finalized.is_earliest());

        // Test Safe variant
        let safe = BlockNumberOrTag::Safe;
        assert_eq!(safe.as_number(), None);
        assert!(safe.is_safe());
        assert!(!safe.is_latest());
        assert!(!safe.is_number());
        assert!(!safe.is_finalized());
        assert!(!safe.is_pending());
        assert!(!safe.is_earliest());

        // Test Earliest variant
        let earliest = BlockNumberOrTag::Earliest;
        assert_eq!(earliest.as_number(), None);
        assert!(earliest.is_earliest());
        assert!(!earliest.is_latest());
        assert!(!earliest.is_number());
        assert!(!earliest.is_finalized());
        assert!(!earliest.is_safe());
        assert!(!earliest.is_pending());

        // Test Pending variant
        let pending = BlockNumberOrTag::Pending;
        assert_eq!(pending.as_number(), None);
        assert!(pending.is_pending());
        assert!(!pending.is_latest());
        assert!(!pending.is_number());
        assert!(!pending.is_finalized());
        assert!(!pending.is_safe());
        assert!(!pending.is_earliest());

        // Test Number variant
        let number = BlockNumberOrTag::Number(42);
        assert_eq!(number.as_number(), Some(42));
        assert!(number.is_number());
        assert!(!number.is_latest());
        assert!(!number.is_finalized());
        assert!(!number.is_safe());
        assert!(!number.is_pending());
        assert!(!number.is_earliest());
    }

    #[test]
    fn test_block_number_or_tag_from() {
        // Test conversion from u64
        let num = 100u64;
        let block: BlockNumberOrTag = num.into();
        assert_eq!(block, BlockNumberOrTag::Number(100));

        // Test conversion from U64
        let num = U64::from(200);
        let block: BlockNumberOrTag = num.into();
        assert_eq!(block, BlockNumberOrTag::Number(200));
    }

    #[test]
    fn test_block_id() {
        let hash = BlockHash::random();

        // Block hash
        let block_id_hash = BlockId::hash(hash);
        assert_eq!(block_id_hash.as_block_hash(), Some(hash));
        assert!(block_id_hash.is_hash());
        assert!(!block_id_hash.is_number());
        assert!(!block_id_hash.is_latest());
        assert!(!block_id_hash.is_pending());
        assert!(!block_id_hash.is_safe());
        assert!(!block_id_hash.is_finalized());
        assert!(!block_id_hash.is_earliest());

        // Block number
        let block_id_number = BlockId::number(123);
        assert_eq!(block_id_number.as_u64(), Some(123));
        assert!(block_id_number.is_number());
        assert!(!block_id_number.is_hash());
        assert!(!block_id_number.is_latest());
        assert!(!block_id_number.is_pending());
        assert!(!block_id_number.is_safe());
        assert!(!block_id_number.is_finalized());
        assert!(!block_id_number.is_earliest());

        // Latest block
        let block_latest = BlockId::latest();
        assert!(block_latest.is_latest());
        assert!(!block_latest.is_number());
        assert!(!block_latest.is_hash());
        assert!(!block_latest.is_pending());
        assert!(!block_latest.is_safe());
        assert!(!block_latest.is_finalized());
        assert!(!block_latest.is_earliest());

        // Pending block
        let block_pending = BlockId::pending();
        assert!(block_pending.is_pending());
        assert!(!block_pending.is_latest());
        assert!(!block_pending.is_number());
        assert!(!block_pending.is_hash());
        assert!(!block_pending.is_safe());
        assert!(!block_pending.is_finalized());
        assert!(!block_pending.is_earliest());

        // Safe block
        let block_safe = BlockId::safe();
        assert!(block_safe.is_safe());
        assert!(!block_safe.is_latest());
        assert!(!block_safe.is_number());
        assert!(!block_safe.is_hash());
        assert!(!block_safe.is_pending());
        assert!(!block_safe.is_finalized());
        assert!(!block_safe.is_earliest());

        // Finalized block
        let block_finalized = BlockId::finalized();
        assert!(block_finalized.is_finalized());
        assert!(!block_finalized.is_latest());
        assert!(!block_finalized.is_number());
        assert!(!block_finalized.is_hash());
        assert!(!block_finalized.is_pending());
        assert!(!block_finalized.is_safe());
        assert!(!block_finalized.is_earliest());

        // Earliest block
        let block_earliest = BlockId::earliest();
        assert!(block_earliest.is_earliest());
        assert!(!block_earliest.is_latest());
        assert!(!block_earliest.is_number());
        assert!(!block_earliest.is_hash());
        assert!(!block_earliest.is_pending());
        assert!(!block_earliest.is_safe());
        assert!(!block_earliest.is_finalized());

        // Default block
        assert!(BlockId::default().is_latest());
        assert!(!BlockId::default().is_number());
        assert!(!BlockId::default().is_hash());
        assert!(!BlockId::default().is_pending());
        assert!(!BlockId::default().is_safe());
        assert!(!BlockId::default().is_finalized());
        assert!(!BlockId::default().is_earliest());
    }

    #[test]
    fn test_u64_to_block_id() {
        // Simple u64
        let num: u64 = 123;
        let block_id: BlockId = num.into();

        match block_id {
            BlockId::Number(BlockNumberOrTag::Number(n)) => assert_eq!(n, 123),
            _ => panic!("Expected BlockId::Number with 123"),
        }

        // Big integer U64
        let num: U64 = U64::from(456);
        let block_id: BlockId = num.into();

        match block_id {
            BlockId::Number(BlockNumberOrTag::Number(n)) => assert_eq!(n, 456),
            _ => panic!("Expected BlockId::Number with 456"),
        }

        // u64 as HashOrNumber
        let num: u64 = 789;
        let block_id: BlockId = HashOrNumber::Number(num).into();

        match block_id {
            BlockId::Number(BlockNumberOrTag::Number(n)) => assert_eq!(n, 789),
            _ => panic!("Expected BlockId::Number with 789"),
        }
    }

    #[test]
    fn test_block_number_or_tag_to_block_id() {
        let block_number_or_tag = BlockNumberOrTag::Pending;
        let block_id: BlockId = block_number_or_tag.into();

        match block_id {
            BlockId::Number(BlockNumberOrTag::Pending) => {}
            _ => panic!("Expected BlockId::Number with Pending"),
        }
    }

    #[test]
    fn test_hash_or_number_to_block_id_hash() {
        // B256 wrapped in HashOrNumber
        let hash: B256 = B256::random();
        let block_id: BlockId = HashOrNumber::Hash(hash).into();

        match block_id {
            BlockId::Hash(rpc_block_hash) => assert_eq!(rpc_block_hash.block_hash, hash),
            _ => panic!("Expected BlockId::Hash"),
        }

        // Simple B256
        let hash: B256 = B256::random();
        let block_id: BlockId = hash.into();

        match block_id {
            BlockId::Hash(rpc_block_hash) => assert_eq!(rpc_block_hash.block_hash, hash),
            _ => panic!("Expected BlockId::Hash"),
        }

        // Tuple with B256 and canonical flag
        let hash: B256 = B256::random();
        let block_id: BlockId = (hash, Some(true)).into();

        match block_id {
            BlockId::Hash(rpc_block_hash) => {
                assert_eq!(rpc_block_hash.block_hash, hash);
                assert_eq!(rpc_block_hash.require_canonical, Some(true));
            }
            _ => panic!("Expected BlockId::Hash with canonical flag"),
        }
    }

    #[test]
    fn test_hash_or_number_as_number() {
        // Test with a number
        let hash_or_number = HashOrNumber::Number(123);
        assert_eq!(hash_or_number.as_number(), Some(123));

        // Test with a hash
        let hash = B256::random();
        let hash_or_number = HashOrNumber::Hash(hash);
        assert_eq!(hash_or_number.as_number(), None);
    }

    #[test]
    fn test_hash_or_number_as_hash() {
        // Test with a hash
        let hash = B256::random();
        let hash_or_number = HashOrNumber::Hash(hash);
        assert_eq!(hash_or_number.as_hash(), Some(hash));

        // Test with a number
        let hash_or_number = HashOrNumber::Number(456);
        assert_eq!(hash_or_number.as_hash(), None);
    }

    #[test]
    fn test_hash_or_number_conversions() {
        // Test conversion from B256
        let hash = B256::random();
        let hash_or_number: HashOrNumber = hash.into();
        assert_eq!(hash_or_number, HashOrNumber::Hash(hash));

        // Test conversion from &B256
        let hash_ref: HashOrNumber = (&hash).into();
        assert_eq!(hash_ref, HashOrNumber::Hash(hash));

        // Test conversion from u64
        let number: u64 = 123;
        let hash_or_number: HashOrNumber = number.into();
        assert_eq!(hash_or_number, HashOrNumber::Number(number));

        // Test conversion from U64
        let u64_value = U64::from(456);
        let hash_or_number: HashOrNumber = u64_value.into();
        assert_eq!(hash_or_number, HashOrNumber::Number(u64_value.to::<u64>()));

        // Test conversion from RpcBlockHash (assuming RpcBlockHash is convertible to B256)
        let rpc_block_hash = RpcBlockHash { block_hash: hash, require_canonical: Some(true) };
        let hash_or_number: HashOrNumber = rpc_block_hash.into();
        assert_eq!(hash_or_number, HashOrNumber::Hash(hash));
    }

    #[test]
    fn test_hash_or_number_rlp_roundtrip_hash() {
        // Test case: encoding and decoding a B256 hash
        let original_hash = B256::random();
        let hash_or_number: HashOrNumber = HashOrNumber::Hash(original_hash);

        // Encode the HashOrNumber
        let mut buf = Vec::new();
        hash_or_number.encode(&mut buf);

        // Decode the encoded bytes
        let decoded: HashOrNumber = HashOrNumber::decode(&mut &buf[..]).expect("Decoding failed");

        // Assert that the decoded value matches the original
        assert_eq!(decoded, hash_or_number);
    }

    #[test]
    fn test_hash_or_number_rlp_roundtrip_u64() {
        // Test case: encoding and decoding a u64 number
        let original_number: u64 = 12345;
        let hash_or_number: HashOrNumber = HashOrNumber::Number(original_number);

        // Encode the HashOrNumber
        let mut buf = Vec::new();
        hash_or_number.encode(&mut buf);

        // Decode the encoded bytes
        let decoded: HashOrNumber = HashOrNumber::decode(&mut &buf[..]).expect("Decoding failed");

        // Assert that the decoded value matches the original
        assert_eq!(decoded, hash_or_number);
    }

    #[test]
    fn test_numhash() {
        let number: u64 = 42;
        let hash = B256::random();

        let num_hash = NumHash::new(number, hash);

        // Validate the initial values
        assert_eq!(num_hash.number, number);
        assert_eq!(num_hash.hash, hash);

        // Test into_components
        assert_eq!(num_hash.into_components(), (number, hash));
    }

    #[test]
    fn test_numhash_matches_block_or_num() {
        let number: u64 = 42;
        let hash = B256::random();

        let num_hash = NumHash::new(number, hash);

        // Test matching by hash
        let block_hash = HashOrNumber::Hash(hash);
        assert!(num_hash.matches_block_or_num(&block_hash));

        // Test matching by number
        let block_number = HashOrNumber::Number(number);
        assert!(num_hash.matches_block_or_num(&block_number));

        // Test non-matching by different hash
        let different_hash = B256::random();
        let non_matching_hash = HashOrNumber::Hash(different_hash);
        assert!(!num_hash.matches_block_or_num(&non_matching_hash));

        // Test non-matching by different number
        let different_number: u64 = 43;
        let non_matching_number = HashOrNumber::Number(different_number);
        assert!(!num_hash.matches_block_or_num(&non_matching_number));
    }

    #[test]
    fn test_numhash_conversions() {
        // From a tuple (u64, B256)
        let number: u64 = 42;
        let hash = B256::random();

        let num_hash_from_tuple: NumHash = (number, hash).into();

        assert_eq!(num_hash_from_tuple.number, number);
        assert_eq!(num_hash_from_tuple.hash, hash);

        // From a reversed tuple (B256, u64)
        let number: u64 = 42;
        let hash = B256::random();

        let num_hash_from_reversed_tuple: NumHash = (hash, number).into();

        assert_eq!(num_hash_from_reversed_tuple.number, number);
        assert_eq!(num_hash_from_reversed_tuple.hash, hash);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_block_id_from_str() {
        // Valid hexadecimal block ID (with 0x prefix)
        let hex_id = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert_eq!(
            BlockId::from_str(hex_id).unwrap(),
            BlockId::Hash(RpcBlockHash::from_hash(B256::from_str(hex_id).unwrap(), None))
        );

        // Valid tag strings
        assert_eq!(BlockId::from_str("latest").unwrap(), BlockNumberOrTag::Latest.into());
        assert_eq!(BlockId::from_str("finalized").unwrap(), BlockNumberOrTag::Finalized.into());
        assert_eq!(BlockId::from_str("safe").unwrap(), BlockNumberOrTag::Safe.into());
        assert_eq!(BlockId::from_str("earliest").unwrap(), BlockNumberOrTag::Earliest.into());
        assert_eq!(BlockId::from_str("pending").unwrap(), BlockNumberOrTag::Pending.into());

        // Valid numeric string without prefix
        let numeric_string = "12345";
        let parsed_numeric_string = BlockId::from_str(numeric_string);
        assert!(parsed_numeric_string.is_ok());

        // Hex interpretation of numeric string
        assert_eq!(
            BlockId::from_str("0x12345").unwrap(),
            BlockId::Number(BlockNumberOrTag::Number(74565))
        );

        // Invalid non-numeric string
        let invalid_string = "invalid_block_id";
        let parsed_invalid_string = BlockId::from_str(invalid_string);
        assert!(parsed_invalid_string.is_err());
    }

    /// Check parsing according to EIP-1898.
    #[test]
    #[cfg(feature = "serde")]
    fn can_parse_blockid_u64() {
        let num = serde_json::json!(
            {"blockNumber": "0xaf"}
        );

        let id = serde_json::from_value::<BlockId>(num);
        assert_eq!(id.unwrap(), BlockId::from(175));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn can_parse_block_hash() {
        let block_hash =
            B256::from_str("0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
                .unwrap();
        let block_hash_json = serde_json::json!(
            { "blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3"}
        );
        let id = serde_json::from_value::<BlockId>(block_hash_json).unwrap();
        assert_eq!(id, BlockId::from(block_hash,));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn can_parse_block_hash_with_canonical() {
        let block_hash =
            B256::from_str("0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
                .unwrap();
        let block_id = BlockId::Hash(RpcBlockHash::from_hash(block_hash, Some(true)));
        let block_hash_json = serde_json::json!(
            { "blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3", "requireCanonical": true }
        );
        let id = serde_json::from_value::<BlockId>(block_hash_json).unwrap();
        assert_eq!(id, block_id)
    }
    #[test]
    #[cfg(feature = "serde")]
    fn can_parse_blockid_tags() {
        let tags = [
            ("latest", BlockNumberOrTag::Latest),
            ("finalized", BlockNumberOrTag::Finalized),
            ("safe", BlockNumberOrTag::Safe),
            ("pending", BlockNumberOrTag::Pending),
        ];
        for (value, tag) in tags {
            let num = serde_json::json!({ "blockNumber": value });
            let id = serde_json::from_value::<BlockId>(num);
            assert_eq!(id.unwrap(), BlockId::from(tag))
        }
    }
    #[test]
    #[cfg(feature = "serde")]
    fn repeated_keys_is_err() {
        let num = serde_json::json!({"blockNumber": 1, "requireCanonical": true, "requireCanonical": false});
        assert!(serde_json::from_value::<BlockId>(num).is_err());
        let num =
            serde_json::json!({"blockNumber": 1, "requireCanonical": true, "blockNumber": 23});
        assert!(serde_json::from_value::<BlockId>(num).is_err());
    }

    /// Serde tests
    #[test]
    #[cfg(feature = "serde")]
    fn serde_blockid_tags() {
        let block_ids = [
            BlockNumberOrTag::Latest,
            BlockNumberOrTag::Finalized,
            BlockNumberOrTag::Safe,
            BlockNumberOrTag::Pending,
        ]
        .map(BlockId::from);
        for block_id in &block_ids {
            let serialized = serde_json::to_string(&block_id).unwrap();
            let deserialized: BlockId = serde_json::from_str(&serialized).unwrap();
            assert_eq!(deserialized, *block_id)
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_blockid_number() {
        let block_id = BlockId::from(100u64);
        let serialized = serde_json::to_string(&block_id).unwrap();
        let deserialized: BlockId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, block_id)
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_blockid_hash() {
        let block_id = BlockId::from(B256::default());
        let serialized = serde_json::to_string(&block_id).unwrap();
        let deserialized: BlockId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, block_id)
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_blockid_hash_from_str() {
        let val = "\"0x898753d8fdd8d92c1907ca21e68c7970abd290c647a202091181deec3f30a0b2\"";
        let block_hash: B256 = serde_json::from_str(val).unwrap();
        let block_id: BlockId = serde_json::from_str(val).unwrap();
        assert_eq!(block_id, BlockId::Hash(block_hash.into()));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_rpc_payload_block_tag() {
        let payload = r#"{"method":"eth_call","params":[{"to":"0xebe8efa441b9302a0d7eaecc277c09d20d684540","data":"0x45848dfc"},"latest"],"id":1,"jsonrpc":"2.0"}"#;
        let value: serde_json::Value = serde_json::from_str(payload).unwrap();
        let block_id_param = value.pointer("/params/1").unwrap();
        let block_id: BlockId = serde_json::from_value::<BlockId>(block_id_param.clone()).unwrap();
        assert_eq!(BlockId::Number(BlockNumberOrTag::Latest), block_id);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_rpc_payload_block_object() {
        let example_payload = r#"{"method":"eth_call","params":[{"to":"0xebe8efa441b9302a0d7eaecc277c09d20d684540","data":"0x45848dfc"},{"blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3"}],"id":1,"jsonrpc":"2.0"}"#;
        let value: serde_json::Value = serde_json::from_str(example_payload).unwrap();
        let block_id_param = value.pointer("/params/1").unwrap().to_string();
        let block_id: BlockId = serde_json::from_str::<BlockId>(&block_id_param).unwrap();
        let hash =
            B256::from_str("0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
                .unwrap();
        assert_eq!(BlockId::from(hash), block_id);
        let serialized = serde_json::to_string(&BlockId::from(hash)).unwrap();
        assert_eq!("{\"blockHash\":\"0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3\"}", serialized)
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_rpc_payload_block_number() {
        let example_payload = r#"{"method":"eth_call","params":[{"to":"0xebe8efa441b9302a0d7eaecc277c09d20d684540","data":"0x45848dfc"},{"blockNumber": "0x0"}],"id":1,"jsonrpc":"2.0"}"#;
        let value: serde_json::Value = serde_json::from_str(example_payload).unwrap();
        let block_id_param = value.pointer("/params/1").unwrap().to_string();
        let block_id: BlockId = serde_json::from_str::<BlockId>(&block_id_param).unwrap();
        assert_eq!(BlockId::from(0u64), block_id);
        let serialized = serde_json::to_string(&BlockId::from(0u64)).unwrap();
        assert_eq!("\"0x0\"", serialized)
    }

    #[test]
    #[should_panic]
    #[cfg(feature = "serde")]
    fn serde_rpc_payload_block_number_duplicate_key() {
        let payload = r#"{"blockNumber": "0x132", "blockNumber": "0x133"}"#;
        let parsed_block_id = serde_json::from_str::<BlockId>(payload);
        parsed_block_id.unwrap();
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_rpc_payload_block_hash() {
        let payload = r#"{"blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3"}"#;
        let parsed = serde_json::from_str::<BlockId>(payload).unwrap();
        let expected = BlockId::from(
            B256::from_str("0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
                .unwrap(),
        );
        assert_eq!(parsed, expected);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_blocknumber_non_0xprefix() {
        let s = "\"2\"";
        let err = serde_json::from_str::<BlockNumberOrTag>(s).unwrap_err();
        assert_eq!(err.to_string(), HexStringMissingPrefixError::default().to_string());
    }
}
