use crate::eth::utils::enveloped;
use ethers_core::{
    types::{Address, Bloom, Bytes, H256, U256},
    utils::{
        rlp,
        rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream},
    },
};
use foundry_common::types::{ToAlloy, ToEthers};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Log {
    pub address: Address,
    pub topics: Vec<H256>,
    pub data: Bytes,
}

impl From<revm::primitives::Log> for Log {
    fn from(log: revm::primitives::Log) -> Self {
        let revm::primitives::Log { address, topics, data } = log;
        Log {
            address: address.to_ethers(),
            topics: topics.into_iter().map(|h| h.to_ethers()).collect(),
            data: ethers_core::types::Bytes(data.0),
        }
    }
}

impl From<Log> for revm::primitives::Log {
    fn from(log: Log) -> Self {
        let Log { address, topics, data } = log;
        revm::primitives::Log {
            address: address.to_alloy(),
            topics: topics.into_iter().map(|t| t.to_alloy()).collect(),
            data: alloy_primitives::Bytes(data.0),
        }
    }
}

impl Encodable for Log {
    fn rlp_append(&self, stream: &mut rlp::RlpStream) {
        stream.begin_list(3);
        stream.append(&self.address);
        stream.append_list(&self.topics);
        stream.append(&self.data.as_ref());
    }
}

impl Decodable for Log {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let result = Log {
            address: rlp.val_at(0)?,
            topics: rlp.list_at(1)?,
            data: rlp.val_at::<Vec<u8>>(2)?.into(),
        };
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EIP658Receipt {
    pub status_code: u8,
    pub gas_used: U256,
    pub logs_bloom: Bloom,
    pub logs: Vec<Log>,
}

impl Encodable for EIP658Receipt {
    fn rlp_append(&self, stream: &mut RlpStream) {
        stream.begin_list(4);
        stream.append(&self.status_code);
        stream.append(&self.gas_used);
        stream.append(&self.logs_bloom);
        stream.append_list(&self.logs);
    }
}

impl Decodable for EIP658Receipt {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let result = EIP658Receipt {
            status_code: rlp.val_at(0)?,
            gas_used: rlp.val_at(1)?,
            logs_bloom: rlp.val_at(2)?,
            logs: rlp.list_at(3)?,
        };
        Ok(result)
    }
}

// same underlying data structure
pub type EIP2930Receipt = EIP658Receipt;
pub type EIP1559Receipt = EIP658Receipt;

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypedReceipt {
    /// Legacy receipt
    Legacy(EIP658Receipt),
    /// EIP-2930 receipt
    EIP2930(EIP2930Receipt),
    /// EIP-1559 receipt
    EIP1559(EIP1559Receipt),
}

// == impl TypedReceipt ==

impl TypedReceipt {
    /// Returns the gas used by the transactions
    pub fn gas_used(&self) -> U256 {
        match self {
            TypedReceipt::Legacy(r) | TypedReceipt::EIP2930(r) | TypedReceipt::EIP1559(r) => {
                r.gas_used
            }
        }
    }

    /// Returns the gas used by the transactions
    pub fn logs_bloom(&self) -> &Bloom {
        match self {
            TypedReceipt::Legacy(r) | TypedReceipt::EIP2930(r) | TypedReceipt::EIP1559(r) => {
                &r.logs_bloom
            }
        }
    }
}

impl Encodable for TypedReceipt {
    fn rlp_append(&self, s: &mut RlpStream) {
        match self {
            TypedReceipt::Legacy(r) => r.rlp_append(s),
            TypedReceipt::EIP2930(r) => enveloped(1, r, s),
            TypedReceipt::EIP1559(r) => enveloped(2, r, s),
        }
    }
}

impl Decodable for TypedReceipt {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let slice = rlp.data()?;

        let first = *slice.first().ok_or(DecoderError::Custom("empty receipt"))?;

        if rlp.is_list() {
            return Ok(TypedReceipt::Legacy(Decodable::decode(rlp)?))
        }

        let s = slice.get(1..).ok_or(DecoderError::Custom("no receipt content"))?;

        if first == 0x01 {
            return rlp::decode(s).map(TypedReceipt::EIP2930)
        }

        if first == 0x02 {
            return rlp::decode(s).map(TypedReceipt::EIP1559)
        }

