use crate::{BlockNumberOrTag, Log as RpcLog, Transaction};
use alloc::{string::String, vec::Vec};
use alloy_primitives::{
    keccak256,
    map::{hash_set, HashSet},
    Address, BlockHash, Bloom, BloomInput, B256, U256, U64,
};
use core::{
    hash::Hash,
    ops::{RangeFrom, RangeInclusive, RangeToInclusive},
};
use itertools::{
    EitherOrBoth::{Both, Left, Right},
    Itertools,
};

/// Helper type to represent a bloom filter used for matching logs.
#[derive(Debug, Default)]
pub struct BloomFilter(Vec<Bloom>);

impl From<Vec<Bloom>> for BloomFilter {
    fn from(src: Vec<Bloom>) -> Self {
        Self(src)
    }
}

impl BloomFilter {
    /// Returns whether the given bloom matches the list of Blooms in the current filter.
    /// If the filter is empty (the list is empty), then any bloom matches
    /// Otherwise, there must be at least one match for the BloomFilter to match.
    pub fn matches(&self, bloom: Bloom) -> bool {
        self.0.is_empty() || self.0.iter().any(|a| bloom.contains(a))
    }
}

/// FilterSet is a set of values that will be used to filter logs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct FilterSet<T: Eq + Hash>(HashSet<T>);

impl<T: Eq + Hash> From<T> for FilterSet<T> {
    fn from(src: T) -> Self {
        Self(core::iter::once(src).collect())
    }
}

impl<T: Eq + Hash> Hash for FilterSet<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        for value in &self.0 {
            value.hash(state);
        }
    }
}

impl<T: Eq + Hash> From<Vec<T>> for FilterSet<T> {
    fn from(src: Vec<T>) -> Self {
        Self(src.into_iter().collect())
    }
}

impl<T: Eq + Hash> From<ValueOrArray<T>> for FilterSet<T> {
    fn from(src: ValueOrArray<T>) -> Self {
        match src {
            ValueOrArray::Value(val) => val.into(),
            ValueOrArray::Array(arr) => arr.into(),
        }
    }
}

impl<T: Eq + Hash> From<ValueOrArray<Option<T>>> for FilterSet<T> {
    fn from(src: ValueOrArray<Option<T>>) -> Self {
        match src {
            ValueOrArray::Value(None) => Self(Default::default()),
            ValueOrArray::Value(Some(val)) => val.into(),
            ValueOrArray::Array(arr) => {
                // If the array contains at least one `null` (ie. None), as it's considered
                // a "wildcard" value, the whole filter should be treated as matching everything,
                // thus is empty.
                if arr.iter().contains(&None) {
                    Self(Default::default())
                } else {
                    // Otherwise, we flatten the array, knowing there are no `None` values
                    arr.into_iter().flatten().collect::<Vec<T>>().into()
                }
            }
        }
    }
}

impl<T: Eq + Hash> IntoIterator for FilterSet<T> {
    type Item = T;
    type IntoIter = hash_set::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T: Eq + Hash> FromIterator<T> for FilterSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(HashSet::from_iter(iter))
    }
}

impl<T: Eq + Hash> FilterSet<T> {
    /// Returns whether the filter is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns whether the given value matches the filter. It the filter is empty
    /// any value matches. Otherwise, the filter must include the value
    pub fn matches(&self, value: &T) -> bool {
        self.is_empty() || self.0.contains(value)
    }

    /// Returns an iterator over the underlying HashSet. Values are visited
    /// in an arbitrary order.
    pub fn iter(&self) -> hash_set::Iter<'_, T> {
        self.0.iter()
    }
}

impl<T: AsRef<[u8]> + Eq + Hash> FilterSet<T> {
    /// Returns a list of Bloom (BloomFilter) corresponding to the filter's values
    pub fn to_bloom_filter(&self) -> BloomFilter {
        self.0.iter().map(|a| BloomInput::Raw(a.as_ref()).into()).collect::<Vec<Bloom>>().into()
    }
}

impl<T: Clone + Eq + Hash> FilterSet<T> {
    /// Returns a ValueOrArray inside an Option, so that:
    /// - If the filter is empty, it returns None
    /// - If the filter has only 1 value, it returns the single value
    /// - Otherwise it returns an array of values
    pub fn to_value_or_array(&self) -> Option<ValueOrArray<T>> {
        let mut values = self.0.iter().cloned().collect::<Vec<T>>();
        match values.len() {
            0 => None,
            1 => Some(ValueOrArray::Value(values.pop().expect("values length is one"))),
            _ => Some(ValueOrArray::Array(values)),
        }
    }
}

/// A single topic
pub type Topic = FilterSet<B256>;

impl From<U256> for Topic {
    fn from(src: U256) -> Self {
        Into::<B256>::into(src).into()
    }
}

/// Represents errors that can occur when setting block filters in `FilterBlockOption`.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FilterBlockError {
    /// Error indicating that the `from_block` is greater than the `to_block`.
    #[error("`from_block` ({from}) is greater than `to_block` ({to})")]
    FromBlockGreaterThanToBlock {
        /// The starting block number, which is greater than `to`.
        from: u64,
        /// The ending block number, which is less than `from`.
        to: u64,
    },
}

/// Represents the target range of blocks for the filter
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterBlockOption {
    /// Represents a range of blocks with optional from and to blocks
    ///
    /// Note: ranges are considered to be __inclusive__
    Range {
        /// The block number or tag this filter should start at.
        from_block: Option<BlockNumberOrTag>,
        /// The block number or that this filter should end at.
        to_block: Option<BlockNumberOrTag>,
    },
    /// The hash of the block if the filter only targets a single block
    AtBlockHash(BlockHash),
}

impl FilterBlockOption {
    /// Returns the `from_block` value, if any
    pub const fn get_to_block(&self) -> Option<&BlockNumberOrTag> {
        match self {
            Self::Range { to_block, .. } => to_block.as_ref(),
            Self::AtBlockHash(_) => None,
        }
    }

