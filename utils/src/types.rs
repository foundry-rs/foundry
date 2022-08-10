use crate::diff_score;
use ethers_core::{abi::Abi, types::Address};
use ethers_solc::ArtifactId;
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
};

type ArtifactWithContractRef<'a> = (&'a ArtifactId, &'a (Abi, Vec<u8>));

/// Wrapper type that maps an artifact to a contract ABI and bytecode.
#[derive(Default)]
pub struct ContractsByArtifact(pub BTreeMap<ArtifactId, (Abi, Vec<u8>)>);

impl ContractsByArtifact {
    /// Finds a contract which has a similar bytecode as `code`.
    pub fn find_by_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef> {
        self.iter().find(|(_, (_, known_code))| diff_score(known_code, code) < 0.1)
    }
    /// Finds a contract which has the same contract name or identifier as `id`. If more than one is
    /// found, return error.
    pub fn find_by_name_or_identifier(
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

impl Deref for ContractsByArtifact {
    type Target = BTreeMap<ArtifactId, (Abi, Vec<u8>)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ContractsByArtifact {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Wrapper type that maps an address to a contract identifier and contract ABI.
pub type ContractsByAddress = BTreeMap<Address, (String, Abi)>;
