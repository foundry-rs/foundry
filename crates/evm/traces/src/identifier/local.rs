use super::{IdentifiedAddress, TraceIdentifier};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, Bytes, map::HashMap};
use foundry_common::contracts::{ContractsByArtifact, bytecode_diff_score};
use foundry_compilers::ArtifactId;
use revm_inspectors::tracing::types::CallTraceNode;
use std::borrow::Cow;

/// A trace identifier that tries to identify addresses using local contracts.
pub struct LocalTraceIdentifier<'a> {
    /// Known contracts to search through.
    known_contracts: &'a ContractsByArtifact,
    /// Vector of pairs of artifact ID and the runtime code length of the given artifact.
    ordered_ids: Vec<(&'a ArtifactId, usize)>,
    /// The contracts bytecode.
    contracts_bytecode: Option<&'a HashMap<Address, Bytes>>,
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
        Self { known_contracts, ordered_ids, contracts_bytecode: None }
    }

    pub fn with_bytecodes(mut self, contracts_bytecode: &'a HashMap<Address, Bytes>) -> Self {
        self.contracts_bytecode = Some(contracts_bytecode);
        self
    }

    /// Returns the known contracts.
    #[inline]
    pub fn contracts(&self) -> &'a ContractsByArtifact {
        self.known_contracts
    }

    /// Identifies the artifact based on score computed for both creation and deployed bytecodes.
    pub fn identify_code(
        &self,
        runtime_code: &[u8],
        creation_code: &[u8],
    ) -> Option<(&'a ArtifactId, &'a JsonAbi)> {
        let len = runtime_code.len();

        let mut min_score = f64::MAX;
        let mut min_score_id = None;

        let mut check = |id, is_creation, min_score: &mut f64| {
            let contract = self.known_contracts.get(id)?;
            // Select bytecodes to compare based on `is_creation` flag.
            let (contract_bytecode, current_bytecode) = if is_creation {
                (contract.bytecode_without_placeholders(), creation_code)
            } else {
                (contract.deployed_bytecode_without_placeholders(), runtime_code)
            };

            if let Some(bytecode) = contract_bytecode {
                let mut current_bytecode = current_bytecode;
                if is_creation && current_bytecode.len() > bytecode.len() {
                    // Try to decode ctor args with contract abi.
                    if let Some(constructor) = contract.abi.constructor() {
                        let constructor_args = &current_bytecode[bytecode.len()..];
                        if constructor.abi_decode_input(constructor_args).is_ok() {
                            // If we can decode args with current abi then remove args from
                            // code to compare.
                            current_bytecode = &current_bytecode[..bytecode.len()]
                        }
                    }
                }

                let score = bytecode_diff_score(&bytecode, current_bytecode);
                if score == 0.0 {
                    trace!(target: "evm::traces::local", "found exact match");
                    return Some((id, &contract.abi));
                }
                if score < *min_score {
                    *min_score = score;
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
            if let found @ Some(_) = check(id, true, &mut min_score) {
                return found;
            }
        }

        // Iterate over the remaining artifacts with less code length: `len*0.9..len`.
        let min_len = (len * 9) / 10;
        let idx = self.find_index(min_len);
        for i in idx..same_length_idx {
            let (id, _) = self.ordered_ids[i];
            if let found @ Some(_) = check(id, true, &mut min_score) {
                return found;
            }
        }

        // Fallback to comparing deployed code if min score greater than threshold.
        if min_score >= 0.85 {
            for (artifact, _) in &self.ordered_ids {
                if let found @ Some(_) = check(artifact, false, &mut min_score) {
                    return found;
                }
            }
        }

        trace!(target: "evm::traces::local", %min_score, "no exact match found");

        // Note: the diff score can be inaccurate for small contracts so we're using a relatively
        // high threshold here to avoid filtering out too many contracts.
        if min_score < 0.85 { min_score_id } else { None }
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
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        if nodes.is_empty() {
            return Vec::new();
        }

        trace!(target: "evm::traces::local", "identify {} addresses", nodes.len());

        nodes
            .iter()
            .map(|&node| {
                (
                    node.trace.address,
                    node.trace.kind.is_any_create().then_some(&node.trace.output[..]),
                    node.trace.kind.is_any_create().then_some(&node.trace.data[..]),
                )
            })
            .filter_map(|(address, runtime_code, creation_code)| {
                let _span =
                    trace_span!(target: "evm::traces::local", "identify", %address).entered();

                // In order to identify the addresses, we need at least the runtime code. It can be
                // obtained from the trace itself (if it's a CREATE* call), or from the fetched
                // bytecodes.
                let (runtime_code, creation_code) = match (runtime_code, creation_code) {
                    (Some(runtime_code), Some(creation_code)) => (runtime_code, creation_code),
                    (Some(runtime_code), _) => (runtime_code, &[] as &[u8]),
                    _ => {
                        let code = self.contracts_bytecode?.get(&address)?;
                        (code.as_ref(), &[] as &[u8])
                    }
                };
                let (id, abi) = self.identify_code(runtime_code, creation_code)?;
                trace!(target: "evm::traces::local", id=%id.identifier(), "identified");

                Some(IdentifiedAddress {
                    address,
                    contract: Some(id.identifier()),
                    label: Some(id.name.clone()),
                    abi: Some(Cow::Borrowed(abi)),
                    artifact_id: Some(id.clone()),
                })
            })
            .collect()
    }
}
