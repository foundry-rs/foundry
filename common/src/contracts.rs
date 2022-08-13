//! commonly used contract types and functions

use ethers_core::{
    abi::{Abi, Event, Function},
    types::{Address, H256},
};
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

    /// Flattens a group of contracts into maps of all events and functions
    pub fn flatten(&self) -> (BTreeMap<[u8; 4], Function>, BTreeMap<H256, Event>, Abi) {
        let flattened_funcs: BTreeMap<[u8; 4], Function> = self
            .iter()
            .flat_map(|(_name, (abi, _code))| {
                abi.functions()
                    .map(|func| (func.short_signature(), func.clone()))
                    .collect::<BTreeMap<[u8; 4], Function>>()
            })
            .collect();

        let flattened_events: BTreeMap<H256, Event> = self
            .iter()
            .flat_map(|(_name, (abi, _code))| {
                abi.events()
                    .map(|event| (event.signature(), event.clone()))
                    .collect::<BTreeMap<H256, Event>>()
            })
            .collect();

        // We need this for better revert decoding, and want it in abi form
        let mut errors_abi = Abi::default();
        self.iter().for_each(|(_name, (abi, _code))| {
            abi.errors().for_each(|error| {
                let entry =
                    errors_abi.errors.entry(error.name.clone()).or_insert_with(Default::default);
                entry.push(error.clone());
            });
        });
        (flattened_funcs, flattened_events, errors_abi)
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

/// Very simple fuzzy matching of contract bytecode.
///
/// Will fail for small contracts that are essentially all immutable variables.
pub fn diff_score(a: &[u8], b: &[u8]) -> f64 {
    let cutoff_len = usize::min(a.len(), b.len());
    if cutoff_len == 0 {
        return 1.0
    }

    let a = &a[..cutoff_len];
    let b = &b[..cutoff_len];
    let mut diff_chars = 0;
    for i in 0..cutoff_len {
        if a[i] != b[i] {
            diff_chars += 1;
        }
    }
    diff_chars as f64 / cutoff_len as f64
}
