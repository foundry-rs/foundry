//! Various branch nodes produced by the hash builder.

use alloy_primitives::{Bytes, B256};
use alloy_rlp::{Decodable, Encodable, Header, EMPTY_STRING_CODE};
use core::ops::Range;
use nybbles::Nibbles;
use smallvec::SmallVec;

#[allow(unused_imports)]
use alloc::vec::Vec;

mod branch;
pub use branch::{BranchNode, BranchNodeCompact, BranchNodeRef};

mod extension;
pub use extension::{ExtensionNode, ExtensionNodeRef};

mod leaf;
pub use leaf::{LeafNode, LeafNodeRef};

mod rlp;
pub use rlp::RlpNode;

/// The range of valid child indexes.
pub const CHILD_INDEX_RANGE: Range<u8> = 0..16;

/// Enum representing an MPT trie node.
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum TrieNode {
    /// Variant representing empty root node.
    EmptyRoot,
    /// Variant representing a [BranchNode].
    Branch(BranchNode),
    /// Variant representing a [ExtensionNode].
    Extension(ExtensionNode),
    /// Variant representing a [LeafNode].
    Leaf(LeafNode),
}

impl Encodable for TrieNode {
    #[inline]
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        match self {
            Self::EmptyRoot => {
                out.put_u8(EMPTY_STRING_CODE);
            }
            Self::Branch(branch) => branch.encode(out),
            Self::Extension(extension) => extension.encode(out),
            Self::Leaf(leaf) => leaf.encode(out),
        }
    }

    #[inline]
    fn length(&self) -> usize {
        match self {
            Self::EmptyRoot => 1,
            Self::Branch(branch) => branch.length(),
            Self::Extension(extension) => extension.length(),
            Self::Leaf(leaf) => leaf.length(),
        }
    }
}

impl Decodable for TrieNode {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let mut items = match Header::decode_raw(buf)? {
            alloy_rlp::PayloadView::List(list) => list,
            alloy_rlp::PayloadView::String(val) => {
                return if val.is_empty() {
                    Ok(Self::EmptyRoot)
                } else {
                    Err(alloy_rlp::Error::UnexpectedString)
                }
            }
        };

        // A valid number of trie node items is either 17 (branch node)
        // or 2 (extension or leaf node).
        match items.len() {
            17 => {
                let mut branch = BranchNode::default();
                for (idx, item) in items.into_iter().enumerate() {
                    if idx == 16 {
                        if item != [EMPTY_STRING_CODE] {
                            return Err(alloy_rlp::Error::Custom(
                                "branch node values are not supported",
                            ));
                        }
                    } else if item != [EMPTY_STRING_CODE] {
                        branch.stack.push(RlpNode::from_raw_rlp(item)?);
                        branch.state_mask.set_bit(idx as u8);
                    }
                }
                Ok(Self::Branch(branch))
            }
            2 => {
                let mut key = items.remove(0);

                let encoded_key = Header::decode_bytes(&mut key, false)?;
                if encoded_key.is_empty() {
                    return Err(alloy_rlp::Error::Custom("trie node key empty"));
                }

                // extract the high order part of the nibble to then pick the odd nibble out
                let key_flag = encoded_key[0] & 0xf0;
                // Retrieve first byte. If it's [Some], then the nibbles are odd.
                let first = match key_flag {
                    ExtensionNode::ODD_FLAG | LeafNode::ODD_FLAG => Some(encoded_key[0] & 0x0f),
                    ExtensionNode::EVEN_FLAG | LeafNode::EVEN_FLAG => None,
                    _ => return Err(alloy_rlp::Error::Custom("node is not extension or leaf")),
                };

                let key = unpack_path_to_nibbles(first, &encoded_key[1..]);
                let node = if key_flag == LeafNode::EVEN_FLAG || key_flag == LeafNode::ODD_FLAG {
                    let value = Bytes::decode(&mut items.remove(0))?.into();
                    Self::Leaf(LeafNode::new(key, value))
                } else {
                    // We don't decode value because it is expected to be RLP encoded.
                    Self::Extension(ExtensionNode::new(
                        key,
                        RlpNode::from_raw_rlp(items.remove(0))?,
                    ))
                };
                Ok(node)
            }
            _ => Err(alloy_rlp::Error::Custom("invalid number of items in the list")),
        }
    }
}

impl TrieNode {
    /// RLP-encodes the node and returns either `rlp(node)` or `rlp(keccak(rlp(node)))`.
    #[inline]
    pub fn rlp(&self, rlp: &mut Vec<u8>) -> RlpNode {
        self.encode(rlp);
        RlpNode::from_rlp(rlp)
    }
}

