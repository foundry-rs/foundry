use crate::{proof::ProofNodes, Nibbles};
use alloy_primitives::Bytes;

#[allow(unused_imports)]
use alloc::vec::Vec;

/// Proof retainer is used to store proofs during merkle trie construction.
/// It is intended to be used within the [`HashBuilder`](crate::HashBuilder).
#[derive(Default, Clone, Debug)]
pub struct ProofRetainer {
    /// The nibbles of the target trie keys to retain proofs for.
    targets: Vec<Nibbles>,
    /// The map retained trie node keys to RLP serialized trie nodes.
    proof_nodes: ProofNodes,
}

impl FromIterator<Nibbles> for ProofRetainer {
    fn from_iter<T: IntoIterator<Item = Nibbles>>(iter: T) -> Self {
        Self::new(FromIterator::from_iter(iter))
    }
}

impl ProofRetainer {
    /// Create new retainer with target nibbles.
    pub fn new(targets: Vec<Nibbles>) -> Self {
        Self { targets, proof_nodes: Default::default() }
    }

    /// Returns `true` if the given prefix matches the retainer target.
    pub fn matches(&self, prefix: &Nibbles) -> bool {
        self.targets.iter().any(|target| target.starts_with(prefix))
    }

    /// Returns all collected proofs.
    pub fn into_proof_nodes(self) -> ProofNodes {
        self.proof_nodes
    }

    /// Retain the proof if the key matches any of the targets.
    pub fn retain(&mut self, prefix: &Nibbles, proof: &[u8]) {
        if prefix.is_empty() || self.matches(prefix) {
            self.proof_nodes.insert(prefix.clone(), Bytes::from(proof.to_vec()));
        }
    }
}
