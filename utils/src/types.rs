use crate::diff_score;
use ethers_core::{abi::Abi, types::Address};
use ethers_solc::ArtifactId;
use std::collections::BTreeMap;

type ArtifactWithContractRef<'a> = (&'a ArtifactId, &'a (Abi, Vec<u8>));

/// Wrapper type that maps an artifact to a contract ABI and bytecode.
pub type ContractsByArtifact = BTreeMap<ArtifactId, (Abi, Vec<u8>)>;

pub trait ContractsByArtifactExt {
    /// Finds a contract which has a similar bytecode as `code`.
    fn find_by_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef>;
    /// Finds a contract which has the same contract name or identifier as `id`. If more than one is
    /// found, return error.
    fn find_by_name_or_identifier(&self, id: &str)
        -> eyre::Result<Option<ArtifactWithContractRef>>;
}

impl ContractsByArtifactExt for ContractsByArtifact {
    fn find_by_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef> {
        self.iter().find(|(_, (_, known_code))| diff_score(known_code, code) < 0.1)
    }

    fn find_by_name_or_identifier(
        &self,
        id: &str,
    ) -> eyre::Result<Option<ArtifactWithContractRef>> {
        let contracts = self
            .iter()
            .filter(|(artifact, _)| artifact.name == id || artifact.identifier() == id)
            .collect::<Vec<_>>();

        if contracts.len() > 1 {
            eyre::bail!("{id} has more than one implementation.");
        }

        Ok(contracts.first().cloned())
    }
}

/// Wrapper type that maps an address to a contract identifier and contract ABI.
pub type ContractsByAddress = BTreeMap<Address, (String, Abi)>;
