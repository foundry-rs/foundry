use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Selector};
use foundry_compilers::ArtifactId;
use foundry_evm_core::utils::get_function;
use std::collections::BTreeMap;

/// Contains which contracts are to be targeted or excluded on an invariant test through their
/// artifact identifiers.
#[derive(Default)]
pub struct ArtifactFilters {
    /// List of `contract_path:contract_name` along with selectors, which are to be targeted. If
    /// list of functions is not empty, target only those.
    pub targeted: BTreeMap<String, Vec<Selector>>,
    /// List of `contract_path:contract_name` which are to be excluded.
    pub excluded: Vec<String>,
}

impl ArtifactFilters {
    /// Returns `true` if the given identifier matches this filter.
    pub fn matches(&self, identifier: &str) -> bool {
        (self.targeted.is_empty() || self.targeted.contains_key(identifier)) &&
            (self.excluded.is_empty() || !self.excluded.iter().any(|id| id == identifier))
    }

    /// Gets all the targeted functions from `artifact`. Returns error, if selectors do not match
    /// the `artifact`.
    ///
    /// An empty vector means that it targets any mutable function.
    pub fn get_targeted_functions(
        &self,
        artifact: &ArtifactId,
        abi: &JsonAbi,
    ) -> eyre::Result<Option<Vec<Function>>> {
        if let Some(selectors) = self.targeted.get(&artifact.identifier()) {
            let functions = selectors
                .iter()
                .map(|selector| get_function(&artifact.name, *selector, abi).cloned())
                .collect::<eyre::Result<Vec<_>>>()?;
            // targetArtifactSelectors > excludeArtifacts > targetArtifacts
            if functions.is_empty() && self.excluded.contains(&artifact.identifier()) {
                return Ok(None)
            }
            return Ok(Some(functions))
        }
        // If no contract is specifically targeted, and this contract is not excluded, then accept
        // all functions.
        if self.targeted.is_empty() && !self.excluded.contains(&artifact.identifier()) {
            return Ok(Some(vec![]))
        }
        Ok(None)
    }
}

/// Filter for acceptable senders to use for invariant testing. Exclusion takes priority if
/// clashing.
///
/// `address(0)` is excluded by default.
#[derive(Default)]
pub struct SenderFilters {
    pub targeted: Vec<Address>,
    pub excluded: Vec<Address>,
}

impl SenderFilters {
    pub fn new(mut targeted: Vec<Address>, mut excluded: Vec<Address>) -> Self {
        let addr_0 = Address::ZERO;
        if !excluded.contains(&addr_0) {
            excluded.push(addr_0);
        }
        targeted.retain(|addr| !excluded.contains(addr));
        Self { targeted, excluded }
    }
}
