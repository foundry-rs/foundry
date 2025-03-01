use crate::{HashMap, Nibbles};
use alloy_primitives::Bytes;
use core::ops::Deref;

use alloc::vec::Vec;

/// A wrapper struct for trie node key to RLP encoded trie node.
#[derive(PartialEq, Eq, Clone, Default, Debug)]
pub struct ProofNodes(HashMap<Nibbles, Bytes>);

impl Deref for ProofNodes {
    type Target = HashMap<Nibbles, Bytes>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromIterator<(Nibbles, Bytes)> for ProofNodes {
    fn from_iter<T: IntoIterator<Item = (Nibbles, Bytes)>>(iter: T) -> Self {
        Self(HashMap::from_iter(iter))
    }
}

impl Extend<(Nibbles, Bytes)> for ProofNodes {
    fn extend<T: IntoIterator<Item = (Nibbles, Bytes)>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl ProofNodes {
    /// Return iterator over proof nodes that match the target.
    pub fn matching_nodes_iter<'a>(
        &'a self,
        target: &'a Nibbles,
    ) -> impl Iterator<Item = (&'a Nibbles, &'a Bytes)> {
        self.0.iter().filter(|(key, _)| target.starts_with(key))
    }

    /// Return the vec of proof nodes that match the target.
    pub fn matching_nodes(&self, target: &Nibbles) -> Vec<(Nibbles, Bytes)> {
        self.matching_nodes_iter(target).map(|(key, node)| (key.clone(), node.clone())).collect()
    }

    /// Return the sorted vec of proof nodes that match the target.
    pub fn matching_nodes_sorted(&self, target: &Nibbles) -> Vec<(Nibbles, Bytes)> {
        let mut nodes = self.matching_nodes(target);
        nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    /// Insert the RLP encoded trie node at key.
    pub fn insert(&mut self, key: Nibbles, node: Bytes) -> Option<Bytes> {
        self.0.insert(key, node)
    }

    /// Return the sorted vec of all proof nodes.
    pub fn nodes_sorted(&self) -> Vec<(Nibbles, Bytes)> {
        let mut nodes = Vec::from_iter(self.0.iter().map(|(k, v)| (k.clone(), v.clone())));
        nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    /// Convert into sorted vec of all proof nodes.
    pub fn into_nodes_sorted(self) -> Vec<(Nibbles, Bytes)> {
        let mut nodes = Vec::from_iter(self.0);
        nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        nodes
    }

    /// Convert wrapper struct into inner map.
    pub fn into_inner(self) -> HashMap<Nibbles, Bytes> {
        self.0
    }

    /// Extends with the elements of another `ProofNodes`.
    pub fn extend_from(&mut self, other: Self) {
        self.extend(other.0);
    }
}
