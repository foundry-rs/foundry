use super::{super::Nibbles, encode_path_leaf, unpack_path_to_nibbles, RlpNode};
use alloy_primitives::{hex, Bytes};
use alloy_rlp::{length_of_length, BufMut, Decodable, Encodable, Header};
use core::fmt;

#[allow(unused_imports)]
use alloc::vec::Vec;

/// An extension node in an Ethereum Merkle Patricia Trie.
///
/// An intermediate node that exists solely to compress the trie's paths. It contains a path segment
/// (a shared prefix of keys) and a single child pointer. Essentially, an extension node can be
/// thought of as a shortcut within the trie to reduce its overall depth.
///
/// The purpose of an extension node is to optimize the trie structure by collapsing multiple nodes
/// with a single child into one node. This simplification reduces the space and computational
/// complexity when performing operations on the trie.
#[derive(PartialEq, Eq, Clone)]
pub struct ExtensionNode {
    /// The key for this extension node.
    pub key: Nibbles,
    /// A pointer to the child node.
    pub child: RlpNode,
}

impl fmt::Debug for ExtensionNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionNode")
            .field("key", &self.key)
            .field("child", &hex::encode(&self.child))
            .finish()
    }
}

impl Encodable for ExtensionNode {
    #[inline]
    fn encode(&self, out: &mut dyn BufMut) {
        self.as_ref().encode(out)
    }

    #[inline]
    fn length(&self) -> usize {
        self.as_ref().length()
    }
}

impl Decodable for ExtensionNode {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let mut bytes = Header::decode_bytes(buf, true)?;
        let encoded_key = Bytes::decode(&mut bytes)?;
        if encoded_key.is_empty() {
            return Err(alloy_rlp::Error::Custom("extension node key empty"));
        }

        // Retrieve first byte. If it's [Some], then the nibbles are odd.
        let first = match encoded_key[0] & 0xf0 {
            Self::ODD_FLAG => Some(encoded_key[0] & 0x0f),
            Self::EVEN_FLAG => None,
            _ => return Err(alloy_rlp::Error::Custom("node is not extension")),
        };

        let key = unpack_path_to_nibbles(first, &encoded_key[1..]);
        let child = RlpNode::from_raw_rlp(bytes)?;
        Ok(Self { key, child })
    }
}

impl ExtensionNode {
    /// The flag representing the even number of nibbles in the extension key.
    pub const EVEN_FLAG: u8 = 0x00;

    /// The flag representing the odd number of nibbles in the extension key.
    pub const ODD_FLAG: u8 = 0x10;

    /// Creates a new extension node with the given key and a pointer to the child.
    pub const fn new(key: Nibbles, child: RlpNode) -> Self {
        Self { key, child }
    }

    /// Return extension node as [ExtensionNodeRef].
    pub fn as_ref(&self) -> ExtensionNodeRef<'_> {
        ExtensionNodeRef { key: &self.key, child: &self.child }
    }
}

/// Reference to the extension node. See [ExtensionNode] from more information.
pub struct ExtensionNodeRef<'a> {
    /// The key for this extension node.
    pub key: &'a Nibbles,
    /// A pointer to the child node.
    pub child: &'a [u8],
}

impl fmt::Debug for ExtensionNodeRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionNodeRef")
            .field("key", &self.key)
            .field("node", &hex::encode(self.child))
            .finish()
    }
}

impl Encodable for ExtensionNodeRef<'_> {
    #[inline]
    fn encode(&self, out: &mut dyn BufMut) {
        Header { list: true, payload_length: self.rlp_payload_length() }.encode(out);
        encode_path_leaf(self.key, false).as_slice().encode(out);
        // Pointer to the child is already RLP encoded.
        out.put_slice(self.child);
    }

    #[inline]
    fn length(&self) -> usize {
        let payload_length = self.rlp_payload_length();
        payload_length + length_of_length(payload_length)
    }
}

impl<'a> ExtensionNodeRef<'a> {
    /// Creates a new extension node with the given key and a pointer to the child.
    #[inline]
    pub const fn new(key: &'a Nibbles, child: &'a [u8]) -> Self {
        Self { key, child }
    }

    /// RLP-encodes the node and returns either `rlp(node)` or `rlp(keccak(rlp(node)))`.
    #[inline]
    pub fn rlp(&self, rlp: &mut Vec<u8>) -> RlpNode {
        self.encode(rlp);
        RlpNode::from_rlp(rlp)
    }

    /// Returns the length of RLP encoded fields of extension node.
    #[inline]
    fn rlp_payload_length(&self) -> usize {
        let mut encoded_key_len = self.key.len() / 2 + 1;
        // For extension nodes the first byte cannot be greater than 0x80.
        if encoded_key_len != 1 {
            encoded_key_len += length_of_length(encoded_key_len);
        }
        encoded_key_len + self.child.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlp_extension_node_roundtrip() {
        let nibble = Nibbles::from_nibbles_unchecked(hex!("0604060f"));
        let val = hex!("76657262");
        let mut child = vec![];
        val.to_vec().as_slice().encode(&mut child);
        let extension = ExtensionNode::new(nibble, RlpNode::from_raw(&child).unwrap());
        let rlp = extension.as_ref().rlp(&mut vec![]);
        assert_eq!(rlp.as_ref(), hex!("c98300646f8476657262"));
        assert_eq!(ExtensionNode::decode(&mut &rlp[..]).unwrap(), extension);
    }
}