    /// Returns the `to_block` value, if any
    pub const fn get_from_block(&self) -> Option<&BlockNumberOrTag> {
        match self {
            Self::Range { from_block, .. } => from_block.as_ref(),
            Self::AtBlockHash(_) => None,
        }
    }

    /// Returns the range (`from_block`, `to_block`) if this is a range filter.
    pub const fn as_range(&self) -> (Option<&BlockNumberOrTag>, Option<&BlockNumberOrTag>) {
        match self {
            Self::Range { from_block, to_block } => (from_block.as_ref(), to_block.as_ref()),
            Self::AtBlockHash(_) => (None, None),
        }
    }

    /// Returns the block hash if this is a block hash filter.
    pub const fn as_block_hash(&self) -> Option<&BlockHash> {
        match self {
            Self::AtBlockHash(hash) => Some(hash),
            Self::Range { .. } => None,
        }
    }

    /// Returns true if this is a range filter.
    pub const fn is_range(&self) -> bool {
        matches!(self, Self::Range { .. })
    }

    /// Returns true if this is a block hash filter.
    pub const fn is_block_hash(&self) -> bool {
        matches!(self, Self::AtBlockHash(_))
    }

    /// Ensure block range validity
    pub fn ensure_valid_block_range(&self) -> Result<(), FilterBlockError> {
        // Check if from_block is greater than to_block
        if let (Some(from), Some(to)) = (
            self.get_from_block().and_then(|from| from.as_number()),
            self.get_to_block().and_then(|to| to.as_number()),
        ) {
            if from > to {
                return Err(FilterBlockError::FromBlockGreaterThanToBlock { from, to });
            }
        }
        Ok(())
    }

    /// Sets the block number this range filter should start at.
    #[must_use]
    pub fn with_from_block(&self, block: BlockNumberOrTag) -> Self {
        Self::Range { from_block: Some(block), to_block: self.get_to_block().copied() }
    }

    /// Sets the block number this range filter should end at.
    #[must_use]
    pub fn with_to_block(&self, block: BlockNumberOrTag) -> Self {
        Self::Range { from_block: self.get_from_block().copied(), to_block: Some(block) }
    }

    /// Pins the block hash this filter should target.
    #[must_use]
    pub const fn with_block_hash(&self, hash: B256) -> Self {
        Self::AtBlockHash(hash)
    }
}

impl From<BlockNumberOrTag> for FilterBlockOption {
    fn from(block: BlockNumberOrTag) -> Self {
        let block = Some(block);
        Self::Range { from_block: block, to_block: block }
    }
}

impl From<U64> for FilterBlockOption {
    fn from(block: U64) -> Self {
        BlockNumberOrTag::from(block).into()
    }
}

impl From<u64> for FilterBlockOption {
    fn from(block: u64) -> Self {
        BlockNumberOrTag::from(block).into()
    }
}

impl<T: Into<BlockNumberOrTag>> From<RangeInclusive<T>> for FilterBlockOption {
    fn from(r: RangeInclusive<T>) -> Self {
        let (start, end) = r.into_inner();
        let from_block = Some(start.into());
        let to_block = Some(end.into());
        Self::Range { from_block, to_block }
    }
}

impl<T: Into<BlockNumberOrTag>> From<RangeToInclusive<T>> for FilterBlockOption {
    fn from(r: RangeToInclusive<T>) -> Self {
        let to_block = Some(r.end.into());
        Self::Range { from_block: Some(BlockNumberOrTag::Earliest), to_block }
    }
}

impl<T: Into<BlockNumberOrTag>> From<RangeFrom<T>> for FilterBlockOption {
    fn from(r: RangeFrom<T>) -> Self {
        let from_block = Some(r.start.into());
        Self::Range { from_block, to_block: Some(BlockNumberOrTag::Latest) }
    }
}

impl From<B256> for FilterBlockOption {
    fn from(hash: B256) -> Self {
        Self::AtBlockHash(hash)
    }
}

impl Default for FilterBlockOption {
    fn default() -> Self {
        Self::Range { from_block: None, to_block: None }
    }
}

/// Filter for logs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Filter {
    /// Filter block options, specifying on which blocks the filter should match.
    // https://eips.ethereum.org/EIPS/eip-234
    pub block_option: FilterBlockOption,
    /// Address
    pub address: FilterSet<Address>,
    /// Topics (maximum of 4)
    pub topics: [Topic; 4],
}

impl Filter {
    /// Creates a new, empty filter
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the inner filter object
    ///
    /// *NOTE:* ranges are always inclusive
    ///
    /// # Examples
    ///
    /// Match only a specific block
    ///
    /// ```rust
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new().select(69u64);
    /// # }
    /// ```
    /// This is the same as `Filter::new().from_block(1337u64).to_block(1337u64)`
    ///
    /// Match the latest block only
    ///
    /// ```rust
    /// # use alloy_rpc_types_eth::BlockNumberOrTag;
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new().select(BlockNumberOrTag::Latest);
    /// # }
    /// ```
    ///
    /// Match a block by its hash
    ///
    /// ```rust
    /// # use alloy_primitives::B256;
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new().select(B256::ZERO);
    /// # }
    /// ```
    /// This is the same as `at_block_hash`
    ///
    /// Match a range of blocks
    ///
    /// ```rust
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new().select(0u64..=100u64);
    /// # }
    /// ```
    ///
    /// Match all blocks in range `(1337..BlockNumberOrTag::Latest)`
    ///
    /// ```rust
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new().select(1337u64..);
    /// # }
    /// ```
    ///
    /// Match all blocks in range `(BlockNumberOrTag::Earliest..1337)`
    ///
    /// ```rust
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new().select(..=1337u64);
    /// # }
    /// ```
    #[must_use]
    pub fn select(mut self, filter: impl Into<FilterBlockOption>) -> Self {
        self.block_option = filter.into();
        self
    }

