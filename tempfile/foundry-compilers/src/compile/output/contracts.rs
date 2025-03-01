use crate::{compilers::CompilerContract, ArtifactId};
use foundry_compilers_artifacts::{
    CompactContractBytecode, CompactContractRef, FileToContractsMap,
};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

/// file -> [(contract name  -> Contract + solc version)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionedContracts<C>(pub FileToContractsMap<Vec<VersionedContract<C>>>);

impl<C> Default for VersionedContracts<C> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<C> VersionedContracts<C>
where
    C: CompilerContract,
{
    /// Converts all `\\` separators in _all_ paths to `/`
    pub fn slash_paths(&mut self) {
        #[cfg(windows)]
        {
            use path_slash::PathExt;
            self.0 = std::mem::take(&mut self.0)
                .into_iter()
                .map(|(path, files)| (PathBuf::from(path.to_slash_lossy().as_ref()), files))
                .collect()
        }
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns an iterator over all files
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> + '_ {
        self.0.keys()
    }

    /// Finds the _first_ contract with the given name
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let output = project.compile()?.into_output();
    /// let contract = output.find_first("Greeter").unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn find_first(&self, contract_name: &str) -> Option<CompactContractRef<'_>> {
        self.contracts().find_map(|(name, contract)| {
            (name == contract_name).then(|| contract.as_compact_contract_ref())
        })
    }

    /// Finds the contract with matching path and name
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let output = project.compile()?.into_output();
    /// let contract = output.contracts.find("src/Greeter.sol".as_ref(), "Greeter").unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn find(
        &self,
        contract_path: &Path,
        contract_name: &str,
    ) -> Option<CompactContractRef<'_>> {
        self.contracts_with_files().find_map(|(path, name, contract)| {
            (path == contract_path && name == contract_name)
                .then(|| contract.as_compact_contract_ref())
        })
    }

    /// Removes the _first_ contract with the given name from the set
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let (_, mut contracts) = project.compile()?.into_output().split();
    /// let contract = contracts.remove_first("Greeter").unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove_first(&mut self, contract_name: &str) -> Option<C> {
        self.0.values_mut().find_map(|all_contracts| {
            let mut contract = None;
            if let Some((c, mut contracts)) = all_contracts.remove_entry(contract_name) {
                if !contracts.is_empty() {
                    contract = Some(contracts.remove(0).contract);
                }
                if !contracts.is_empty() {
                    all_contracts.insert(c, contracts);
                }
            }
            contract
        })
    }

    ///  Removes the contract with matching path and name
    ///
    /// # Examples
    /// ```no_run
    /// use foundry_compilers::{artifacts::*, Project};
    ///
    /// let project = Project::builder().build(Default::default())?;
    /// let (_, mut contracts) = project.compile()?.into_output().split();
    /// let contract = contracts.remove("src/Greeter.sol".as_ref(), "Greeter").unwrap();
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove(&mut self, path: &Path, contract_name: &str) -> Option<C> {
        let (key, mut all_contracts) = self.0.remove_entry(path)?;
        let mut contract = None;
        if let Some((c, mut contracts)) = all_contracts.remove_entry(contract_name) {
            if !contracts.is_empty() {
                contract = Some(contracts.remove(0).contract);
            }
            if !contracts.is_empty() {
                all_contracts.insert(c, contracts);
            }
        }

        if !all_contracts.is_empty() {
            self.0.insert(key, all_contracts);
        }
        contract
    }

    /// Given the contract file's path and the contract's name, tries to return the contract's
    /// bytecode, runtime bytecode, and ABI.
    pub fn get(&self, path: &Path, contract: &str) -> Option<CompactContractRef<'_>> {
        self.0
            .get(path)
            .and_then(|contracts| {
                contracts.get(contract).and_then(|c| c.first().map(|c| &c.contract))
            })
            .map(|c| c.as_compact_contract_ref())
    }

    /// Returns an iterator over all contracts and their names.
    pub fn contracts(&self) -> impl Iterator<Item = (&String, &C)> {
        self.0
            .values()
            .flat_map(|c| c.iter().flat_map(|(name, c)| c.iter().map(move |c| (name, &c.contract))))
    }

    /// Returns an iterator over (`file`, `name`, `Contract`).
    pub fn contracts_with_files(&self) -> impl Iterator<Item = (&PathBuf, &String, &C)> {
        self.0.iter().flat_map(|(file, contracts)| {
            contracts
                .iter()
                .flat_map(move |(name, c)| c.iter().map(move |c| (file, name, &c.contract)))
        })
    }

    /// Returns an iterator over (`file`, `name`, `Contract`, `Version`).
    pub fn contracts_with_files_and_version(
        &self,
    ) -> impl Iterator<Item = (&PathBuf, &String, &C, &Version)> {
        self.0.iter().flat_map(|(file, contracts)| {
            contracts.iter().flat_map(move |(name, c)| {
                c.iter().map(move |c| (file, name, &c.contract, &c.version))
            })
        })
    }

    /// Returns an iterator over all contracts and their source names.
    pub fn into_contracts(self) -> impl Iterator<Item = (String, C)> {
        self.0.into_values().flat_map(|c| {
            c.into_iter()
                .flat_map(|(name, c)| c.into_iter().map(move |c| (name.clone(), c.contract)))
        })
    }

    /// Returns an iterator over (`file`, `name`, `Contract`)
    pub fn into_contracts_with_files(self) -> impl Iterator<Item = (PathBuf, String, C)> {
        self.0.into_iter().flat_map(|(file, contracts)| {
            contracts.into_iter().flat_map(move |(name, c)| {
                let file = file.clone();
                c.into_iter().map(move |c| (file.clone(), name.clone(), c.contract))
            })
        })
    }

    /// Returns an iterator over (`file`, `name`, `Contract`, `Version`)
    pub fn into_contracts_with_files_and_version(
        self,
    ) -> impl Iterator<Item = (PathBuf, String, C, Version)> {
        self.0.into_iter().flat_map(|(file, contracts)| {
            contracts.into_iter().flat_map(move |(name, c)| {
                let file = file.clone();
                c.into_iter().map(move |c| (file.clone(), name.clone(), c.contract, c.version))
            })
        })
    }

    /// Sets the contract's file paths to `root` adjoined to `self.file`.
    pub fn join_all(&mut self, root: &Path) -> &mut Self {
        self.0 = std::mem::take(&mut self.0)
            .into_iter()
            .map(|(contract_path, contracts)| (root.join(contract_path), contracts))
            .collect();
        self
    }

    /// Removes `base` from all contract paths
    pub fn strip_prefix_all(&mut self, base: &Path) -> &mut Self {
        self.0 = std::mem::take(&mut self.0)
            .into_iter()
            .map(|(contract_path, contracts)| {
                (
                    contract_path.strip_prefix(base).unwrap_or(&contract_path).to_path_buf(),
                    contracts,
                )
            })
            .collect();
        self
    }
}