/// Given an RLP-encoded node, returns it either as `rlp(node)` or `rlp(keccak(rlp(node)))`.
#[inline]
#[deprecated = "use `RlpNode::from_rlp` instead"]
pub fn rlp_node(rlp: &[u8]) -> RlpNode {
    RlpNode::from_rlp(rlp)
}

/// Optimization for quick RLP-encoding of a 32-byte word.
#[inline]
#[deprecated = "use `RlpNode::word_rlp` instead"]
pub fn word_rlp(word: &B256) -> RlpNode {
    RlpNode::word_rlp(word)
}

/// Unpack node path to nibbles.
///
/// NOTE: The first nibble should be less than or equal to `0xf` if provided.
/// If first nibble is greater than `0xf`, the method will not panic, but initialize invalid nibbles
/// instead.
///
/// ## Arguments
///
/// `first` - first nibble of the path if it is odd
/// `rest` - rest of the nibbles packed
#[inline]
pub(crate) fn unpack_path_to_nibbles(first: Option<u8>, rest: &[u8]) -> Nibbles {
    let Some(first) = first else { return Nibbles::unpack(rest) };
    debug_assert!(first <= 0xf);
    let len = rest.len() * 2 + 1;
    // SAFETY: `len` is calculated correctly.
    unsafe {
        Nibbles::from_repr_unchecked(nybbles::smallvec_with(len, |buf| {
            let (f, r) = buf.split_first_mut().unwrap_unchecked();
            f.write(first);
            Nibbles::unpack_to_unchecked(rest, r);
        }))
    }
}