    /// Sets the from block number
    #[must_use]
    pub fn from_block<T: Into<BlockNumberOrTag>>(mut self, block: T) -> Self {
        self.block_option = self.block_option.with_from_block(block.into());
        self
    }

    /// Sets the to block number
    #[must_use]
    pub fn to_block<T: Into<BlockNumberOrTag>>(mut self, block: T) -> Self {
        self.block_option = self.block_option.with_to_block(block.into());
        self
    }

    /// Return `true` if filter configured to match pending block.
    /// This means that both from_block and to_block are set to the pending tag.
    pub fn is_pending_block_filter(&self) -> bool {
        self.block_option.get_from_block().is_some_and(BlockNumberOrTag::is_pending)
            && self.block_option.get_to_block().is_some_and(BlockNumberOrTag::is_pending)
    }

    /// Pins the block hash for the filter
    #[must_use]
    pub fn at_block_hash<T: Into<B256>>(mut self, hash: T) -> Self {
        self.block_option = self.block_option.with_block_hash(hash.into());
        self
    }

    /// Sets the address to query with this filter.
    ///
    /// # Examples
    ///
    /// Match only a specific address `("0xAc4b3DacB91461209Ae9d41EC517c2B9Cb1B7DAF")`
    ///
    /// ```rust
    /// # use alloy_primitives::Address;
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let filter = Filter::new()
    ///     .address("0xAc4b3DacB91461209Ae9d41EC517c2B9Cb1B7DAF".parse::<Address>().unwrap());
    /// # }
    /// ```
    ///
    /// Match all addresses in array `(vec!["0xAc4b3DacB91461209Ae9d41EC517c2B9Cb1B7DAF",
    /// "0x8ad599c3A0ff1De082011EFDDc58f1908eb6e6D8"])`
    ///
    /// ```rust
    /// # use alloy_primitives::Address;
    /// # use alloy_rpc_types_eth::Filter;
    /// # fn main() {
    /// let addresses = vec![
    ///     "0xAc4b3DacB91461209Ae9d41EC517c2B9Cb1B7DAF".parse::<Address>().unwrap(),
    ///     "0x8ad599c3A0ff1De082011EFDDc58f1908eb6e6D8".parse::<Address>().unwrap(),
    /// ];
    /// let filter = Filter::new().address(addresses);
    /// # }
    /// ```
    #[must_use]
    pub fn address<T: Into<ValueOrArray<Address>>>(mut self, address: T) -> Self {
        self.address = address.into().into();
        self
    }

    /// Given the event signature in string form, it hashes it and adds it to the topics to monitor
    #[must_use]
    pub fn event(self, event_name: &str) -> Self {
        let hash = keccak256(event_name.as_bytes());
        self.event_signature(hash)
    }

    /// Hashes all event signatures and sets them as array to event_signature(topic0)
    #[must_use]
    pub fn events(self, events: impl IntoIterator<Item = impl AsRef<[u8]>>) -> Self {
        let events = events.into_iter().map(|e| keccak256(e.as_ref())).collect::<Vec<_>>();
        self.event_signature(events)
    }

    /// Sets event_signature(topic0) (the event name for non-anonymous events)
    #[must_use]
    pub fn event_signature<T: Into<Topic>>(mut self, topic: T) -> Self {
        self.topics[0] = topic.into();
        self
    }

    /// Sets topic0 (the event name for non-anonymous events)
    #[must_use]
    #[deprecated(note = "use `event_signature` instead")]
    pub fn topic0<T: Into<Topic>>(mut self, topic: T) -> Self {
        self.topics[0] = topic.into();
        self
    }

    /// Sets the 1st indexed topic
    #[must_use]
    pub fn topic1<T: Into<Topic>>(mut self, topic: T) -> Self {
        self.topics[1] = topic.into();
        self
    }

    /// Sets the 2nd indexed topic
    #[must_use]
    pub fn topic2<T: Into<Topic>>(mut self, topic: T) -> Self {
        self.topics[2] = topic.into();
        self
    }

    /// Sets the 3rd indexed topic
    #[must_use]
    pub fn topic3<T: Into<Topic>>(mut self, topic: T) -> Self {
        self.topics[3] = topic.into();
        self
    }

    /// Returns true if this is a range filter and has a from block
    pub fn is_paginatable(&self) -> bool {
        self.get_from_block().is_some()
    }

    /// Returns the numeric value of the `toBlock` field
    pub fn get_to_block(&self) -> Option<u64> {
        self.block_option.get_to_block().and_then(|b| b.as_number())
    }

    /// Returns the numeric value of the `fromBlock` field
    pub fn get_from_block(&self) -> Option<u64> {
        self.block_option.get_from_block().and_then(|b| b.as_number())
    }

    /// Returns the numeric value of the `fromBlock` field
    pub const fn get_block_hash(&self) -> Option<B256> {
        match self.block_option {
            FilterBlockOption::AtBlockHash(hash) => Some(hash),
            FilterBlockOption::Range { .. } => None,
        }
    }