        Err(DecoderError::Custom("unknown receipt type"))
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Encodable for TypedReceipt {
    fn length(&self) -> usize {
        match self {
            TypedReceipt::Legacy(r) => r.length(),
            receipt => {
                let payload_len = match receipt {
                    TypedReceipt::EIP2930(r) => r.length() + 1,
                    TypedReceipt::EIP1559(r) => r.length() + 1,
                    _ => unreachable!("receipt already matched"),
                };

                // we include a string header for typed receipts, so include the length here
                payload_len + open_fastrlp::length_of_length(payload_len)
            }
        }
    }
    fn encode(&self, out: &mut dyn open_fastrlp::BufMut) {
        use open_fastrlp::Header;

        match self {
            TypedReceipt::Legacy(r) => r.encode(out),
            receipt => {
                let payload_len = match receipt {
                    TypedReceipt::EIP2930(r) => r.length() + 1,
                    TypedReceipt::EIP1559(r) => r.length() + 1,
                    _ => unreachable!("receipt already matched"),
                };

                match receipt {
                    TypedReceipt::EIP2930(r) => {
                        let receipt_string_header =
                            Header { list: false, payload_length: payload_len };

                        receipt_string_header.encode(out);
                        out.put_u8(0x01);
                        r.encode(out);
                    }
                    TypedReceipt::EIP1559(r) => {
                        let receipt_string_header =
                            Header { list: false, payload_length: payload_len };

                        receipt_string_header.encode(out);
                        out.put_u8(0x02);
                        r.encode(out);
                    }
                    _ => unreachable!("receipt already matched"),
                }
            }
        }
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Decodable for TypedReceipt {
    fn decode(buf: &mut &[u8]) -> Result<Self, open_fastrlp::DecodeError> {
        use bytes::Buf;
        use open_fastrlp::Header;
        use std::cmp::Ordering;

        // a receipt is either encoded as a string (non legacy) or a list (legacy).
        // We should not consume the buffer if we are decoding a legacy receipt, so let's
        // check if the first byte is between 0x80 and 0xbf.
        let rlp_type = *buf
            .first()
            .ok_or(open_fastrlp::DecodeError::Custom("cannot decode a receipt from empty bytes"))?;

        match rlp_type.cmp(&open_fastrlp::EMPTY_LIST_CODE) {
            Ordering::Less => {
                // strip out the string header
                let _header = Header::decode(buf)?;
                let receipt_type = *buf.first().ok_or(open_fastrlp::DecodeError::Custom(
                    "typed receipt cannot be decoded from an empty slice",
                ))?;
                if receipt_type == 0x01 {
                    buf.advance(1);
                    <EIP2930Receipt as open_fastrlp::Decodable>::decode(buf)
                        .map(TypedReceipt::EIP2930)
                } else if receipt_type == 0x02 {
                    buf.advance(1);
                    <EIP1559Receipt as open_fastrlp::Decodable>::decode(buf)
                        .map(TypedReceipt::EIP1559)
                } else {
                    Err(open_fastrlp::DecodeError::Custom("invalid receipt type"))
                }
            }
            Ordering::Equal => Err(open_fastrlp::DecodeError::Custom(
                "an empty list is not a valid receipt encoding",
            )),
            Ordering::Greater => {
                <EIP658Receipt as open_fastrlp::Decodable>::decode(buf).map(TypedReceipt::Legacy)
            }
        }
    }
}

impl From<TypedReceipt> for EIP658Receipt {
    fn from(v3: TypedReceipt) -> Self {
        match v3 {
            TypedReceipt::Legacy(receipt) => receipt,
            TypedReceipt::EIP2930(receipt) => receipt,
            TypedReceipt::EIP1559(receipt) => receipt,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "fastrlp")]
    // Test vector from: https://eips.ethereum.org/EIPS/eip-2481
    fn encode_legacy_receipt() {
        use crate::eth::receipt::{EIP658Receipt, Log, TypedReceipt};
        use ethers_core::{
            types::{Bytes, H160, H256},
            utils::hex,
        };
        use open_fastrlp::Encodable;
        use std::str::FromStr;

        let expected = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let mut data = vec![];
        let receipt = TypedReceipt::Legacy(EIP658Receipt {
            logs_bloom: [0; 256].into(),
            gas_used: 0x1u64.into(),
            logs: vec![Log {
                address: H160::from_str("0000000000000000000000000000000000000011").unwrap(),
                topics: vec![
                    H256::from_str(
                        "000000000000000000000000000000000000000000000000000000000000dead",
                    )
                    .unwrap(),
                    H256::from_str(
                        "000000000000000000000000000000000000000000000000000000000000beef",
                    )
                    .unwrap(),
                ],
                data: Bytes::from_str("0100ff").unwrap(),
            }],
            status_code: 0,
        });
        receipt.encode(&mut data);

        // check that the rlp length equals the length of the expected rlp
        assert_eq!(receipt.length(), expected.len());
        assert_eq!(data, expected);
    }

    #[test]
    #[cfg(feature = "fastrlp")]
    // Test vector from: https://eips.ethereum.org/EIPS/eip-2481
    fn decode_legacy_receipt() {
        use crate::eth::receipt::{EIP658Receipt, Log, TypedReceipt};
        use ethers_core::{
            types::{Bytes, H160, H256},
            utils::hex,
        };
        use open_fastrlp::Decodable;
        use std::str::FromStr;

        let data = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let expected = TypedReceipt::Legacy(EIP658Receipt {
            logs_bloom: [0; 256].into(),
            gas_used: 0x1u64.into(),
            logs: vec![Log {
                address: H160::from_str("0000000000000000000000000000000000000011").unwrap(),
                topics: vec![
                    H256::from_str(
                        "000000000000000000000000000000000000000000000000000000000000dead",
                    )
                    .unwrap(),
                    H256::from_str(
                        "000000000000000000000000000000000000000000000000000000000000beef",
                    )
                    .unwrap(),
                ],
                data: Bytes::from_str("0100ff").unwrap(),
            }],
            status_code: 0,
        });

        let receipt = TypedReceipt::decode(&mut &data[..]).unwrap();
        assert_eq!(receipt, expected);
    }
}
