use super::{AddressIdentity, TraceIdentifier};
use alloy_json_abi::{Event, Function};
use alloy_primitives::Address;
use foundry_common::contracts::{diff_score, ContractsByArtifact};
use foundry_compilers::ArtifactId;
use std::borrow::Cow;

/// A trace identifier that tries to identify addresses using local contracts.
pub struct LocalTraceIdentifier<'a> {
    known_contracts: &'a ContractsByArtifact,
    // Vector of pairs of artifact id and the code length of the given artifact.
    // Stored in descending order of code length to optimize the search.
    ordered_ids: Vec<(&'a ArtifactId, usize)>,
}

impl<'a> LocalTraceIdentifier<'a> {
    pub fn new(known_contracts: &'a ContractsByArtifact) -> Self {
        let mut ordered_ids =
            known_contracts.iter().map(|(id, contract)| (id, contract.1.len())).collect::<Vec<_>>();
        ordered_ids.sort_by_key(|(_, len)| *len);
        ordered_ids.reverse();

        Self { known_contracts, ordered_ids }
    }

    /// Get all the functions of the local contracts.
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.known_contracts.iter().flat_map(|(_, (abi, _))| abi.functions())
    }

    /// Get all the events of the local contracts.
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.known_contracts.iter().flat_map(|(_, (abi, _))| abi.events())
    }

    /// Iterates over artifacts with code length less than or equal to the given code and tries to
    /// find a match.
    ///
    /// We do not consider artifacts with code length greater than the given code length as it is
    /// considered that after compilation code can only be extended by additional parameters
    /// (immutables) and cannot be shortened.
    pub fn identify_code(&'a self, code: &[u8]) -> Option<&'a ArtifactId> {
        let ids = self
            .ordered_ids
            .iter()
            .filter(|(_, known_code_len)| code.len() >= *known_code_len)
            .map(|(id, _)| *id);

        let mut min_score = 1.0;
        let mut min_score_id = None;
        for id in ids {
            let (_, known_code) = self.known_contracts.get(id)?;
            let score = diff_score(code, known_code);
            if score < 0.1 {
                return Some(id);
            }
            if score < min_score {
                min_score = score;
                min_score_id = Some(id);
            }
        }

        min_score_id
    }
}

impl TraceIdentifier for LocalTraceIdentifier<'_> {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        addresses
            .filter_map(|(address, code)| {
                let id = self.identify_code(code?)?;
                let (abi, _) = self.known_contracts.get(id)?;

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