    /// Returns true if at least one topic is set
    pub fn has_topics(&self) -> bool {
        self.topics.iter().any(|t| !t.is_empty())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Filter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut s = serializer.serialize_struct("Filter", 5)?;
        match self.block_option {
            FilterBlockOption::Range { from_block, to_block } => {
                if let Some(ref from_block) = from_block {
                    s.serialize_field("fromBlock", from_block)?;
                }

                if let Some(ref to_block) = to_block {
                    s.serialize_field("toBlock", to_block)?;
                }
            }

            FilterBlockOption::AtBlockHash(ref h) => s.serialize_field("blockHash", h)?,
        }

        if let Some(address) = self.address.to_value_or_array() {
            s.serialize_field("address", &address)?;
        }

        let mut filtered_topics = Vec::new();
        let mut filtered_topics_len = 0;
        for (i, topic) in self.topics.iter().enumerate() {
            if !topic.is_empty() {
                filtered_topics_len = i + 1;
            }
            filtered_topics.push(topic.to_value_or_array());
        }
        filtered_topics.truncate(filtered_topics_len);
        s.serialize_field("topics", &filtered_topics)?;

        s.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Filter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        type RawAddressFilter = ValueOrArray<Option<Address>>;
        type RawTopicsFilter = Vec<Option<ValueOrArray<Option<B256>>>>;

        struct FilterVisitor;

        impl<'de> serde::de::Visitor<'de> for FilterVisitor {
            type Value = Filter;

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("Filter object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut from_block: Option<Option<BlockNumberOrTag>> = None;
                let mut to_block: Option<Option<BlockNumberOrTag>> = None;
                let mut block_hash: Option<Option<B256>> = None;
                let mut address: Option<Option<RawAddressFilter>> = None;
                let mut topics: Option<Option<RawTopicsFilter>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "fromBlock" => {
                            if from_block.is_some() {
                                return Err(serde::de::Error::duplicate_field("fromBlock"));
                            }
                            from_block = Some(map.next_value()?)
                        }
                        "toBlock" => {
                            if to_block.is_some() {
                                return Err(serde::de::Error::duplicate_field("toBlock"));
                            }
                            to_block = Some(map.next_value()?)
                        }
                        "blockHash" => {
                            if block_hash.is_some() {
                                return Err(serde::de::Error::duplicate_field("blockHash"));
                            }
                            block_hash = Some(map.next_value()?)
                        }
                        "address" => {
                            if address.is_some() {
                                return Err(serde::de::Error::duplicate_field("address"));
                            }
                            address = Some(map.next_value()?)
                        }
                        "topics" => {
                            if topics.is_some() {
                                return Err(serde::de::Error::duplicate_field("topics"));
                            }
                            topics = Some(map.next_value()?)
                        }

                        key => {
                            return Err(serde::de::Error::unknown_field(
                                key,
                                &["fromBlock", "toBlock", "address", "topics", "blockHash"],
                            ))
                        }
                    }
                }

                // conflict check between block_hash and from_block/to_block
                let (block_hash, from_block, to_block) = if let Some(Some(hash)) = block_hash {
                    if from_block.is_some_and(|inner| inner.is_some())
                        || to_block.is_some_and(|inner| inner.is_some())
                    {
                        return Err(serde::de::Error::custom(
                            "cannot specify both blockHash and fromBlock/toBlock, choose one or the other",
                        ));
                    }
                    (Some(hash), None, None)
                } else {
                    (None, from_block.unwrap_or_default(), to_block.unwrap_or_default())
                };

                let address = address.flatten().map(|a| a.into()).unwrap_or_default();
                let topics_vec = topics.flatten().unwrap_or_default();

                // maximum allowed filter len
                if topics_vec.len() > 4 {
                    return Err(serde::de::Error::custom("exceeded maximum topics len"));
                }
                let mut topics: [Topic; 4] = [
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ];
                for (idx, topic) in topics_vec.into_iter().enumerate() {
                    topics[idx] = topic.map(|t| t.into()).unwrap_or_default();
                }

                let block_option = block_hash
                    .map_or(FilterBlockOption::Range { from_block, to_block }, |block_hash| {
                        FilterBlockOption::AtBlockHash(block_hash)
                    });

                Ok(Filter { block_option, address, topics })
            }
        }

        deserializer.deserialize_any(FilterVisitor)
    }
}

/// Union type for representing a single value or a vector of values inside a filter
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ValueOrArray<T> {
    /// A single value
    Value(T),
    /// A vector of values
    Array(Vec<T>),
}

impl<T> ValueOrArray<T> {
    /// Get the value if present.
    pub const fn as_value(&self) -> Option<&T> {
        if let Self::Value(value) = self {
            Some(value)
        } else {
            None
        }
    }

    /// Get the array if present.
    pub fn as_array(&self) -> Option<&[T]> {
        if let Self::Array(array) = self {
            Some(array)
        } else {
            None
        }
    }

    /// Check if the enum is a single value.
    pub const fn is_value(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    /// Check if the enum is an array.
    pub const fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }
}

impl From<Address> for ValueOrArray<Address> {
    fn from(src: Address) -> Self {
        Self::Value(src)
    }
}

impl From<Vec<Address>> for ValueOrArray<Address> {
    fn from(src: Vec<Address>) -> Self {
        Self::Array(src)
    }
}

impl From<Vec<B256>> for ValueOrArray<B256> {
    fn from(src: Vec<B256>) -> Self {
        Self::Array(src)
    }
}

#[cfg(feature = "serde")]
impl<T> serde::Serialize for ValueOrArray<T>
where
    T: serde::Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Value(inner) => inner.serialize(serializer),
            Self::Array(inner) => inner.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'a, T> serde::Deserialize<'a> for ValueOrArray<T>
where
    T: serde::de::DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;

        if value.is_null() {
            return Ok(Self::Array(Vec::new()));
        }

        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Variadic<T> {
            Value(T),
            Array(Vec<T>),
        }

        match serde_json::from_value::<Variadic<T>>(value).map_err(|err| {
            serde::de::Error::custom(format!("Invalid variadic value or array type: {err}"))
        })? {
            Variadic::Value(val) => Ok(Self::Value(val)),
            Variadic::Array(arr) => Ok(Self::Array(arr)),
        }
    }
}

/// Support for matching [Filter]s
#[derive(Debug, Default)]
pub struct FilteredParams {
    /// The original filter, if any
    pub filter: Option<Filter>,
}

impl FilteredParams {
    /// Creates a new wrapper type for a [Filter], if any with flattened topics, that can be used
    /// for matching
    pub fn new(filter: Option<Filter>) -> Self {
        filter.map_or_else(Default::default, |filter| Self { filter: Some(filter) })
    }

    /// Returns the [BloomFilter] for the given address
    pub fn address_filter(address: &FilterSet<Address>) -> BloomFilter {
        address.to_bloom_filter()
    }

