use super::{AddressIdentity, TraceIdentifier};
use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;
use foundry_common::contracts::{bytecode_diff_score, ContractsByArtifact};
use foundry_compilers::ArtifactId;
use std::borrow::Cow;

/// A trace identifier that tries to identify addresses using local contracts.
pub struct LocalTraceIdentifier<'a> {
    /// Known contracts to search through.
    known_contracts: &'a ContractsByArtifact,
    /// Vector of pairs of artifact ID and the runtime code length of the given artifact.
    ordered_ids: Vec<(&'a ArtifactId, usize)>,
}

impl<'a> LocalTraceIdentifier<'a> {
    /// Creates a new local trace identifier.
    #[inline]
    pub fn new(known_contracts: &'a ContractsByArtifact) -> Self {
        let mut ordered_ids = known_contracts
            .iter()
            .filter_map(|(id, contract)| Some((id, contract.deployed_bytecode()?)))
            .map(|(id, bytecode)| (id, bytecode.len()))
            .collect::<Vec<_>>();
        ordered_ids.sort_by_key(|(_, len)| *len);
        Self { known_contracts, ordered_ids }
    }

    /// Returns the known contracts.
    #[inline]
    pub fn contracts(&self) -> &'a ContractsByArtifact {
        self.known_contracts
    }

    /// Tries to the bytecode most similar to the given one.
    pub fn identify_code(&self, code: &[u8]) -> Option<(&'a ArtifactId, &'a JsonAbi)> {
        let len = code.len();

        let mut min_score = f64::MAX;
        let mut min_score_id = None;

        let mut check = |id| {
            let contract = self.known_contracts.get(id)?;
            if let Some(deployed_bytecode) = contract.deployed_bytecode() {
                let score = bytecode_diff_score(deployed_bytecode, code);
                if score == 0.0 {
                    trace!(target: "evm::traces", "found exact match");
                    return Some((id, &contract.abi));
                }
                if score < min_score {
                    min_score = score;
                    min_score_id = Some((id, &contract.abi));
                }
            }
            None
        };

        // Check `[len * 0.9, ..., len * 1.1]`.
        let max_len = (len * 11) / 10;

        // Start at artifacts with the same code length: `len..len*1.1`.
        let same_length_idx = self.find_index(len);
        for idx in same_length_idx..self.ordered_ids.len() {
            let (id, len) = self.ordered_ids[idx];
            if len > max_len {
                break;
            }
            if let found @ Some(_) = check(id) {
                return found;
            }
        }

        // Iterate over the remaining artifacts with less code length: `len*0.9..len`.
        let min_len = (len * 9) / 10;
        let idx = self.find_index(min_len);
        for i in idx..same_length_idx {
            let (id, _) = self.ordered_ids[i];
            if let found @ Some(_) = check(id) {
                return found;
            }
        }

        trace!(target: "evm::traces", %min_score, "no exact match found");

        // Note: the diff score can be inaccurate for small contracts so we're using a relatively
        // high threshold here to avoid filtering out too many contracts.
        if min_score < 0.85 {
            min_score_id
        } else {
            None
        }
    }

    /// Returns the index of the artifact with the given code length, or the index of the first
    /// artifact with a greater code length if the exact code length is not found.
    fn find_index(&self, len: usize) -> usize {
        let (Ok(mut idx) | Err(mut idx)) =
            self.ordered_ids.binary_search_by_key(&len, |(_, probe)| *probe);

        // In case of multiple artifacts with the same code length, we need to find the first one.
        while idx > 0 && self.ordered_ids[idx - 1].1 == len {
            idx -= 1;
        }

        idx
    }
}

impl TraceIdentifier for LocalTraceIdentifier<'_> {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        trace!(target: "evm::traces", "identify {:?} addresses", addresses.size_hint().1);

        addresses
            .filter_map(|(address, code)| {
                let _span = trace_span!(target: "evm::traces", "identify", %address).entered();

                trace!(target: "evm::traces", "identifying");
                let (id, abi) = self.identify_code(code?)?;
                trace!(target: "evm::traces", id=%id.identifier(), "identified");

                Some(AddressIdentity {
                    address: *address,
                    contract: Some(id.identifier()),
                    label: Some(id.name.clone()),
                    abi: Some(Cow::Borrowed(abi)),
                    artifact_id: Some(id.clone()),
                })
            })
            .collect()
    }
}
