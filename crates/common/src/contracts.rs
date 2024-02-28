//! Commonly used contract types and functions.

use alloy_json_abi::{Event, Function, JsonAbi};
use alloy_primitives::{hex, Address, Selector, B256};
use eyre::Result;
use foundry_compilers::{
    artifacts::{CompactContractBytecode, ContractBytecodeSome},
    ArtifactId, ProjectPathsConfig,
};
use std::{
    collections::BTreeMap,
    fmt,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

type ArtifactWithContractRef<'a> = (&'a ArtifactId, &'a (JsonAbi, Vec<u8>));

/// Wrapper type that maps an artifact to a contract ABI and bytecode.
#[derive(Clone, Default)]
pub struct ContractsByArtifact(pub BTreeMap<ArtifactId, (JsonAbi, Vec<u8>)>);

impl fmt::Debug for ContractsByArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter().map(|(k, (v1, v2))| (k, (v1, hex::encode(v2))))).finish()
    }
}

impl ContractsByArtifact {
    /// Finds a contract which has a similar bytecode as `code`.
    pub fn find_by_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef> {
        self.iter().find(|(_, (_, known_code))| bytecode_diff_score(known_code, code) <= 0.1)
    }

    /// Finds a contract which has the same contract name or identifier as `id`. If more than one is
    /// found, return error.
    pub fn find_by_name_or_identifier(&self, id: &str) -> Result<Option<ArtifactWithContractRef>> {
        let contracts = self
            .iter()
            .filter(|(artifact, _)| artifact.name == id || artifact.identifier() == id)
            .collect::<Vec<_>>();

        if contracts.len() > 1 {
            eyre::bail!("{id} has more than one implementation.");
        }

        Ok(contracts.first().cloned())
    }

    /// Flattens the contracts into functions, events and errors.
    pub fn flatten(&self) -> (BTreeMap<Selector, Function>, BTreeMap<B256, Event>, JsonAbi) {
        let mut funcs = BTreeMap::new();
        let mut events = BTreeMap::new();
        let mut errors_abi = JsonAbi::new();
        for (_name, (abi, _code)) in self.iter() {
            for func in abi.functions() {
                funcs.insert(func.selector(), func.clone());
            }
            for event in abi.events() {
                events.insert(event.selector(), event.clone());
            }
            for error in abi.errors() {
                errors_abi.errors.entry(error.name.clone()).or_default().push(error.clone());
            }
        }
        (funcs, events, errors_abi)
    }
}

impl Deref for ContractsByArtifact {
    type Target = BTreeMap<ArtifactId, (JsonAbi, Vec<u8>)>;

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
pub type ContractsByAddress = BTreeMap<Address, (String, JsonAbi)>;

/// Very simple fuzzy matching of contract bytecode.
///
/// Returns a value between `0.0` (identical) and `1.0` (completely different).
pub fn bytecode_diff_score<'a>(mut a: &'a [u8], mut b: &'a [u8]) -> f64 {
    // Make sure `a` is the longer one.
    if a.len() < b.len() {
        std::mem::swap(&mut a, &mut b);
    }

    // Account for different lengths.
    let mut n_different_bytes = a.len() - b.len();

    // If the difference is more than 32 bytes and more than 10% of the total length,
    // we assume the bytecodes are completely different.
    // This is a simple heuristic to avoid checking every byte when the lengths are very different.
    // 32 is chosen to be a reasonable minimum as it's the size of metadata hashes and one EVM word.
    if n_different_bytes > 32 && n_different_bytes * 10 > a.len() {
        return 1.0;
    }

    // Count different bytes.
    // SAFETY: `a` is longer than `b`.
    n_different_bytes += unsafe { count_different_bytes(a, b) };

    n_different_bytes as f64 / a.len() as f64
}

/// Returns the amount of different bytes between two slices.
///
/// # Safety
///
/// `a` must be at least as long as `b`.
unsafe fn count_different_bytes(a: &[u8], b: &[u8]) -> usize {
    // This could've been written as `std::iter::zip(a, b).filter(|(x, y)| x != y).count()`,
    // however this function is very hot, and has been written to be as primitive as
    // possible for lower optimization levels.

    let a_ptr = a.as_ptr();
    let b_ptr = b.as_ptr();
    let len = b.len();

    let mut sum = 0;
    let mut i = 0;
    while i < len {
        // SAFETY: `a` is at least as long as `b`, and `i` is in bound of `b`.
        sum += unsafe { *a_ptr.add(i) != *b_ptr.add(i) } as usize;
        i += 1;
    }
    sum
}

/// Flattens the contracts into  (`id` -> (`JsonAbi`, `Vec<u8>`)) pairs
pub fn flatten_contracts(
    contracts: &BTreeMap<ArtifactId, ContractBytecodeSome>,
    deployed_code: bool,
) -> ContractsByArtifact {
    ContractsByArtifact(
        contracts
            .iter()
            .filter_map(|(id, c)| {
                let bytecode =
                    if deployed_code { c.deployed_bytecode.bytes() } else { c.bytecode.bytes() };
                bytecode.cloned().map(|code| (id.clone(), (c.abi.clone(), code.into())))
            })
            .collect(),
    )
}

/// Artifact/Contract identifier can take the following form:
/// `<artifact file name>:<contract name>`, the `artifact file name` is the name of the json file of
/// the contract's artifact and the contract name is the name of the solidity contract, like
/// `SafeTransferLibTest.json:SafeTransferLibTest`
///
/// This returns the `contract name` part
///
/// # Example
///
/// ```
/// use foundry_common::*;
/// assert_eq!(
///     "SafeTransferLibTest",
///     get_contract_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_contract_name(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// This returns the `file name` part, See [`get_contract_name`]
///
/// # Example
///
/// ```
/// use foundry_common::*;
/// assert_eq!(
///     "SafeTransferLibTest.json",
///     get_file_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_file_name(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

/// Returns the path to the json artifact depending on the input
pub fn get_artifact_path(paths: &ProjectPathsConfig, path: &str) -> PathBuf {
    if path.ends_with(".json") {
        PathBuf::from(path)
    } else {
        let parts: Vec<&str> = path.split(':').collect();
        let file = parts[0];
        let contract_name =
            if parts.len() == 1 { parts[0].replace(".sol", "") } else { parts[1].to_string() };
        paths.artifacts.join(format!("{file}/{contract_name}.json"))
    }
}

/// Helper function to convert CompactContractBytecode ~> ContractBytecodeSome
pub fn compact_to_contract(contract: CompactContractBytecode) -> Result<ContractBytecodeSome> {
    Ok(ContractBytecodeSome {
        abi: contract.abi.ok_or_else(|| eyre::eyre!("No contract abi"))?,
        bytecode: contract.bytecode.ok_or_else(|| eyre::eyre!("No contract bytecode"))?.into(),
        deployed_bytecode: contract
            .deployed_bytecode
            .ok_or_else(|| eyre::eyre!("No contract deployed bytecode"))?
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytecode_diffing() {
        assert_eq!(bytecode_diff_score(b"a", b"a"), 0.0);
        assert_eq!(bytecode_diff_score(b"a", b"b"), 1.0);

        let a_100 = &b"a".repeat(100)[..];
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(100)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(99)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(101)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(120)), 1.0);
        assert_eq!(bytecode_diff_score(a_100, &b"b".repeat(1000)), 1.0);

        let a_99 = &b"a".repeat(99)[..];
        assert!(bytecode_diff_score(a_100, a_99) <= 0.01);
    }
}