    /// Returns the [BloomFilter] for the given topics
    pub fn topics_filter(topics: &[FilterSet<B256>]) -> Vec<BloomFilter> {
        topics.iter().map(|t| t.to_bloom_filter()).collect()
    }

    /// Returns `true` if the bloom matches the topics
    pub fn matches_topics(bloom: Bloom, topic_filters: &[BloomFilter]) -> bool {
        if topic_filters.is_empty() {
            return true;
        }

        // for each filter, iterate through the list of filter blooms. for each set of filter
        // (each BloomFilter), the given `bloom` must match at least one of them, unless the list is
        // empty (no filters).
        for filter in topic_filters {
            if !filter.matches(bloom) {
                return false;
            }
        }
        true
    }

    /// Returns `true` if the bloom contains one of the address blooms, or the address blooms
    /// list is empty (thus, no filters)
    pub fn matches_address(bloom: Bloom, address_filter: &BloomFilter) -> bool {
        address_filter.matches(bloom)
    }

    /// Returns true if the filter matches the given block number
    pub fn filter_block_range(&self, block_number: u64) -> bool {
        if self.filter.is_none() {
            return true;
        }
        let filter = self.filter.as_ref().unwrap();
        let mut res = true;

        if let Some(BlockNumberOrTag::Number(num)) = filter.block_option.get_from_block() {
            if *num > block_number {
                res = false;
            }
        }

        if let Some(to) = filter.block_option.get_to_block() {
            match to {
                BlockNumberOrTag::Number(num) => {
                    if *num < block_number {
                        res = false;
                    }
                }
                BlockNumberOrTag::Earliest => {
                    res = false;
                }
                _ => {}
            }
        }
        res
    }

    /// Returns `true` if the filter matches the given block hash.
    pub fn filter_block_hash(&self, block_hash: B256) -> bool {
        if let Some(h) = self.filter.as_ref().and_then(|f| f.get_block_hash()) {
            if h != block_hash {
                return false;
            }
        }
        true
    }

    /// Return `true` if the filter configured to match pending block.
    /// This means that both from_block and to_block are set to the pending tag.
    /// It calls [`Filter::is_pending_block_filter`] undercover.
    pub fn is_pending_block_filter(&self) -> bool {
        self.filter.as_ref().is_some_and(|f| f.is_pending_block_filter())
    }

    /// Returns `true` if the filter matches the given address.
    pub fn filter_address(&self, address: &Address) -> bool {
        self.filter.as_ref().map(|f| f.address.matches(address)).unwrap_or(true)
    }

    /// Returns `true` if the log matches the given topics
    pub fn filter_topics(&self, log_topics: &[B256]) -> bool {
        let topics = match self.filter.as_ref() {
            None => return true,
            Some(f) => &f.topics,
        };
        for topic_tuple in topics.iter().zip_longest(log_topics.iter()) {
            match topic_tuple {
                // We exhausted the `log.topics`, so if there's a filter set for
                // this topic index, there is no match. Otherwise (empty filter), continue.
                Left(filter_topic) => {
                    if !filter_topic.is_empty() {
                        return false;
                    }
                }
                // We exhausted the filter topics, therefore any subsequent log topic
                // will match.
                Right(_) => return true,
                // Check that `log_topic` is included in `filter_topic`
                Both(filter_topic, log_topic) => {
                    if !filter_topic.matches(log_topic) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Response of the `eth_getFilterChanges` RPC.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum FilterChanges<T = Transaction> {
    /// Empty result.
    #[cfg_attr(feature = "serde", serde(with = "empty_array"))]
    #[default]
    Empty,
    /// New logs.
    Logs(Vec<RpcLog>),
    /// New hashes (block or transactions).
    Hashes(Vec<B256>),
    /// New transactions.
    Transactions(Vec<T>),
}

impl From<Vec<RpcLog>> for FilterChanges {
    fn from(logs: Vec<RpcLog>) -> Self {
        Self::Logs(logs)
    }
}

impl From<Vec<B256>> for FilterChanges {
    fn from(hashes: Vec<B256>) -> Self {
        Self::Hashes(hashes)
    }
}

impl From<Vec<Transaction>> for FilterChanges {
    fn from(transactions: Vec<Transaction>) -> Self {
        Self::Transactions(transactions)
    }
}

impl<T> FilterChanges<T> {
    /// Get the hashes if present.
    pub fn as_hashes(&self) -> Option<&[B256]> {
        if let Self::Hashes(hashes) = self {
            Some(hashes)
        } else {
            None
        }
    }

    /// Get the logs if present.
    pub fn as_logs(&self) -> Option<&[RpcLog]> {
        if let Self::Logs(logs) = self {
            Some(logs)
        } else {
            None
        }
    }

    /// Get the transactions if present.
    pub fn as_transactions(&self) -> Option<&[T]> {
        if let Self::Transactions(transactions) = self {
            Some(transactions)
        } else {
            None
        }
    }

    /// Check if the filter changes are empty.
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Check if the filter changes contain logs.
    pub const fn is_logs(&self) -> bool {
        matches!(self, Self::Logs(_))
    }

    /// Check if the filter changes contain hashes.
    pub const fn is_hashes(&self) -> bool {
        matches!(self, Self::Hashes(_))
    }

    /// Check if the filter changes contain transactions.
    pub const fn is_transactions(&self) -> bool {
        matches!(self, Self::Transactions(_))
    }
}

#[cfg(feature = "serde")]
mod empty_array {
    use serde::{Serialize, Serializer};

    pub(super) fn serialize<S>(s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (&[] as &[()]).serialize(s)
    }
}

#[cfg(feature = "serde")]
impl<'de, T> serde::Deserialize<'de> for FilterChanges<T>
where
    T: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Changes<T = Transaction> {
            Hashes(Vec<B256>),
            Logs(Vec<RpcLog>),
            Transactions(Vec<T>),
        }

        let changes = Changes::deserialize(deserializer)?;
        let changes = match changes {
            Changes::Logs(vals) => {
                if vals.is_empty() {
                    Self::Empty
                } else {
                    Self::Logs(vals)
                }
            }
            Changes::Hashes(vals) => {
                if vals.is_empty() {
                    Self::Empty
                } else {
                    Self::Hashes(vals)
                }
            }
            Changes::Transactions(vals) => {
                if vals.is_empty() {
                    Self::Empty
                } else {
                    Self::Transactions(vals)
                }
            }
        };
        Ok(changes)
    }
}

/// Owned equivalent of a `SubscriptionId`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum FilterId {
    /// Numeric id
    Num(u64),
    /// String id
    Str(String),
}

impl From<u64> for FilterId {
    fn from(num: u64) -> Self {
        Self::Num(num)
    }
}

impl From<String> for FilterId {
    fn from(str: String) -> Self {
        Self::Str(str)
    }
}

#[cfg(feature = "jsonrpsee-types")]
impl From<FilterId> for jsonrpsee_types::SubscriptionId<'_> {
    fn from(value: FilterId) -> Self {
        match value {
            FilterId::Num(n) => jsonrpsee_types::SubscriptionId::Num(n),
            FilterId::Str(s) => jsonrpsee_types::SubscriptionId::Str(s.into()),
        }
    }
}

#[cfg(feature = "jsonrpsee-types")]
impl From<jsonrpsee_types::SubscriptionId<'_>> for FilterId {
    fn from(value: jsonrpsee_types::SubscriptionId<'_>) -> Self {
        match value {
            jsonrpsee_types::SubscriptionId::Num(n) => n.into(),
            jsonrpsee_types::SubscriptionId::Str(s) => s.into_owned().into(),
        }
    }
}
/// Specifies the kind of information you wish to receive from the `eth_newPendingTransactionFilter`
/// RPC endpoint.
///
/// When this type is used in a request, it determines whether the client wishes to receive:
/// - Only the transaction hashes (`Hashes` variant), or
/// - Full transaction details (`Full` variant).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PendingTransactionFilterKind {
    /// Receive only the hashes of the transactions.
    #[default]
    Hashes,
    /// Receive full details of the transactions.
    Full,
}

