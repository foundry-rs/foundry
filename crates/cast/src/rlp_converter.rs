use alloy_rlp::{Buf, Decodable, Encodable, Header};
use serde_json::Value;
use std::fmt;

/// Arbitrary nested data
/// Item::Array(vec![]); is equivalent to []
/// Item::Array(vec![Item::Data(vec![])]); is equivalent to [""] or [null]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Data(Vec<u8>),
    Array(Vec<Item>),
}

impl Encodable for Item {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        match self {
            Item::Array(arr) => arr.encode(out),
            Item::Data(data) => <[u8]>::encode(data, out),
        }
    }
}

impl Decodable for Item {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let h = Header::decode(buf)?;
        if buf.len() < h.payload_length {
            return Err(alloy_rlp::Error::InputTooShort);
        }
        let mut d = &buf[..h.payload_length];
        let r = if h.list {
            let view = &mut d;
            let mut v = Vec::new();
            while !view.is_empty() {
                v.push(Item::decode(view)?);
            }
            Ok(Item::Array(v))
        } else {
            Ok(Item::Data(d.to_vec()))
        };
        buf.advance(h.payload_length);
        r
    }
}

impl Item {
    pub(crate) fn value_to_item(value: &Value) -> eyre::Result<Item> {
        return match value {
            Value::Null => Ok(Item::Data(vec![])),
            Value::Bool(_) => {
                eyre::bail!("RLP input should not contain booleans")
            }
            // If a value is passed without quotes we cast it to string
            Value::Number(n) => Ok(Item::value_to_item(&Value::String(n.to_string()))?),
            Value::String(s) => Ok(Item::Data(hex::decode(s).expect("Could not decode hex"))),
            Value::Array(values) => values.iter().map(Item::value_to_item).collect(),
            Value::Object(_) => {
                eyre::bail!("RLP input can not contain objects")
            }
        }
    }
}

impl FromIterator<Item> for Item {
    fn from_iter<T: IntoIterator<Item = Item>>(iter: T) -> Self {
        Item::Array(Vec::from_iter(iter))
    }
}

// Display as hex values
impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Item::Data(dat) => {
                write!(f, "\"0x{}\"", hex::encode(dat))?;
            }
            Item::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    fmt::Display::fmt(item, f)?;
                }
                f.write_str("]")?;
            }
        };
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_macros)]
mod test {
    use crate::rlp_converter::Item;
    use alloy_rlp::Decodable;
    use serde_json::Result as JsonResult;

    // https://en.wikipedia.org/wiki/Set-theoretic_definition_of_natural_numbers
    fn array_von_neuman() -> Item {
        Item::Array(vec![
            Item::Array(vec![]),
            Item::Array(vec![Item::Array(vec![])]),
            Item::Array(vec![Item::Array(vec![]), Item::Array(vec![Item::Array(vec![])])]),
        ])
    }

    #[test]
    fn encode_decode_test() -> alloy_rlp::Result<()> {
        let parameters = vec![
            (1, b"\xc0".to_vec(), Item::Array(vec![])),
            (2, b"\xc1\x80".to_vec(), Item::Array(vec![Item::Data(vec![])])),
            (3, b"\xc4\x83dog".to_vec(), Item::Array(vec![Item::Data(vec![0x64, 0x6f, 0x67])])),
            (
                4,
                b"\xc5\xc4\x83dog".to_vec(),
                Item::Array(vec![Item::Array(vec![Item::Data(vec![0x64, 0x6f, 0x67])])]),
            ),
            (
                5,
                b"\xc8\x83dog\x83cat".to_vec(),
                Item::Array(vec![
                    Item::Data(vec![0x64, 0x6f, 0x67]),
                    Item::Data(vec![0x63, 0x61, 0x74]),
                ]),
            ),
            (6, b"\xc7\xc0\xc1\xc0\xc3\xc0\xc1\xc0".to_vec(), array_von_neuman()),
            (
                7,
                b"\xcd\x83\x6c\x6f\x6c\xc3\xc2\xc1\xc0\xc4\x83\x6f\x6c\x6f".to_vec(),
                Item::Array(vec![
                    Item::Data(vec![b'\x6c', b'\x6f', b'\x6c']),
                    Item::Array(vec![Item::Array(vec![Item::Array(vec![Item::Array(vec![])])])]),
                    Item::Array(vec![Item::Data(vec![b'\x6f', b'\x6c', b'\x6f'])]),
                ]),
            ),
        ];
        for params in parameters {
            let encoded = alloy_rlp::encode(&params.2);
            assert_eq!(Item::decode(&mut &encoded[..])?, params.2);
            let decoded = Item::decode(&mut &params.1[..])?;
            assert_eq!(alloy_rlp::encode(&decoded), params.1);
            println!("case {} validated", params.0);
        }

        Ok(())
    }

    #[test]
    fn deserialize_from_str_test_hex() -> JsonResult<()> {
        let parameters = vec![
            (1, "[\"\"]", Item::Array(vec![Item::Data(vec![])])),
            (2, "[\"0x646f67\"]", Item::Array(vec![Item::Data(vec![0x64, 0x6f, 0x67])])),
            (
                3,
                "[[\"646f67\"]]",
                Item::Array(vec![Item::Array(vec![Item::Data(vec![0x64, 0x6f, 0x67])])]),
            ),
            (
                4,
                "[\"646f67\",\"0x636174\"]",
                Item::Array(vec![
                    Item::Data(vec![0x64, 0x6f, 0x67]),
                    Item::Data(vec![0x63, 0x61, 0x74]),
                ]),
            ),
            (6, "[[],[[]],[[],[[]]]]", array_von_neuman()),
        ];
        for params in parameters {
            let val = serde_json::from_str(params.1)?;
            let item = Item::value_to_item(&val).unwrap();
            assert_eq!(item, params.2);
        }

        Ok(())
    }
}