impl<C> AsRef<FileToContractsMap<Vec<VersionedContract<C>>>> for VersionedContracts<C>
where
    C: CompilerContract,
{
    fn as_ref(&self) -> &FileToContractsMap<Vec<VersionedContract<C>>> {
        &self.0
    }
}

impl<C> AsMut<FileToContractsMap<Vec<VersionedContract<C>>>> for VersionedContracts<C>
where
    C: CompilerContract,
{
    fn as_mut(&mut self) -> &mut FileToContractsMap<Vec<VersionedContract<C>>> {
        &mut self.0
    }
}

impl<C> Deref for VersionedContracts<C>
where
    C: CompilerContract,
{
    type Target = FileToContractsMap<Vec<VersionedContract<C>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<C> IntoIterator for VersionedContracts<C>
where
    C: CompilerContract,
{
    type Item = (PathBuf, BTreeMap<String, Vec<VersionedContract<C>>>);
    type IntoIter =
        std::collections::btree_map::IntoIter<PathBuf, BTreeMap<String, Vec<VersionedContract<C>>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A contract and the compiler version used to compile it
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedContract<C> {
    pub contract: C,
    pub version: Version,
    pub build_id: String,
    pub profile: String,
}

/// A mapping of `ArtifactId` and their `CompactContractBytecode`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtifactContracts<T = CompactContractBytecode>(pub BTreeMap<ArtifactId, T>);

impl<T: Serialize> Serialize for ArtifactContracts<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ArtifactContracts<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(BTreeMap::<_, _>::deserialize(deserializer)?))
    }
}

impl<T> Deref for ArtifactContracts<T> {
    type Target = BTreeMap<ArtifactId, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for ArtifactContracts<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<V, C: Into<V>> FromIterator<(ArtifactId, C)> for ArtifactContracts<V> {
    fn from_iter<T: IntoIterator<Item = (ArtifactId, C)>>(iter: T) -> Self {
        Self(iter.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

impl<T> IntoIterator for ArtifactContracts<T> {
    type Item = (ArtifactId, T);
    type IntoIter = std::collections::btree_map::IntoIter<ArtifactId, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