#[cfg(feature = "serde")]
impl serde::Serialize for PendingTransactionFilterKind {
    /// Serializes the `PendingTransactionFilterKind` into a boolean value:
    /// - `false` for `Hashes`
    /// - `true` for `Full`
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Hashes => false.serialize(serializer),
            Self::Full => true.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'a> serde::Deserialize<'a> for PendingTransactionFilterKind {
    /// Deserializes a boolean value into `PendingTransactionFilterKind`:
    /// - `false` becomes `Hashes`
    /// - `true` becomes `Full`
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let val = Option::<bool>::deserialize(deserializer)?;
        match val {
            Some(true) => Ok(Self::Full),
            _ => Ok(Self::Hashes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use similar_asserts::assert_eq;

    #[cfg(feature = "serde")]
    fn serialize<T: serde::Serialize>(t: &T) -> serde_json::Value {
        serde_json::to_value(t).expect("Failed to serialize value")
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_empty_filter_topics_list() {
        let s = r#"{"fromBlock": "0xfc359e", "toBlock": "0xfc359e", "topics": [["0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"], [], ["0x0000000000000000000000000c17e776cd218252adfca8d4e761d3fe757e9778"]]}"#;
        let filter = serde_json::from_str::<Filter>(s).unwrap();
        assert_eq!(
            filter.topics,
            [
                "0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                Default::default(),
                "0x0000000000000000000000000c17e776cd218252adfca8d4e761d3fe757e9778"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                Default::default(),
            ]
        );
    }

    #[test]
    fn test_with_from_block_correct_range() {
        // Test scenario where from_block is less than to_block
        let original = FilterBlockOption::Range {
            from_block: Some(BlockNumberOrTag::Number(1)),
            to_block: Some(BlockNumberOrTag::Number(10)),
        };
        let updated = original.with_from_block(BlockNumberOrTag::Number(5));
        assert!(updated.ensure_valid_block_range().is_ok());
    }

    #[test]
    fn test_with_from_block_failure() {
        // Test scenario where from_block is greater than to_block
        let original = FilterBlockOption::Range {
            from_block: Some(BlockNumberOrTag::Number(10)),
            to_block: Some(BlockNumberOrTag::Number(5)),
        };

        assert!(matches!(
            original.ensure_valid_block_range(),
            Err(FilterBlockError::FromBlockGreaterThanToBlock { .. })
        ));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_block_hash() {
        let s =
            r#"{"blockHash":"0x58dc57ab582b282c143424bd01e8d923cddfdcda9455bad02a29522f6274a948"}"#;
        let filter = serde_json::from_str::<Filter>(s).unwrap();
        assert_eq!(
            filter.block_option,
            FilterBlockOption::AtBlockHash(
                "0x58dc57ab582b282c143424bd01e8d923cddfdcda9455bad02a29522f6274a948"
                    .parse()
                    .unwrap()
            )
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_filter_topics_middle_wildcard() {
        let s = r#"{"fromBlock": "0xfc359e", "toBlock": "0xfc359e", "topics": [["0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"], [], [null, "0x0000000000000000000000000c17e776cd218252adfca8d4e761d3fe757e9778"]]}"#;
        let filter = serde_json::from_str::<Filter>(s).unwrap();
        assert_eq!(
            filter.topics,
            [
                "0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                Default::default(),
                Default::default(),
                Default::default(),
            ]
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn can_serde_value_or_array() {
        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        struct Item {
            value: ValueOrArray<U256>,
        }

        let item = Item { value: ValueOrArray::Value(U256::from(1u64)) };
        let json = serde_json::to_value(item.clone()).unwrap();
        let deserialized: Item = serde_json::from_value(json).unwrap();
        assert_eq!(item, deserialized);

        let item = Item { value: ValueOrArray::Array(vec![U256::from(1u64), U256::ZERO]) };
        let json = serde_json::to_value(item.clone()).unwrap();
        let deserialized: Item = serde_json::from_value(json).unwrap();
        assert_eq!(item, deserialized);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn filter_serialization_test() {
        let t1 = "0000000000000000000000009729a6fbefefc8f6005933898b13dc45c3a2c8b7"
            .parse::<B256>()
            .unwrap();
        let t2 = B256::from([0; 32]);
        let t3 = U256::from(123);

        let t1_padded = t1;
        let t3_padded = B256::from({
            let mut x = [0; 32];
            x[31] = 123;
            x
        });

        let event = "ValueChanged(address,string,string)";
        let t0 = keccak256(event.as_bytes());
        let addr: Address = "f817796F60D268A36a57b8D2dF1B97B14C0D0E1d".parse().unwrap();
        let filter = Filter::new();

        let ser = serialize(&filter);
        assert_eq!(ser, json!({ "topics": [] }));

        let filter = filter.address(ValueOrArray::Value(addr));

        let ser = serialize(&filter);
        assert_eq!(ser, json!({"address" : addr, "topics": []}));

        let filter = filter.event(event);

        // 0
        let ser = serialize(&filter);
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0]}));

        // 1
        let ser = serialize(&filter.clone().topic1(t1));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, t1_padded]}));

        // 2
        let ser = serialize(&filter.clone().topic2(t2));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, null, t2]}));

        // 3
        let ser = serialize(&filter.clone().topic3(t3));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, null, null, t3_padded]}));

        // 1 & 2
        let ser = serialize(&filter.clone().topic1(t1).topic2(t2));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, t1_padded, t2]}));

        // 1 & 3
        let ser = serialize(&filter.clone().topic1(t1).topic3(t3));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, t1_padded, null, t3_padded]}));

        // 2 & 3
        let ser = serialize(&filter.clone().topic2(t2).topic3(t3));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, null, t2, t3_padded]}));

        // 1 & 2 & 3
        let ser = serialize(&filter.topic1(t1).topic2(t2).topic3(t3));
        assert_eq!(ser, json!({ "address" : addr, "topics": [t0, t1_padded, t2, t3_padded]}));
    }

    fn build_bloom(address: Address, topic1: B256, topic2: B256) -> Bloom {
        let mut block_bloom = Bloom::default();
        block_bloom.accrue(BloomInput::Raw(&address[..]));
        block_bloom.accrue(BloomInput::Raw(&topic1[..]));
        block_bloom.accrue(BloomInput::Raw(&topic2[..]));
        block_bloom
    }

    fn topic_filter(topic1: B256, topic2: B256, topic3: B256) -> Filter {
        Filter {
            block_option: Default::default(),
            address: Default::default(),
            topics: [
                topic1.into(),
                vec![topic2, topic3].into(),
                Default::default(),
                Default::default(),
            ],
        }
    }

    #[test]
    fn can_detect_different_topics() {
        let topic1 = B256::random();
        let topic2 = B256::random();
        let topic3 = B256::random();

        let topics = topic_filter(topic1, topic2, topic3).topics;
        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(!FilteredParams::matches_topics(
            build_bloom(Address::random(), B256::random(), B256::random()),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_topic() {
        let topic1 = B256::random();
        let topic2 = B256::random();
        let topic3 = B256::random();

        let topics = topic_filter(topic1, topic2, topic3).topics;
        let _topics_bloom = FilteredParams::topics_filter(&topics);

        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(FilteredParams::matches_topics(
            build_bloom(Address::random(), topic1, topic2),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_empty_topics() {
        let filter = Filter {
            block_option: Default::default(),
            address: Default::default(),
            topics: Default::default(),
        };
        let topics = filter.topics;

        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(FilteredParams::matches_topics(
            build_bloom(Address::random(), B256::random(), B256::random()),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_address_and_topics() {
        let rng_address = Address::random();
        let topic1 = B256::random();
        let topic2 = B256::random();
        let topic3 = B256::random();

        let filter = Filter {
            block_option: Default::default(),
            address: rng_address.into(),
            topics: [
                topic1.into(),
                vec![topic2, topic3].into(),
                Default::default(),
                Default::default(),
            ],
        };
        let topics = filter.topics;

        let address_filter = FilteredParams::address_filter(&filter.address);
        let topics_filter = FilteredParams::topics_filter(&topics);
        assert!(
            FilteredParams::matches_address(
                build_bloom(rng_address, topic1, topic2),
                &address_filter
            ) && FilteredParams::matches_topics(
                build_bloom(rng_address, topic1, topic2),
                &topics_filter
            )
        );
    }

    #[test]
    fn can_match_topics_wildcard() {
        let topic1 = B256::random();
        let topic2 = B256::random();
        let topic3 = B256::random();

        let filter = Filter {
            block_option: Default::default(),
            address: Default::default(),
            topics: [
                Default::default(),
                vec![topic2, topic3].into(),
                Default::default(),
                Default::default(),
            ],
        };
        let topics = filter.topics;

        let topics_bloom = FilteredParams::topics_filter(&topics);
        assert!(FilteredParams::matches_topics(
            build_bloom(Address::random(), topic1, topic2),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_topics_wildcard_mismatch() {
        let filter = Filter {
            block_option: Default::default(),
            address: Default::default(),
            topics: [
                Default::default(),
                vec![B256::random(), B256::random()].into(),
                Default::default(),
                Default::default(),
            ],
        };
        let topics_input = filter.topics;

        let topics_bloom = FilteredParams::topics_filter(&topics_input);
        assert!(!FilteredParams::matches_topics(
            build_bloom(Address::random(), B256::random(), B256::random()),
            &topics_bloom
        ));
    }

    #[test]
    fn can_match_address_filter() {
        let rng_address = Address::random();
        let filter = Filter {
            block_option: Default::default(),
            address: rng_address.into(),
            topics: Default::default(),
        };
        let address_bloom = FilteredParams::address_filter(&filter.address);
        assert!(FilteredParams::matches_address(
            build_bloom(rng_address, B256::random(), B256::random(),),
            &address_bloom
        ));
    }

    #[test]
    fn can_detect_different_address() {
        let bloom_address = Address::random();
        let rng_address = Address::random();
        let filter = Filter {
            block_option: Default::default(),
            address: rng_address.into(),
            topics: Default::default(),
        };
        let address_bloom = FilteredParams::address_filter(&filter.address);
        assert!(!FilteredParams::matches_address(
            build_bloom(bloom_address, B256::random(), B256::random(),),
            &address_bloom
        ));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn can_convert_to_ethers_filter() {
        let json = json!(
                    {
          "fromBlock": "0x429d3b",
          "toBlock": "0x429d3b",
          "address": "0xb59f67a8bff5d8cd03f6ac17265c550ed8f33907",
          "topics": [
          "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
          "0x00000000000000000000000000b46c2526e227482e2ebb8f4c69e4674d262e75",
          "0x00000000000000000000000054a2d42a40f51259dedd1978f6c118a0f0eff078"
          ]
        }
            );

        let filter: Filter = serde_json::from_value(json).unwrap();
        assert_eq!(
            filter,
            Filter {
                block_option: FilterBlockOption::Range {
                    from_block: Some(4365627u64.into()),
                    to_block: Some(4365627u64.into()),
                },
                address: "0xb59f67a8bff5d8cd03f6ac17265c550ed8f33907"
                    .parse::<Address>()
                    .unwrap()
                    .into(),
                topics: [
                    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
                        .parse::<B256>()
                        .unwrap()
                        .into(),
                    "0x00000000000000000000000000b46c2526e227482e2ebb8f4c69e4674d262e75"
                        .parse::<B256>()
                        .unwrap()
                        .into(),
                    "0x00000000000000000000000054a2d42a40f51259dedd1978f6c118a0f0eff078"
                        .parse::<B256>()
                        .unwrap()
                        .into(),
                    Default::default(),
                ],
            }
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn can_convert_to_ethers_filter_with_null_fields() {
        let json = json!(
                    {
          "fromBlock": "0x429d3b",
          "toBlock": "0x429d3b",
          "address": null,
          "topics": null
        }
            );

        let filter: Filter = serde_json::from_value(json).unwrap();
        assert_eq!(
            filter,
            Filter {
                block_option: FilterBlockOption::Range {
                    from_block: Some(4365627u64.into()),
                    to_block: Some(4365627u64.into()),
                },
                address: Default::default(),
                topics: Default::default(),
            }
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_filter_with_null_range_block() {
        let json = json!(
                    {
          "fromBlock": null,
          "toBlock": null,
          "blockHash": "0xe903ebc49101d30b28d7256be411f81418bf6809ddbaefc40201b1b97f2e64ee",
          "address": null,
          "topics": null
        }
            );

        let filter: Filter = serde_json::from_value(json).unwrap();
        assert_eq!(
            filter.block_option,
            FilterBlockOption::AtBlockHash(
                "0xe903ebc49101d30b28d7256be411f81418bf6809ddbaefc40201b1b97f2e64ee"
                    .parse()
                    .unwrap()
            )
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_filter_with_null_block_hash() {
        let json = json!(
                    {
          "fromBlock": "0x1",
          "toBlock": "0x2",
          "blockHash": null,
          "address": null,
          "topics": null
        }
            );

        let filter: Filter = serde_json::from_value(json).unwrap();
        assert_eq!(
            filter.block_option,
            FilterBlockOption::Range { from_block: Some(1u64.into()), to_block: Some(2u64.into()) }
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_filter_with_null_block_hash_and_null_from_block() {
        let json = json!(
                    {
          "fromBlock": null,
          "toBlock": "0x2",
          "blockHash": null,
          "address": null,
          "topics": null
        }
            );

        let filter: Filter = serde_json::from_value(json).unwrap();
        assert_eq!(
            filter.block_option,
            FilterBlockOption::Range { from_block: None, to_block: Some(2u64.into()) }
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_filter_with_null_block_hash_and_null_to_block() {
        let json = json!(
                    {
          "fromBlock": "0x1",
          "toBlock": null,
          "blockHash": null,
          "address": null,
          "topics": null
        }
            );

        let filter: Filter = serde_json::from_value(json).unwrap();
        assert_eq!(
            filter.block_option,
            FilterBlockOption::Range { from_block: Some(1u64.into()), to_block: None }
        );
    }

    #[test]
    fn test_is_pending_block_filter() {
        let filter = Filter {
            block_option: FilterBlockOption::Range {
                from_block: Some(BlockNumberOrTag::Pending),
                to_block: Some(BlockNumberOrTag::Pending),
            },
            address: "0xb59f67a8bff5d8cd03f6ac17265c550ed8f33907"
                .parse::<Address>()
                .unwrap()
                .into(),
            topics: [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                "0x00000000000000000000000000b46c2526e227482e2ebb8f4c69e4674d262e75"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                "0x00000000000000000000000054a2d42a40f51259dedd1978f6c118a0f0eff078"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                Default::default(),
            ],
        };
        assert!(filter.is_pending_block_filter());
        let filter_params = FilteredParams::new(Some(filter));
        assert!(filter_params.is_pending_block_filter());

        let filter = Filter {
            block_option: FilterBlockOption::Range {
                from_block: Some(4365627u64.into()),
                to_block: Some(4365627u64.into()),
            },
            address: "0xb59f67a8bff5d8cd03f6ac17265c550ed8f33907"
                .parse::<Address>()
                .unwrap()
                .into(),
            topics: [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                "0x00000000000000000000000000b46c2526e227482e2ebb8f4c69e4674d262e75"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                "0x00000000000000000000000054a2d42a40f51259dedd1978f6c118a0f0eff078"
                    .parse::<B256>()
                    .unwrap()
                    .into(),
                Default::default(),
            ],
        };
        assert!(!filter.is_pending_block_filter());
        let filter_params = FilteredParams::new(Some(filter));
        assert!(!filter_params.is_pending_block_filter());
    }
}
