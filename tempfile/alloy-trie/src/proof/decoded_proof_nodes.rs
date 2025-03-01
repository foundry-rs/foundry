use crate::{nodes::TrieNode, proof::ProofNodes, HashMap, Nibbles};
use alloy_primitives::Bytes;
use alloy_rlp::Decodable;
use core::ops::Deref;

use alloc::vec::Vec;

/// A wrapper struct for trie node key to RLP encoded trie node.
#[derive(PartialEq, Eq, Clone, Default, Debug)]
pub struct DecodedProofNodes(HashMap<Nibbles, TrieNode>);

impl Deref for DecodedProofNodes {
    type Target = HashMap<Nibbles, TrieNode>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromIterator<(Nibbles, TrieNode)> for DecodedProofNodes {
    fn from_iter<T: IntoIterator<Item = (Nibbles, TrieNode)>>(iter: T) -> Self {
        Self(HashMap::from_iter(iter))
    }
}

impl Extend<(Nibbles, TrieNode)> for DecodedProofNodes {
    fn extend<T: IntoIterator<Item = (Nibbles, TrieNode)>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl TryFrom<ProofNodes> for DecodedProofNodes {
    type Error = alloy_rlp::Error;

    fn try_from(proof_nodes: ProofNodes) -> Result<Self, Self::Error> {
        let mut decoded_proof_nodes =
            HashMap::with_capacity_and_hasher(proof_nodes.len(), Default::default());
        for (key, node) in proof_nodes.into_inner() {
            decoded_proof_nodes.insert(key, TrieNode::decode(&mut &node[..])?);
        }
        Ok(Self(decoded_proof_nodes))
    }
}

impl DecodedProofNodes {
    /// Return iterator over proof nodes that match the target.
    pub fn matching_nodes_iter<'a>(
        &'a self,
        target: &'a Nibbles,
    ) -> impl Iterator<Item = (&'a Nibbles, &'a TrieNode)> {
        self.0.iter().filter(|(key, _)| target.starts_with(key))
    }

    /// Return the vec of proof nodes that match the target.
    pub fn matching_nodes(&self, target: &Nibbles) -> Vec<(Nibbles, TrieNode)> {
        self.matching_nodes_iter(target).map(|(key, node)| (key.clone(), node.clone())).collect()
    }

    /// Return the sorted vec of proof nodes that match the target.
    pub fn matching_nodes_sorted(&self, target: &Nibbles) -> Vec<(Nibbles, TrieNode)> {
        let mut nodes = self.matching_nodes(target);
        nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    /// Insert the trie node at key.
    pub fn insert(&mut self, key: Nibbles, node: TrieNode) -> Option<TrieNode> {
        self.0.insert(key, node)
    }

    /// Insert the RLP encoded trie nodoe at key
    pub fn insert_encoded(
        &mut self,
        key: Nibbles,
        node: Bytes,
    ) -> Result<Option<TrieNode>, alloy_rlp::Error> {
        Ok(self.0.insert(key, TrieNode::decode(&mut &node[..])?))
    }

    /// Return the sorted vec of all proof nodes.
    pub fn nodes_sorted(&self) -> Vec<(Nibbles, TrieNode)> {
        let mut nodes = Vec::from_iter(self.0.iter().map(|(k, v)| (k.clone(), v.clone())));
        nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    /// Convert into sorted vec of all proof nodes.
    pub fn into_nodes_sorted(self) -> Vec<(Nibbles, TrieNode)> {
        let mut nodes = Vec::from_iter(self.0);
        nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    /// Convert wrapper struct into inner map.
    pub fn into_inner(self) -> HashMap<Nibbles, TrieNode> {
        self.0
    }

    /// Extends with the elements of another `DecodedProofNodes`.
    pub fn extend_from(&mut self, other: Self) {
        self.extend(other.0);
    }
}
