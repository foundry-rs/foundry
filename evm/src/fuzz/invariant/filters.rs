use crate::utils::get_function;
use ethers::{
    abi::{Abi, FixedBytes, Function},
    solc::ArtifactId,
};
use std::collections::BTreeMap;

/// Contains which contracts are to be targeted or excluded on an invariant test through their
/// artifact identifiers.
#[derive(Default)]
pub struct ArtifactFilters {
    /// List of `contract_path:contract_name` which are to be targeted. If list of functions is not
    /// empty, target only those.
    pub targeted: BTreeMap<String, Vec<FixedBytes>>,
    /// List of `contract_path:contract_name` which are to be excluded.
    pub excluded: Vec<String>,
}

impl ArtifactFilters {
    /// Gets all the targeted functions from `artifact`. Returns error, if selectors do not match
    /// the `artifact`.
    ///
    /// An empty vector means that it targets any mutable function. See `select_random_function` for
    /// more.
    pub fn get_targeted_functions(
        &self,
        artifact: &ArtifactId,
        abi: &Abi,
    ) -> eyre::Result<Option<Vec<Function>>> {
        if let Some(selectors) = self.targeted.get(&artifact.identifier()) {
            let functions = selectors
                .iter()
                .map(|selector| get_function(&artifact.name, selector, abi))
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
