use super::{AddressIdentity, TraceIdentifier};
use alloy_json_abi::Event;
use alloy_primitives::Address;
use foundry_common::contracts::{diff_score, ContractsByArtifact};
use ordered_float::OrderedFloat;
use std::borrow::Cow;

/// A trace identifier that tries to identify addresses using local contracts.
pub struct LocalTraceIdentifier<'a> {
    known_contracts: &'a ContractsByArtifact,
}

impl<'a> LocalTraceIdentifier<'a> {
    pub fn new(known_contracts: &'a ContractsByArtifact) -> Self {
        Self { known_contracts }
    }

    /// Get all the events of the local contracts.
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.known_contracts.iter().flat_map(|(_, (abi, _))| abi.events())
    }
}

impl TraceIdentifier for LocalTraceIdentifier<'_> {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        addresses
            .filter_map(|(address, code)| {
                let code = code?;
                let (_, id, abi) = self
                    .known_contracts
                    .iter()
                    .filter_map(|(id, (abi, known_code))| {
                        let score = diff_score(known_code, code);
                        // Note: the diff score can be inaccurate for small contracts so we're using
                        // a relatively high threshold here to avoid filtering out too many
                        // contracts.
                        if score < 0.85 {
                            Some((OrderedFloat(score), id, abi))
                        } else {
                            None
                        }
                    })
                    .min_by_key(|(score, _, _)| *score)?;

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
