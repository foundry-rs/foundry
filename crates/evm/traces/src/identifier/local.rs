use super::{AddressIdentity, TraceIdentifier};
use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;
use foundry_common::contracts::{bytecode_diff_score, ContractsByArtifact};
use foundry_compilers::ArtifactId;
use ordered_float::OrderedFloat;
use std::{borrow::Cow, collections::HashSet};

/// A trace identifier that tries to identify addresses using local contracts.
pub struct LocalTraceIdentifier<'a> {
    known_contracts: &'a ContractsByArtifact,
    already_found: HashSet<Address>,
}

impl<'a> LocalTraceIdentifier<'a> {
    /// Creates a new local trace identifier.
    #[inline]
    pub fn new(known_contracts: &'a ContractsByArtifact) -> Self {
        Self { known_contracts, already_found: HashSet::new() }
    }

    /// Returns the known contracts.
    #[inline]
    pub fn contracts(&self) -> &'a ContractsByArtifact {
        self.known_contracts
    }

    fn find_contract_from_bytecode(
        &mut self,
        address: &Address,
        code: Option<&[u8]>,
    ) -> Option<(&'a ArtifactId, &'a JsonAbi)> {
        // Only provide the contract for the first time we see it.
        let code = code?;
        if self.already_found.contains(address) {
            return None;
        }

        let (id, abi) = self.find_contract_from_bytecode_uncached(code)?;
        self.already_found.insert(*address);
        Some((id, abi))
    }

    fn find_contract_from_bytecode_uncached(
        &mut self,
        code: &[u8],
    ) -> Option<(&'a ArtifactId, &'a JsonAbi)> {
        self.known_contracts
            .iter()
            .filter_map(|(id, (abi, known_code))| {
                // Note: the diff score can be inaccurate for small contracts so we're using
                // a relatively high threshold here to avoid filtering out too many
                // contracts.
                let score = bytecode_diff_score(known_code, code);
                (score < 0.85).then(|| (score, id, abi))
            })
            .min_by_key(|(score, _, _)| OrderedFloat(*score))
            .map(|(_, id, abi)| (id, abi))
    }
}

impl TraceIdentifier for LocalTraceIdentifier<'_> {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        addresses
            .filter_map(|(address, code)| {
                let (id, abi) = self.find_contract_from_bytecode(address, code)?;
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
