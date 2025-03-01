use super::{super::Nibbles, encode_path_leaf, unpack_path_to_nibbles, RlpNode};
use alloy_primitives::{hex, Bytes};
use alloy_rlp::{length_of_length, BufMut, Decodable, Encodable, Header};
use core::fmt;

#[allow(unused_imports)]
use alloc::vec::Vec;

/// A leaf node represents the endpoint or terminal node in the trie. In other words, a leaf node is
/// where actual values are stored.
///
/// A leaf node consists of two parts: the key (or path) and the value. The key is typically the
/// remaining portion of the key after following the path through the trie, and the value is the
/// data associated with the full key. When searching the trie for a specific key, reaching a leaf
/// node means that the search has successfully found the value associated with that key.
#[derive(PartialEq, Eq, Clone)]
pub struct LeafNode {
    /// The key for this leaf node.
    pub key: Nibbles,
    /// The node value.
    pub value: Vec<u8>,
}

impl fmt::Debug for LeafNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeafNode")
            .field("key", &self.key)
            .field("value", &hex::encode(&self.value))
            .finish()
    }
}

impl Encodable for LeafNode {
    #[inline]
    fn encode(&self, out: &mut dyn BufMut) {
        self.as_ref().encode(out)
    }

    #[inline]
    fn length(&self) -> usize {
        self.as_ref().length()
    }
}

impl Decodable for LeafNode {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let mut bytes = Header::decode_bytes(buf, true)?;
        let encoded_key = Bytes::decode(&mut bytes)?;
        if encoded_key.is_empty() {
            return Err(alloy_rlp::Error::Custom("leaf node key empty"));
        }

        // Retrieve first byte. If it's [Some], then the nibbles are odd.
        let first = match encoded_key[0] & 0xf0 {
            Self::ODD_FLAG => Some(encoded_key[0] & 0x0f),
            Self::EVEN_FLAG => None,
            _ => return Err(alloy_rlp::Error::Custom("node is not leaf")),
        };

        let key = unpack_path_to_nibbles(first, &encoded_key[1..]);
        let value = Bytes::decode(&mut bytes)?.into();
        Ok(Self { key, value })
    }
}

impl LeafNode {
    /// The flag representing the even number of nibbles in the leaf key.
    pub const EVEN_FLAG: u8 = 0x20;

    /// The flag representing the odd number of nibbles in the leaf key.
    pub const ODD_FLAG: u8 = 0x30;

    /// Creates a new leaf node with the given key and value.
    pub const fn new(key: Nibbles, value: Vec<u8>) -> Self {
        Self { key, value }
    }

    /// Return leaf node as [LeafNodeRef].
    pub fn as_ref(&self) -> LeafNodeRef<'_> {
        LeafNodeRef { key: &self.key, value: &self.value }
    }
}

/// Reference to the leaf node. See [LeafNode] from more information.
pub struct LeafNodeRef<'a> {
    /// The key for this leaf node.
    pub key: &'a Nibbles,
    /// The node value.
    pub value: &'a [u8],
}

impl fmt::Debug for LeafNodeRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeafNodeRef")
            .field("key", &self.key)
            .field("value", &hex::encode(self.value))
            .finish()
    }
}

/// Manual implementation of encoding for the leaf node of Merkle Patricia Trie.
impl Encodable for LeafNodeRef<'_> {
    #[inline]
    fn encode(&self, out: &mut dyn BufMut) {
        Header { list: true, payload_length: self.rlp_payload_length() }.encode(out);
        encode_path_leaf(self.key, true).as_slice().encode(out);
        self.value.encode(out);
    }

    #[inline]
    fn length(&self) -> usize {
        let payload_length = self.rlp_payload_length();
        payload_length + length_of_length(payload_length)
    }
}

impl<'a> LeafNodeRef<'a> {
    /// Creates a new leaf node with the given key and value.
    pub const fn new(key: &'a Nibbles, value: &'a [u8]) -> Self {
        Self { key, value }
    }

    /// RLP-encodes the node and returns either `rlp(node)` or `rlp(keccak(rlp(node)))`.
    #[inline]
    pub fn rlp(&self, rlp: &mut Vec<u8>) -> RlpNode {
        self.encode(rlp);
        RlpNode::from_rlp(rlp)
    }

    /// Returns the length of RLP encoded fields of leaf node.
    #[inline]
    fn rlp_payload_length(&self) -> usize {
        let mut encoded_key_len = self.key.len() / 2 + 1;
        // For leaf nodes the first byte cannot be greater than 0x80.
        if encoded_key_len != 1 {
            encoded_key_len += length_of_length(encoded_key_len);
        }
        encoded_key_len + Encodable::length(&self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // From manual regression test
    #[test]
    fn encode_leaf_node_nibble() {
        let nibbles = Nibbles::from_nibbles_unchecked(hex!("0604060f"));
        let encoded = encode_path_leaf(&nibbles, true);
        assert_eq!(encoded[..], hex!("20646f"));
    }

    #[test]
    fn rlp_leaf_node_roundtrip() {
        let nibble = Nibbles::from_nibbles_unchecked(hex!("0604060f"));
        let val = hex!("76657262");
        let leaf = LeafNode::new(nibble, val.to_vec());
        let rlp = leaf.as_ref().rlp(&mut vec![]);
        assert_eq!(rlp.as_ref(), hex!("c98320646f8476657262"));
        assert_eq!(LeafNode::decode(&mut &rlp[..]).unwrap(), leaf);
    }
}
