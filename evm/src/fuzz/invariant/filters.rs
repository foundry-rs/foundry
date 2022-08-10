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
    /// Gets all the targeted functions from `artifact`. Returns error, if `artifact` doesn't exist.
    pub fn get_functions(&self, artifact: &ArtifactId, abi: &Abi) -> eyre::Result<Vec<Function>> {
        self.targeted
            .get(&artifact.identifier())
            .ok_or_else(|| {
                eyre::eyre!("{} does not exist inside ArtifactFilters.", artifact.identifier())
            })?
            .iter()
            .map(|selector| get_function(&artifact.name, selector, abi))
            .collect::<eyre::Result<Vec<_>>>()
    }
}
