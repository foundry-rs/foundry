use crate::{DynSolType, DynSolValue, Error, Result};
use alloc::vec::Vec;
use alloy_primitives::{IntoLogData, Log, LogData, B256};

/// A dynamic ABI event.
///
/// This is a representation of a Solidity event, which can be used to decode
/// logs.
#[derive(Clone, Debug, PartialEq)]
pub struct DynSolEvent {
    /// The event signature hash, if any.
    pub(crate) topic_0: Option<B256>,
    /// The indexed types.
    pub(crate) indexed: Vec<DynSolType>,
    /// The un-indexed types.
    pub(crate) body: DynSolType,
}

impl DynSolEvent {
    /// Creates a new event, without length-checking the indexed, or ensuring
    /// the body is a tuple. This allows creation of invalid events.
    pub const fn new_unchecked(
        topic_0: Option<B256>,
        indexed: Vec<DynSolType>,
        body: DynSolType,
    ) -> Self {
        Self { topic_0, indexed, body }
    }

    /// Creates a new event.
    ///
    /// Checks that the indexed length is less than or equal to 4, and that the
    /// body is a tuple.
    pub fn new(topic_0: Option<B256>, indexed: Vec<DynSolType>, body: DynSolType) -> Option<Self> {
        let topics = indexed.len() + topic_0.is_some() as usize;
        if topics > 4 || body.as_tuple().is_none() {
            return None;
        }
        Some(Self::new_unchecked(topic_0, indexed, body))
    }

    /// True if anonymous.
    pub const fn is_anonymous(&self) -> bool {
        self.topic_0.is_none()
    }

    /// Decode the event from the given log info.
    pub fn decode_log_parts<I>(
        &self,
        topics: I,
        data: &[u8],
        validate: bool,
    ) -> Result<DecodedEvent>
    where
        I: IntoIterator<Item = B256>,
    {
        let mut topics = topics.into_iter();
        let num_topics = self.indexed.len() + !self.is_anonymous() as usize;
        if validate {
            match topics.size_hint() {
                (n, Some(m)) if n == m && n != num_topics => {
                    return Err(Error::TopicLengthMismatch { expected: num_topics, actual: n })
                }
                _ => {}
            }
        }

        // skip event hash if not anonymous
        if !self.is_anonymous() {
            let t = topics.next();
            // Validate only if requested
            if validate {
                match t {
                    Some(sig) => {
                        let expected = self.topic_0.expect("not anonymous");
                        if sig != expected {
                            return Err(Error::EventSignatureMismatch { expected, actual: sig });
                        }
                    }
                    None => {
                        return Err(Error::TopicLengthMismatch { expected: num_topics, actual: 0 })
                    }
                }
            }
        }

        let indexed = self
            .indexed
            .iter()
            .zip(topics.by_ref().take(self.indexed.len()))
            .map(|(ty, topic)| {
                let value = ty.decode_event_topic(topic);
                Ok(value)
            })
            .collect::<Result<_>>()?;

        let body = self.body.abi_decode_sequence(data)?.into_fixed_seq().expect("body is a tuple");

        if validate {
            let remaining = topics.count();
            if remaining > 0 {
                return Err(Error::TopicLengthMismatch {
                    expected: num_topics,
                    actual: num_topics + remaining,
                });
            }
        }

        Ok(DecodedEvent { selector: self.topic_0, indexed, body })
    }

    /// Decode the event from the given log info.
    pub fn decode_log_data(&self, log: &LogData, validate: bool) -> Result<DecodedEvent> {
        self.decode_log_parts(log.topics().iter().copied(), &log.data, validate)
    }

    /// Get the selector for this event, if any.
    pub const fn topic_0(&self) -> Option<B256> {
        self.topic_0
    }

    /// Get the indexed types.
    pub fn indexed(&self) -> &[DynSolType] {
        &self.indexed
    }

    /// Get the un-indexed types.
    pub fn body(&self) -> &[DynSolType] {
        self.body.as_tuple().expect("body is a tuple")
    }
}

/// A decoded dynamic ABI event.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedEvent {
    /// The hashes event_signature (if any)
    #[doc(alias = "topic_0")]
    pub selector: Option<B256>,
    /// The indexed values, in order.
    pub indexed: Vec<DynSolValue>,
    /// The un-indexed values, in order.
    pub body: Vec<DynSolValue>,
}

impl DecodedEvent {
    /// True if anonymous. False if not.
    pub const fn is_anonymous(&self) -> bool {
        self.selector.is_none()
    }

    /// Re-encode the event into a [`LogData`]
    pub fn encode_log_data(&self) -> LogData {
        debug_assert!(
            self.indexed.len() + !self.is_anonymous() as usize <= 4,
            "too many indexed values"
        );

        LogData::new_unchecked(
            self.selector
                .iter()
                .copied()
                .chain(self.indexed.iter().flat_map(DynSolValue::as_word))
                .collect(),
            DynSolValue::encode_seq(&self.body).into(),
        )
    }

    /// Transform a [`Log`] containing this event into a [`Log`] containing
    /// [`LogData`].
    pub fn encode_log(log: Log<Self>) -> Log<LogData> {
        Log { address: log.address, data: log.data.encode_log_data() }
    }
}

impl IntoLogData for DecodedEvent {
    fn to_log_data(&self) -> LogData {
        self.encode_log_data()
    }

    fn into_log_data(self) -> LogData {
        self.encode_log_data()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use alloy_primitives::{address, b256, bytes, U256};

    #[test]
    fn it_decodes_a_simple_log() {
        let log = LogData::new_unchecked(vec![], U256::ZERO.to_be_bytes_vec().into());
        let event = DynSolEvent {
            topic_0: None,
            indexed: vec![],
            body: DynSolType::Tuple(vec![DynSolType::Uint(256)]),
        };
        event.decode_log_data(&log, true).unwrap();
    }

    #[test]
    fn it_decodes_logs_with_indexed_params() {
        let t0 = b256!("0xcf74b4e62f836eeedcd6f92120ffb5afea90e6fa490d36f8b81075e2a7de0cf7");
        let log: LogData = LogData::new_unchecked(
            vec![t0, b256!("0x0000000000000000000000000000000000000000000000000000000000012321")],
            bytes!(
                "
			    0000000000000000000000000000000000000000000000000000000000012345
			    0000000000000000000000000000000000000000000000000000000000054321
			    "
            ),
        );
        let event = DynSolEvent {
            topic_0: Some(t0),
            indexed: vec![DynSolType::Address],
            body: DynSolType::Tuple(vec![DynSolType::Tuple(vec![
                DynSolType::Address,
                DynSolType::Address,
            ])]),
        };

        let decoded = event.decode_log_data(&log, true).unwrap();
        assert_eq!(
            decoded.indexed,
            vec![DynSolValue::Address(address!("0x0000000000000000000000000000000000012321"))]
        );

        let encoded = decoded.encode_log_data();
        assert_eq!(encoded, log);
    }
}
