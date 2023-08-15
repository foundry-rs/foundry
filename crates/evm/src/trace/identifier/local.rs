use super::{AddressIdentity, TraceIdentifier};
use ethers::{
    abi::{Abi, Address, Event},
    prelude::ArtifactId,
};
use foundry_common::contracts::{diff_score, ContractsByArtifact};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use std::{borrow::Cow, collections::BTreeMap};

/// A trace identifier that tries to identify addresses using local contracts.
pub struct LocalTraceIdentifier {
    local_contracts: BTreeMap<Vec<u8>, (ArtifactId, Abi)>,
}

impl LocalTraceIdentifier {
    pub fn new(known_contracts: &ContractsByArtifact) -> Self {
        Self {
            local_contracts: known_contracts
                .iter()
                .map(|(id, (abi, runtime_code))| (runtime_code.clone(), (id.clone(), abi.clone())))
                .collect(),
        }
    }

    /// Get all the events of the local contracts.
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.local_contracts.iter().flat_map(|(_, (_, abi))| abi.events())
    }
}

impl TraceIdentifier for LocalTraceIdentifier {
    fn identify_addresses(
        &mut self,
        addresses: Vec<(&Address, Option<&[u8]>)>,
    ) -> Vec<AddressIdentity> {
        addresses
            .into_iter()
            .filter_map(|(address, code)| {
                let code = code?;
                let (_, (_, (id, abi))) = self
                    .local_contracts
                    .iter()
                    .filter_map(|entry| {
                        let score = diff_score(entry.0, code);
                        if score < 0.1 {
                            Some((OrderedFloat(score), entry))
                        } else {
                            None
                        }
                    })
                    .sorted_by_key(|(score, _)| *score)
                    .next()?;

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