/// Encodes a given path leaf as a compact array of bytes.
///
/// In resulted array, each byte represents two "nibbles" (half-bytes or 4 bits) of the original hex
/// data, along with additional information about the leaf itself.
///
/// The method takes the following input:
/// `is_leaf`: A boolean value indicating whether the current node is a leaf node or not.
///
/// The first byte of the encoded vector is set based on the `is_leaf` flag and the parity of
/// the hex data length (even or odd number of nibbles).
///  - If the node is an extension with even length, the header byte is `0x00`.
///  - If the node is an extension with odd length, the header byte is `0x10 + <first nibble>`.
///  - If the node is a leaf with even length, the header byte is `0x20`.
///  - If the node is a leaf with odd length, the header byte is `0x30 + <first nibble>`.
///
/// If there is an odd number of nibbles, store the first nibble in the lower 4 bits of the
/// first byte of encoded.
///
/// # Returns
///
/// A vector containing the compact byte representation of the nibble sequence, including the
/// header byte.
///
/// This vector's length is `self.len() / 2 + 1`. For stack-allocated nibbles, this is at most
/// 33 bytes, so 36 was chosen as the stack capacity to round up to the next usize-aligned
/// size.
///
/// # Examples
///
/// ```
/// use alloy_trie::nodes::encode_path_leaf;
/// use nybbles::Nibbles;
///
/// // Extension node with an even path length:
/// let nibbles = Nibbles::from_nibbles(&[0x0A, 0x0B, 0x0C, 0x0D]);
/// assert_eq!(encode_path_leaf(&nibbles, false)[..], [0x00, 0xAB, 0xCD]);
///
/// // Extension node with an odd path length:
/// let nibbles = Nibbles::from_nibbles(&[0x0A, 0x0B, 0x0C]);
/// assert_eq!(encode_path_leaf(&nibbles, false)[..], [0x1A, 0xBC]);
///
/// // Leaf node with an even path length:
/// let nibbles = Nibbles::from_nibbles(&[0x0A, 0x0B, 0x0C, 0x0D]);
/// assert_eq!(encode_path_leaf(&nibbles, true)[..], [0x20, 0xAB, 0xCD]);
///
/// // Leaf node with an odd path length:
/// let nibbles = Nibbles::from_nibbles(&[0x0A, 0x0B, 0x0C]);
/// assert_eq!(encode_path_leaf(&nibbles, true)[..], [0x3A, 0xBC]);
/// ```
#[inline]
pub fn encode_path_leaf(nibbles: &Nibbles, is_leaf: bool) -> SmallVec<[u8; 36]> {
    let mut nibbles = nibbles.as_slice();
    let encoded_len = nibbles.len() / 2 + 1;
    let odd_nibbles = nibbles.len() % 2 != 0;
    // SAFETY: `len` is calculated correctly.
    unsafe {
        nybbles::smallvec_with(encoded_len, |buf| {
            let (first, rest) = buf.split_first_mut().unwrap_unchecked();
            first.write(match (is_leaf, odd_nibbles) {
                (true, true) => LeafNode::ODD_FLAG | *nibbles.get_unchecked(0),
                (true, false) => LeafNode::EVEN_FLAG,
                (false, true) => ExtensionNode::ODD_FLAG | *nibbles.get_unchecked(0),
                (false, false) => ExtensionNode::EVEN_FLAG,
            });
            if odd_nibbles {
                nibbles = nibbles.get_unchecked(1..);
            }
            nybbles::pack_to_unchecked(nibbles, rest);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrieMask;
    use alloy_primitives::hex;

    #[test]
    fn rlp_empty_root_node() {
        let empty_root = TrieNode::EmptyRoot;
        let rlp = empty_root.rlp(&mut vec![]);
        assert_eq!(rlp[..], hex!("80"));
        assert_eq!(TrieNode::decode(&mut &rlp[..]).unwrap(), empty_root);
    }

    #[test]
    fn rlp_zero_value_leaf_roundtrip() {
        let leaf = TrieNode::Leaf(LeafNode::new(
            Nibbles::from_nibbles_unchecked(hex!("0604060f")),
            alloy_rlp::encode(alloy_primitives::U256::ZERO),
        ));
        let rlp = leaf.rlp(&mut vec![]);
        assert_eq!(rlp[..], hex!("c68320646f8180"));
        assert_eq!(TrieNode::decode(&mut &rlp[..]).unwrap(), leaf);
    }

    #[test]
    fn rlp_trie_node_roundtrip() {
        // leaf
        let leaf = TrieNode::Leaf(LeafNode::new(
            Nibbles::from_nibbles_unchecked(hex!("0604060f")),
            hex!("76657262").to_vec(),
        ));
        let rlp = leaf.rlp(&mut vec![]);
        assert_eq!(rlp[..], hex!("c98320646f8476657262"));
        assert_eq!(TrieNode::decode(&mut &rlp[..]).unwrap(), leaf);

        // extension
        let mut child = vec![];
        hex!("76657262").to_vec().as_slice().encode(&mut child);
        let extension = TrieNode::Extension(ExtensionNode::new(
            Nibbles::from_nibbles_unchecked(hex!("0604060f")),
            RlpNode::from_raw(&child).unwrap(),
        ));
        let rlp = extension.rlp(&mut vec![]);
        assert_eq!(rlp[..], hex!("c98300646f8476657262"));
        assert_eq!(TrieNode::decode(&mut &rlp[..]).unwrap(), extension);

        // branch
        let branch = TrieNode::Branch(BranchNode::new(
            core::iter::repeat(RlpNode::word_rlp(&B256::repeat_byte(23))).take(16).collect(),
            TrieMask::new(u16::MAX),
        ));
        let mut rlp = vec![];
        let rlp_node = branch.rlp(&mut rlp);
        assert_eq!(
            rlp_node[..],
            hex!("a0bed74980bbe29d9c4439c10e9c451e29b306fe74bcf9795ecf0ebbd92a220513")
        );
        assert_eq!(rlp, hex!("f90211a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a01717171717171717171717171717171717171717171717171717171717171717a0171717171717171717171717171717171717171717171717171717171717171780"));
        assert_eq!(TrieNode::decode(&mut &rlp[..]).unwrap(), branch);
    }

    #[test]
    fn hashed_encode_path_regression() {
        let nibbles = Nibbles::from_nibbles(hex!("05010406040a040203030f010805020b050c04070003070e0909070f010b0a0805020301070c0a0902040b0f000f0006040a04050f020b090701000a0a040b"));
        let path = encode_path_leaf(&nibbles, true);
        let expected = hex!("351464a4233f1852b5c47037e997f1ba852317ca924bf0f064a45f2b9710aa4b");
        assert_eq!(path[..], expected);
    }

    #[test]
    #[cfg(feature = "arbitrary")]
    #[cfg_attr(miri, ignore = "no proptest")]
    fn encode_path_first_byte() {
        use proptest::{collection::vec, prelude::*};

        proptest::proptest!(|(input in vec(any::<u8>(), 0..128))| {
            let input = Nibbles::unpack(input);
            prop_assert!(input.iter().all(|&nibble| nibble <= 0xf));
            let input_is_odd = input.len() % 2 == 1;

            let compact_leaf = encode_path_leaf(&input, true);
            let leaf_flag = compact_leaf[0];
            // Check flag
            assert_ne!(leaf_flag & LeafNode::EVEN_FLAG, 0);
            assert_eq!(input_is_odd, (leaf_flag & ExtensionNode::ODD_FLAG) != 0);
            if input_is_odd {
                assert_eq!(leaf_flag & 0x0f, input.first().unwrap());
            }

            let compact_extension = encode_path_leaf(&input, false);
            let extension_flag = compact_extension[0];
            // Check first byte
            assert_eq!(extension_flag & LeafNode::EVEN_FLAG, 0);
            assert_eq!(input_is_odd, (extension_flag & ExtensionNode::ODD_FLAG) != 0);
            if input_is_odd {
                assert_eq!(extension_flag & 0x0f, input.first().unwrap());
            }
        });
    }
}
