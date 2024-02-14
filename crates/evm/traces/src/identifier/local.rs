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
    /// Vector of pairs of artifact ID and the code length of the given artifact.
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

        // Check `[len * 0.9, ..., len * 1.1]`.
        let min_len = (code.len() * 9) / 10;
        let max_len = (code.len() * 11) / 10;

        for &(id, _) in self.artifact_ids(min_len) {
            let (abi, known_code) = self.known_contracts.get(id)?;
            if known_code.len() > max_len {
                break;
            }
            let score = bytecode_diff_score(known_code, code);
            if score <= 0.1 {
                trace!(%score, "found close-enough match");
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

    /// Returns the IDs of the artifacts with code length greater than or equal to the given length.
    fn artifact_ids(&self, len: usize) -> &[(&'a ArtifactId, usize)] {
        let (Ok(mut start) | Err(mut start)) =
            self.ordered_ids.binary_search_by(|(_, probe)| probe.cmp(&len));
        // In case of multiple artifacts with the same code length, we need to find the first one.
        while start > 0 && self.ordered_ids[start - 1].1 == len {
            start -= 1;
        }
        &self.ordered_ids[start..]
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
