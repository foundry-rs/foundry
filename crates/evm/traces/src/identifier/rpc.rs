use super::{AddressIdentity, TraceIdentifier};
use alloy_primitives::Address;
use ethers_core::types::H160;
use ethers_providers::Middleware;
use foundry_common::contracts::{diff_score, ContractsByArtifact};
use foundry_compilers::utils::RuntimeOrHandle;
use ordered_float::OrderedFloat;
use std::borrow::Cow;

/// A trace identifier that tries to identify addresses using RPC contracts.
pub struct RPCTraceIdentifier<'a> {
    known_contracts: &'a ContractsByArtifact,
    provider: &'a ethers_providers::Provider<foundry_common::runtime_client::RuntimeClient>,
}

impl<'a> RPCTraceIdentifier<'a> {
    pub fn new(known_contracts: &'a ContractsByArtifact, provider: &'a ethers_providers::Provider<foundry_common::runtime_client::RuntimeClient>) -> Self {
        Self { known_contracts, provider}
    }
}

impl TraceIdentifier for RPCTraceIdentifier<'_> {

    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        let provider = self.provider;

        addresses
            .filter_map(|(address, _)| {
                let code = RuntimeOrHandle::new().block_on(provider.get_code(Into::<H160>::into(address.into_array()), None)).expect("");
                let (_, id, abi) = self
                    .known_contracts
                    .iter()
                    .filter_map(|(id, (abi, known_code))| {
                        let score = diff_score(known_code, code.0.as_ref());
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
