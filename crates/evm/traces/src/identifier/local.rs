use super::{AddressIdentity, TraceIdentifier};
use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;
use foundry_common::contracts::{bytecode_diff_score, ContractsByArtifact};
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
    /// Creates a new local trace identifier.
    #[inline]
    pub fn new(known_contracts: &'a ContractsByArtifact) -> Self {
        let mut ordered_ids =
            known_contracts.iter().map(|(id, contract)| (id, contract.1.len())).collect::<Vec<_>>();
        ordered_ids.sort_by_key(|(_, len)| *len);
        Self { known_contracts, ordered_ids }
    }

    /// Returns the known contracts.
    #[inline]
    pub fn contracts(&self) -> &'a ContractsByArtifact {
        self.known_contracts
    }

    /// Iterates over artifacts with code length less than or equal to the given code and tries to
    /// find a match.
    ///
    /// We do not consider artifacts with code length greater than the given code length as it is
    /// considered that after compilation code can only be extended by additional parameters
    /// (immutables) and cannot be shortened.
    pub fn identify_code(&self, code: &[u8]) -> Option<(&'a ArtifactId, &'a JsonAbi)> {
        let mut min_score = f64::MAX;
        let mut min_score_id = None;

        let ids_start = match self
            .ordered_ids
            .binary_search_by(|(_, known_code_len)| known_code_len.cmp(&code.len()))
        {
            // Exact match.
            Ok(i) => i,
            // Not found, start searching from the previous index.
            Err(i) => i.saturating_sub(1),
        };
        for &(id, _) in &self.ordered_ids[ids_start..] {
            let (abi, known_code) = self.known_contracts.get(id)?;
            let score = bytecode_diff_score(known_code, code);
            trace!(%score, abi=?abi.functions().collect::<Vec<_>>());
            if score < 0.1 {
                return Some((id, abi));
            }
            if score < min_score {
                min_score = score;
                min_score_id = Some((id, abi));
            }
        }

        trace!(%min_score, "no close-enough match found");
        min_score_id
    }
}

impl TraceIdentifier for LocalTraceIdentifier<'_> {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        trace!("identify {:?} addresses", addresses.size_hint().1);

        addresses
            .filter_map(|(address, code)| {
                let _span = trace_span!("identify", %address).entered();

                trace!("identifying");
                let (id, abi) = self.identify_code(code?)?;
                trace!(id=%id.identifier(), "identified");

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
