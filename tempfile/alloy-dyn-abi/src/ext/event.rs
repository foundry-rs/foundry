use crate::{DecodedEvent, DynSolEvent, DynSolType, Error, Result, Specifier};
use alloc::vec::Vec;
use alloy_json_abi::Event;
use alloy_primitives::{LogData, B256};

#[allow(unknown_lints, unnameable_types)]
mod sealed {
    pub trait Sealed {}
    impl Sealed for alloy_json_abi::Event {}
}
use sealed::Sealed;

impl Specifier<DynSolEvent> for Event {
    fn resolve(&self) -> Result<DynSolEvent> {
        let mut indexed = Vec::with_capacity(self.inputs.len());
        let mut body = Vec::with_capacity(self.inputs.len());
        for param in &self.inputs {
            let ty = param.resolve()?;
            if param.indexed {
                indexed.push(ty);
            } else {
                body.push(ty);
            }
        }
        let topic_0 = if self.anonymous { None } else { Some(self.selector()) };

        let num_topics = indexed.len() + topic_0.is_some() as usize;
        if num_topics > 4 {
            return Err(Error::TopicLengthMismatch { expected: 4, actual: num_topics });
        }

        Ok(DynSolEvent::new_unchecked(topic_0, indexed, DynSolType::Tuple(body)))
    }
}

/// Provides event encoding and decoding for the [`Event`] type.
///
/// This trait is sealed and cannot be implemented for types outside of this
/// crate. It is implemented only for [`Event`].
pub trait EventExt: Sealed {
    /// Decodes the given log info according to this item's input types.
    ///
    /// The `topics` parameter is the list of indexed topics, and the `data`
    /// parameter is the non-indexed data.
    ///
    /// The first topic is skipped, unless the event is anonymous.
    ///
    /// For more details, see the [Solidity reference][ref].
    ///
    /// [ref]: https://docs.soliditylang.org/en/latest/abi-spec.html#encoding-of-indexed-event-parameters
    ///
    /// # Errors
    ///
    /// This function will return an error if the decoded data does not match
    /// the expected input types.
    fn decode_log_parts<I>(&self, topics: I, data: &[u8], validate: bool) -> Result<DecodedEvent>
    where
        I: IntoIterator<Item = B256>;

    /// Decodes the given log object according to this item's input types.
    ///
    /// See [`decode_log`](EventExt::decode_log).
    #[inline]
    fn decode_log(&self, log: &LogData, validate: bool) -> Result<DecodedEvent> {
        self.decode_log_parts(log.topics().iter().copied(), &log.data, validate)
    }
}

impl EventExt for Event {
    fn decode_log_parts<I>(&self, topics: I, data: &[u8], validate: bool) -> Result<DecodedEvent>
    where
        I: IntoIterator<Item = B256>,
    {
        self.resolve()?.decode_log_parts(topics, data, validate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DynSolValue;
    use alloy_json_abi::EventParam;
    use alloy_primitives::{address, b256, bytes, hex, keccak256, Signed};

    #[test]
    fn empty() {
        let mut event = Event { name: "MyEvent".into(), inputs: vec![], anonymous: false };

        // skips over hash
        let values = event.decode_log_parts(None, &[], false).unwrap();
        assert!(values.indexed.is_empty());
        assert!(values.body.is_empty());

        // but if we validate, we get an error
        let err = event.decode_log_parts(None, &[], true).unwrap_err();
        assert_eq!(err, Error::TopicLengthMismatch { expected: 1, actual: 0 });

        let values = event.decode_log_parts(Some(keccak256("MyEvent()")), &[], true).unwrap();
        assert!(values.indexed.is_empty());
        assert!(values.body.is_empty());
        event.anonymous = true;
        let values = event.decode_log_parts(None, &[], false).unwrap();
        assert!(values.indexed.is_empty());
        assert!(values.body.is_empty());
        let values = event.decode_log_parts(None, &[], true).unwrap();
        assert!(values.indexed.is_empty());
        assert!(values.body.is_empty());
    }

    // https://github.com/rust-ethereum/ethabi/blob/b1710adc18f5b771d2d2519c87248b1ba9430778/ethabi/src/event.rs#L192
    #[test]
    fn test_decoding_event() {
        let event = Event {
            name: "foo".into(),
            inputs: vec![
                EventParam { ty: "int256".into(), indexed: false, ..Default::default() },
                EventParam { ty: "int256".into(), indexed: true, ..Default::default() },
                EventParam { ty: "address".into(), indexed: false, ..Default::default() },
                EventParam { ty: "address".into(), indexed: true, ..Default::default() },
                EventParam { ty: "string".into(), indexed: true, ..Default::default() },
            ],
            anonymous: false,
        };

        let result = event
            .decode_log_parts(
                [
                    b256!("0x0000000000000000000000000000000000000000000000000000000000000000"),
                    b256!("0x0000000000000000000000000000000000000000000000000000000000000002"),
                    b256!("0x0000000000000000000000001111111111111111111111111111111111111111"),
                    b256!("0x00000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                    b256!("0x00000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                    b256!("0x00000000000000000ccccccccccccccccccccccccccccccccccccccccccccccc"),
                ],
                &hex!(
                    "
                    0000000000000000000000000000000000000000000000000000000000000003
                    0000000000000000000000002222222222222222222222222222222222222222
                "
                ),
                false,
            )
            .unwrap();

        assert_eq!(
            result.body,
            [
                DynSolValue::Int(
                    Signed::from_be_bytes(hex!(
                        "0000000000000000000000000000000000000000000000000000000000000003"
                    )),
                    256
                ),
                DynSolValue::Address(address!("0x2222222222222222222222222222222222222222")),
            ]
        );
        assert_eq!(
            result.indexed,
            [
                DynSolValue::Int(
                    Signed::from_be_bytes(hex!(
                        "0000000000000000000000000000000000000000000000000000000000000002"
                    )),
                    256
                ),
                DynSolValue::Address(address!("0x1111111111111111111111111111111111111111")),
                DynSolValue::FixedBytes(
                    b256!("0x00000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                    32
                ),
            ]
        )
    }

    #[test]
    fn parse_log_whole() {
        let correct_event = Event {
            name: "Test".into(),
            inputs: vec![
                EventParam { ty: "(address,address)".into(), indexed: false, ..Default::default() },
                EventParam { ty: "address".into(), indexed: true, ..Default::default() },
            ],
            anonymous: false,
        };
        // swap indexed params
        let mut wrong_event = correct_event.clone();
        wrong_event.inputs[0].indexed = true;
        wrong_event.inputs[1].indexed = false;

        let log = LogData::new_unchecked(
            vec![
                b256!("0xcf74b4e62f836eeedcd6f92120ffb5afea90e6fa490d36f8b81075e2a7de0cf7"),
                b256!("0x0000000000000000000000000000000000000000000000000000000000012321"),
            ],
            bytes!(
                "
			0000000000000000000000000000000000000000000000000000000000012345
			0000000000000000000000000000000000000000000000000000000000054321
			"
            ),
        );

        wrong_event.decode_log(&log, false).unwrap();
        // TODO: How do we verify here?
        // wrong_event.decode_log_object(&log, true).unwrap_err();
        correct_event.decode_log(&log, false).unwrap();
        correct_event.decode_log(&log, true).unwrap();
    }
}
