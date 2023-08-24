use ethers_core::utils::rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};
use serde_json::Value;
use std::fmt::{Debug, Display, Formatter, Write};

/// Arbitrary nested data
/// Item::Array(vec![]); is equivalent to []
/// Item::Array(vec![Item::Data(vec![])]); is equivalent to [""] or [null]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Item {
    Data(Vec<u8>),
    Array(Vec<Item>),
}

impl Encodable for Item {
    fn rlp_append(&self, s: &mut RlpStream) {
        match self {
            Item::Array(arr) => {
                s.begin_unbounded_list();
                for item in arr {
                    s.append(item);
                }
                s.finalize_unbounded_list();
            }
            Item::Data(data) => {
                s.append(data);
            }
        }
    }
}

impl Decodable for Item {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.is_data() {
            return Ok(Item::Data(Vec::from(rlp.data()?)))
        }
        Ok(Item::Array(rlp.as_list()?))
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
        Item::Array(iter.into_iter().collect())
    }
}

// Display as hex values
impl Display for Item {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Item::Data(dat) => {
                write!(f, "\"0x{}\"", hex::encode(dat))?;
            }
            Item::Array(arr) => {
                write!(f, "[")?;
                let mut iter = arr.iter().peekable();
                while let Some(item) = iter.next() {
                    write!(f, "{item}")?;
                    if iter.peek().is_some() {
                        f.write_char(',')?;
                    }
                }
                write!(f, "]")?;
            }
        };
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::rlp_converter::Item;
    use ethers_core::utils::{rlp, rlp::DecoderError};
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
    fn encode_decode_test() -> Result<(), DecoderError> {
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
            let encoded = rlp::encode::<Item>(&params.2);
            assert_eq!(rlp::decode::<Item>(&encoded)?, params.2);
            let decoded = rlp::decode::<Item>(&params.1);
            assert_eq!(rlp::encode::<Item>(&decoded?), params.1);
            println!("case {} validated", params.0)
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
            println!("case {} validated", params.0);
        }

        Ok(())
    }
}
