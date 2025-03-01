use crate::{Address, Bloom, Bytes, B256};
use alloc::vec::Vec;

#[cfg(feature = "serde")]
mod serde;

/// Compute the logs bloom filter for the given logs.
pub fn logs_bloom<'a>(logs: impl IntoIterator<Item = &'a Log>) -> Bloom {
    let mut bloom = Bloom::ZERO;
    for log in logs {
        bloom.accrue_log(log);
    }
    bloom
}

/// An Ethereum event log object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(derive_arbitrary::Arbitrary, proptest_derive::Arbitrary))]
pub struct LogData {
    /// The indexed topic list.
    topics: Vec<B256>,
    /// The plain data.
    pub data: Bytes,
}

impl LogData {
    /// Creates a new log, without length-checking. This allows creation of
    /// invalid logs. May be safely used when the length of the topic list is
    /// known to be 4 or less.
    #[inline]
    pub const fn new_unchecked(topics: Vec<B256>, data: Bytes) -> Self {
        Self { topics, data }
    }

    /// Creates a new log.
    #[inline]
    pub fn new(topics: Vec<B256>, data: Bytes) -> Option<Self> {
        let this = Self::new_unchecked(topics, data);
        this.is_valid().then_some(this)
    }

    /// Creates a new empty log.
    #[inline]
    pub const fn empty() -> Self {
        Self { topics: Vec::new(), data: Bytes::new() }
    }

    /// True if valid, false otherwise.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.topics.len() <= 4
    }

    /// Get the topic list.
    #[inline]
    pub fn topics(&self) -> &[B256] {
        &self.topics
    }

    /// Get the topic list, mutably. This gives access to the internal
    /// array, without allowing extension of that array.
    #[inline]
    pub fn topics_mut(&mut self) -> &mut [B256] {
        &mut self.topics
    }

    /// Get a mutable reference to the topic list. This allows creation of
    /// invalid logs.
    #[inline]
    pub fn topics_mut_unchecked(&mut self) -> &mut Vec<B256> {
        &mut self.topics
    }

    /// Set the topic list, without length-checking. This allows creation of
    /// invalid logs.
    #[inline]
    pub fn set_topics_unchecked(&mut self, topics: Vec<B256>) {
        self.topics = topics;
    }

    /// Set the topic list, truncating to 4 topics.
    #[inline]
    pub fn set_topics_truncating(&mut self, mut topics: Vec<B256>) {
        topics.truncate(4);
        self.set_topics_unchecked(topics);
    }

    /// Consumes the log data, returning the topic list and the data.
    #[inline]
    pub fn split(self) -> (Vec<B256>, Bytes) {
        (self.topics, self.data)
    }
}

/// Trait for an object that can be converted into a log data object.
pub trait IntoLogData {
    /// Convert into a [`LogData`] object.
    fn to_log_data(&self) -> LogData;
    /// Consume and convert into a [`LogData`] object.
    fn into_log_data(self) -> LogData;
}

impl IntoLogData for LogData {
    #[inline]
    fn to_log_data(&self) -> LogData {
        self.clone()
    }

    #[inline]
    fn into_log_data(self) -> LogData {
        self
    }
}

/// A log consists of an address, and some log data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "arbitrary", derive(derive_arbitrary::Arbitrary, proptest_derive::Arbitrary))]
pub struct Log<T = LogData> {
    /// The address which emitted this log.
    pub address: Address,
    /// The log data.
    pub data: T,
}

impl<T> core::ops::Deref for Log<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> core::ops::DerefMut for Log<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T> AsRef<Self> for Log<T> {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl Log {
    /// Creates a new log.
    #[inline]
    pub fn new(address: Address, topics: Vec<B256>, data: Bytes) -> Option<Self> {
        LogData::new(topics, data).map(|data| Self { address, data })
    }

    /// Creates a new log.
    #[inline]
    pub const fn new_unchecked(address: Address, topics: Vec<B256>, data: Bytes) -> Self {
        Self { address, data: LogData::new_unchecked(topics, data) }
    }

    /// Creates a new empty log.
    #[inline]
    pub const fn empty() -> Self {
        Self { address: Address::ZERO, data: LogData::empty() }
    }
}

impl<T> Log<T>
where
    for<'a> &'a T: Into<LogData>,
{
    /// Creates a new log.
    #[inline]
    pub const fn new_from_event_unchecked(address: Address, data: T) -> Self {
        Self { address, data }
    }

    /// Creates a new log from an deserialized event.
    pub fn new_from_event(address: Address, data: T) -> Option<Self> {
        let this = Self::new_from_event_unchecked(address, data);
        (&this.data).into().is_valid().then_some(this)
    }

    /// Reserialize the data.
    #[inline]
    pub fn reserialize(&self) -> Log<LogData> {
        Log { address: self.address, data: (&self.data).into() }
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Encodable for Log {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        let payload_length =
            self.address.length() + self.data.data.length() + self.data.topics.length();

        alloy_rlp::Header { list: true, payload_length }.encode(out);
        self.address.encode(out);
        self.data.topics.encode(out);
        self.data.data.encode(out);
    }

    fn length(&self) -> usize {
        let payload_length =
            self.address.length() + self.data.data.length() + self.data.topics.length();
        payload_length + alloy_rlp::length_of_length(payload_length)
    }
}

#[cfg(feature = "rlp")]
impl<T> alloy_rlp::Encodable for Log<T>
where
    for<'a> &'a T: Into<LogData>,
{
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.reserialize().encode(out)
    }

    fn length(&self) -> usize {
        self.reserialize().length()
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Decodable for Log {
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        let h = alloy_rlp::Header::decode(buf)?;
        let pre = buf.len();

        let address = alloy_rlp::Decodable::decode(buf)?;
        let topics = alloy_rlp::Decodable::decode(buf)?;
        let data = alloy_rlp::Decodable::decode(buf)?;

        if h.payload_length != pre - buf.len() {
            return Err(alloy_rlp::Error::Custom("did not consume exact payload"));
        }

        Ok(Self { address, data: LogData { topics, data } })
    }
}

#[cfg(feature = "rlp")]
#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rlp::{Decodable, Encodable};

    #[test]
    fn test_roundtrip_rlp_log_data() {
        let log = Log::<LogData>::default();
        let mut buf = Vec::<u8>::new();
        log.encode(&mut buf);
        assert_eq!(Log::decode(&mut &buf[..]).unwrap(), log);
    }
}
