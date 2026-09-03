use alloy_primitives::{U256, hex};
use alloy_rlp::{Decodable, Encodable, Header, PayloadView};
use eyre::Context;
use serde_json::Value;
use std::fmt;

/// Arbitrary nested data.
///
/// - `Item::Array(vec![])` is equivalent to `[]`.
/// - `Item::Array(vec![Item::Data(vec![])])` is equivalent to `[""]` or `[null]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Data(Vec<u8>),
    Array(Vec<Self>),
}

impl Encodable for Item {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        match self {
            Self::Array(arr) => arr.encode(out),
            Self::Data(data) => <[u8]>::encode(data, out),
        }
    }
}

impl Decodable for Item {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        struct ListFrame<'a> {
            remaining: std::vec::IntoIter<&'a [u8]>,
            items: Vec<Item>,
        }

        let items = match Header::decode_raw(buf)? {
            PayloadView::String(data) => return Ok(Self::Data(data.to_vec())),
            PayloadView::List(items) => items,
        };

        let mut frames = vec![ListFrame { remaining: items.into_iter(), items: Vec::new() }];
        loop {
            let Some(encoded) = frames.last_mut().unwrap().remaining.next() else {
                let frame = frames.pop().unwrap();
                let item = Self::Array(frame.items);
                if let Some(parent) = frames.last_mut() {
                    parent.items.push(item);
                    continue;
                }
                return Ok(item);
            };

            match Header::decode_raw(&mut &encoded[..])? {
                PayloadView::String(data) => {
                    frames.last_mut().unwrap().items.push(Self::Data(data.to_vec()));
                }
                PayloadView::List(items) => {
                    frames.push(ListFrame { remaining: items.into_iter(), items: Vec::new() });
                }
            }
        }
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        // The default recursive drop can overflow after successfully decoding deeply nested RLP.
        let Self::Array(items) = self else { return };
        let mut pending = std::mem::take(items);
        while let Some(mut item) = pending.pop() {
            if let Self::Array(children) = &mut item {
                pending.append(children);
            }
        }
    }
}

impl Item {
    pub(crate) fn value_to_item(value: &Value) -> eyre::Result<Self> {
        match value {
            Value::Null => Ok(Self::Data(vec![])),
            Value::Bool(_) => {
                eyre::bail!("RLP input can not contain booleans");
            }
            Value::Number(n) => {
                Ok(Self::Data(n.to_string().parse::<U256>()?.to_be_bytes_trimmed_vec()))
            }
            Value::String(s) => Ok(Self::Data(hex::decode(s).wrap_err("Could not decode hex")?)),
            Value::Array(values) => values.iter().map(Self::value_to_item).collect(),
            Value::Object(_) => {
                eyre::bail!("RLP input can not contain objects");
            }
        }
    }
}

impl FromIterator<Self> for Item {
    fn from_iter<T: IntoIterator<Item = Self>>(iter: T) -> Self {
        Self::Array(Vec::from_iter(iter))
    }
}

// Display as hex values
impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        enum Task<'a> {
            Item(&'a Item),
            Comma,
            Close,
        }

        let mut tasks = vec![Task::Item(self)];
        while let Some(task) = tasks.pop() {
            match task {
                Task::Item(Self::Data(data)) => write!(f, "\"0x{}\"", hex::encode(data))?,
                Task::Item(Self::Array(items)) => {
                    f.write_str("[")?;
                    tasks.push(Task::Close);
                    for (i, item) in items.iter().enumerate().rev() {
                        tasks.push(Task::Item(item));
                        if i > 0 {
                            tasks.push(Task::Comma);
                        }
                    }
                }
                Task::Comma => f.write_str(",")?,
                Task::Close => f.write_str("]")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::rlp_converter::Item;
    use alloy_primitives::hex;
    use alloy_rlp::{Bytes, Decodable};
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
    #[expect(clippy::disallowed_macros)]
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
            println!("case {} validated", params.0)
        }

        Ok(())
    }

    #[test]
    #[expect(clippy::disallowed_macros)]
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

    #[test]
    fn rlp_data() {
        // <https://github.com/foundry-rs/foundry/issues/9197>
        let hex_val_rlp = hex!("820002");
        let item = Item::decode(&mut &hex_val_rlp[..]).unwrap();

        let data = hex!("0002");
        let encoded = alloy_rlp::encode(&data[..]);
        let decoded: Bytes = alloy_rlp::decode_exact(&encoded[..]).unwrap();
        assert_eq!(Item::Data(decoded.to_vec()), item);

        let hex_val_rlp = hex!("00");
        let item = Item::decode(&mut &hex_val_rlp[..]).unwrap();

        let data = hex!("00");
        let encoded = alloy_rlp::encode(&data[..]);
        let decoded: Bytes = alloy_rlp::decode_exact(&encoded[..]).unwrap();
        assert_eq!(Item::Data(decoded.to_vec()), item);
    }
}
