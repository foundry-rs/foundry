//! # foundry-linking
//!
//! EVM bytecode linker.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use alloy_primitives::{Address, B256, Bytes, map::HashMap as AlloyHashMap};
use foundry_compilers::{
    Artifact, ArtifactId,
    artifacts::{CompactBytecode, CompactContractBytecodeCow, Libraries},
    contracts::ArtifactContracts,
};
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::OnceLock,
};

/// Errors that can occur during linking.
#[derive(Debug, thiserror::Error)]
pub enum LinkerError {
    #[error("wasn't able to find artifact for library {name} at {file}")]
    MissingLibraryArtifact { file: String, name: String },
    #[error("multiple library artifacts resolve to the same key {file}:{name}")]
    ConflictingLibraryArtifacts { file: String, name: String },
    #[error("target artifact is not present in provided artifacts set")]
    MissingTargetArtifact,
    #[error(transparent)]
    InvalidAddress(<Address as std::str::FromStr>::Err),
    #[error("cyclic dependency found, can't link libraries via CREATE2")]
    CyclicDependency,
    #[error("failed linking {artifact}")]
    LinkingFailed { artifact: String },
}

type Index = AlloyHashMap<PathBuf, AlloyHashMap<String, Vec<ArtifactId>>>;

pub struct Linker<'a> {
    /// Root of the project, used to determine whether artifact/library path can be stripped.
    pub root: PathBuf,
    /// Compilation artifacts.
    pub contracts: ArtifactContracts<CompactContractBytecodeCow<'a>>,
}

/// Reuses artifact lookups across multiple linker operations.
pub struct Resolver<'a, 'b> {
    linker: &'b Linker<'a>,
    index: OnceLock<Index>,
}

/// Output of the `link_with_nonce_or_address`
pub struct LinkOutput {
    /// Flattened resolved library addresses. Contains both user-provided and newly deployed
    /// libraries, with stripped path prefixes. Auto-linked keys that resolve to different
    /// addresses for different artifacts are omitted; use
    /// [`DetailedLinkOutput::artifact_libraries`] for complete per-artifact mappings.
    pub libraries: Libraries,
    /// Addresses of libraries required by the linked targets.
    pub library_addresses: Vec<Address>,
    /// Vector of libraries that need to be deployed from sender address.
    /// The order in which they appear in the vector is the order in which they should be deployed.
    pub libs_to_deploy: Vec<Bytes>,
}

/// Detailed linker output for callers that need metadata about auto-linked libraries.
pub struct DetailedLinkOutput {
    /// The backwards-compatible linker output.
    pub output: LinkOutput,
    /// Complete transitive library address mapping selected for each linked artifact.
    pub artifact_libraries: BTreeMap<ArtifactId, Libraries>,
    /// Address selected for every resolved library artifact, including byte-identical variants.
    pub artifact_addresses: BTreeMap<ArtifactId, Address>,
    /// Unique physical library deployments and their linked creation bytecode.
    ///
    /// Multiple byte-identical artifacts can share one CREATE2 deployment. In that case, this
    /// contains one representative artifact while [`Self::artifact_addresses`] preserves every
    /// artifact identity.
    pub linked_libraries: Vec<LinkedLibrary>,
}

/// An auto-linked library and the data used to deploy and classify it.
#[derive(Clone, Debug)]
pub struct LinkedLibrary {
    /// Compilation artifact for the library.
    pub id: ArtifactId,
    /// Address assigned by the linker.
    pub address: Address,
    /// Fully linked creation bytecode.
    pub bytecode: Bytes,
}

impl<'a> Linker<'a> {
    pub fn new(
        root: impl Into<PathBuf>,
        contracts: ArtifactContracts<CompactContractBytecodeCow<'a>>,
    ) -> Self {
        Linker { root: root.into(), contracts }
    }

    /// Helper method to convert [ArtifactId] to the format in which libraries are stored in
    /// [Libraries] object.
    ///
    /// Strips project root path from source file path.
    fn convert_artifact_id_to_lib_path(&self, id: &ArtifactId) -> (PathBuf, String) {
        // name is either {LibName} or {LibName}.{version}
        let name = id.name.split('.').next().unwrap();

        (self.project_relative_path(&id.source), name.to_owned())
    }

    fn project_relative_path(&self, path: &Path) -> PathBuf {
        if path.is_relative() {
            return path.to_path_buf();
        }

        if let Ok(stripped) = path.strip_prefix(&self.root) {
            return stripped.to_path_buf();
        }

        if let Ok(root) = self.root.canonicalize()
            && let Ok(path) = path.canonicalize()
            && let Ok(stripped) = path.strip_prefix(root)
        {
            return stripped.to_path_buf();
        }

        path.to_path_buf()
    }

    fn index<'b>(&self, index: &'b OnceLock<Index>) -> &'b Index {
        index.get_or_init(|| {
            let mut artifacts = AlloyHashMap::<_, AlloyHashMap<_, Vec<_>>>::default();
            for id in self.contracts.keys() {
                let path = self.project_relative_path(&id.source);
                let name = id.name.split('.').next().unwrap().to_owned();
                artifacts.entry(path).or_default().entry(name).or_default().push(id.clone());
            }
            artifacts
        })
    }

    /// Resolves `path` against the project root and canonicalizes it for comparison only.
    fn canonical_path(&self, path: &Path) -> Option<PathBuf> {
        let path = if path.is_relative() { self.root.join(path) } else { path.to_path_buf() };
        path.canonicalize().ok()
    }

    fn path_matches(
        &self,
        path: &Path,
        expected: &Path,
        canonical_expected: Option<&Path>,
    ) -> bool {
        let path = self.project_relative_path(path);
        path == expected
            || canonical_expected
                .is_some_and(|expected| self.canonical_path(&path).as_deref() == Some(expected))
    }

    fn link_bytecode(
        &self,
        bytecode: &mut CompactBytecode,
        target: &ArtifactId,
        file: &Path,
        name: &str,
        address: Address,
    ) -> Result<(), LinkerError> {
        self.link_bytecode_inner(bytecode, target, file, name, address, true)
    }

    fn link_bytecode_inner(
        &self,
        bytecode: &mut CompactBytecode,
        target: &ArtifactId,
        file: &Path,
        name: &str,
        address: Address,
        canonical: bool,
    ) -> Result<(), LinkerError> {
        let file_relative = self.project_relative_path(file);
        let canonical_file = self.canonical_path(&file_relative);
        let mut references = Vec::new();
        for (reference, libraries) in &bytecode.link_references {
            if !libraries.contains_key(name) {
                continue;
            }
            let reference_path = self.project_relative_path(Path::new(reference));
            if reference_path == file_relative {
                references.push(reference.clone());
                continue;
            }
            if !canonical {
                continue;
            }
            if !self.path_matches(Path::new(reference), &file_relative, canonical_file.as_deref()) {
                continue;
            }
            if let Some(id) =
                self.find_artifact_id_by_library_path(None, reference, name, target)?
            {
                let (artifact_file, artifact_name) = self.convert_artifact_id_to_lib_path(id);
                if artifact_file == file_relative && artifact_name == name {
                    references.push(reference.clone());
                }
            }
        }

        if references.is_empty() && canonical {
            bytecode.link(&file.to_string_lossy(), name, address);
        } else {
            for reference in references {
                bytecode.link(&reference, name, address);
            }
        }
        Ok(())
    }

    /// Finds an [ArtifactId] object in the given [ArtifactContracts] keys which corresponds to the
    /// library path in the form of "./path/to/Lib.sol:Lib"
    ///
    /// Dependency lookups can use the index. Canonical paths retain the fallback scan required
    /// for symlinked library aliases.
    fn find_artifact_id_by_library_path<'b>(
        &'b self,
        index: Option<&'b OnceLock<Index>>,
        file: &str,
        name: &str,
        target: &ArtifactId,
    ) -> Result<Option<&'b ArtifactId>, LinkerError> {
        let library_path = self.project_relative_path(Path::new(file));
        let candidates = if let Some(index) = index {
            self.index(index)
                .get(&library_path)
                .and_then(|artifacts| artifacts.get(name))
                .into_iter()
                .flatten()
                .filter(|id| id.version == target.version)
                .collect::<Vec<_>>()
        } else {
            self.contracts
                .keys()
                .filter(|id| {
                    if id.version != target.version {
                        return false;
                    }
                    let (artifact_path, artifact_name) = self.convert_artifact_id_to_lib_path(id);
                    artifact_name == *name && artifact_path == library_path
                })
                .collect()
        };
        let candidates = if candidates.is_empty() {
            let canonical_library_path = self.canonical_path(&library_path);
            self.contracts
                .keys()
                .filter(|id| {
                    if id.version != target.version {
                        return false;
                    }
                    let (artifact_path, artifact_name) = self.convert_artifact_id_to_lib_path(id);
                    artifact_name == *name
                        && self.path_matches(
                            &artifact_path,
                            &library_path,
                            canonical_library_path.as_deref(),
                        )
                })
                .collect()
        } else {
            candidates
        };
        let same_build_and_profile = candidates
            .iter()
            .copied()
            .filter(|id| id.build_id == target.build_id && id.profile == target.profile)
            .collect::<Vec<_>>();
        let candidates =
            if same_build_and_profile.is_empty() { candidates } else { same_build_and_profile };

        if candidates.len() > 1 {
            return Err(LinkerError::ConflictingLibraryArtifacts {
                file: library_path.display().to_string(),
                name: name.to_owned(),
            });
        }
        Ok(candidates.into_iter().next())
    }

    fn direct_dependencies<'b>(
        &'b self,
        index: &'b OnceLock<Index>,
        target: &ArtifactId,
        references: &mut BTreeMap<&'b ArtifactId, BTreeSet<PathBuf>>,
    ) -> Result<BTreeSet<&'b ArtifactId>, LinkerError> {
        let contract = self.contracts.get(target).ok_or(LinkerError::MissingTargetArtifact)?;

        let mut link_references: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut extend = |bytecode: &CompactBytecode| {
            for (file, libs) in &bytecode.link_references {
                link_references.entry(file.clone()).or_default().extend(libs.keys().cloned());
            }
        };
        if let Some(bytecode) = &contract.bytecode {
            extend(bytecode);
        }
        if let Some(deployed_bytecode) = &contract.deployed_bytecode
            && let Some(bytecode) = &deployed_bytecode.bytecode
        {
            extend(bytecode);
        }

        let mut dependencies = BTreeSet::new();
        for (file, libs) in link_references {
            for name in libs {
                let id = self
                    .find_artifact_id_by_library_path(Some(index), &file, &name, target)?
                    .ok_or_else(|| LinkerError::MissingLibraryArtifact {
                        file: file.clone(),
                        name,
                    })?;
                references.entry(id).or_default().insert(file.clone().into());
                dependencies.insert(id);
            }
        }

        Ok(dependencies)
    }

    /// Performs DFS on the graph of link references, and populates `deps` with all found libraries.
    fn collect_dependencies<'b>(
        &'b self,
        index: &'b OnceLock<Index>,
        target: &ArtifactId,
        deps: &mut BTreeSet<&'b ArtifactId>,
        references: &mut BTreeMap<&'b ArtifactId, BTreeSet<PathBuf>>,
    ) -> Result<(), LinkerError> {
        for id in self.direct_dependencies(index, target, references)? {
            if deps.insert(id) {
                self.collect_dependencies(index, id, deps, references)?;
            }
        }

        Ok(())
    }

    fn apply_configured_references(
        &self,
        references: &BTreeMap<&ArtifactId, BTreeSet<PathBuf>>,
        libraries: &mut Libraries,
    ) -> Result<(), LinkerError> {
        for (id, references) in references {
            let (file, name) = self.convert_artifact_id_to_lib_path(id);
            let mut configured = references
                .iter()
                .filter_map(|reference| libraries.libs.get(reference)?.get(&name))
                .map(|address| Address::from_str(address).map_err(LinkerError::InvalidAddress))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let exact = !configured.is_empty();
            if configured.is_empty()
                && let Some(canonical_file) = self.canonical_path(&file)
            {
                configured = libraries
                    .libs
                    .iter()
                    .filter_map(|(configured_file, libraries)| {
                        if self.canonical_path(configured_file).as_deref() == Some(&canonical_file)
                        {
                            libraries.get(&name)
                        } else {
                            None
                        }
                    })
                    .map(|address| Address::from_str(address).map_err(LinkerError::InvalidAddress))
                    .collect::<Result<BTreeSet<_>, _>>()?;
            }
            if configured.len() > 1 {
                return Err(LinkerError::ConflictingLibraryArtifacts {
                    file: file.display().to_string(),
                    name,
                });
            }
            let Some(address) = configured.first().copied() else { continue };
            let canonical = libraries.libs.entry(file).or_default();
            if exact {
                canonical.insert(name, address.to_checksum(None));
            } else {
                canonical.entry(name).or_insert_with(|| address.to_checksum(None));
            }
        }
        Ok(())
    }

    fn collect_library_keys(
        &self,
        needed_libraries: &BTreeSet<&ArtifactId>,
        libraries: &Libraries,
    ) -> Result<BTreeSet<(PathBuf, String)>, LinkerError> {
        let mut library_keys = BTreeSet::new();
        for id in needed_libraries {
            let (file, name) = self.convert_artifact_id_to_lib_path(id);
            let is_configured =
                libraries.libs.get(&file).is_some_and(|libraries| libraries.contains_key(&name));
            if !library_keys.insert((file.clone(), name.clone())) && !is_configured {
                return Err(LinkerError::ConflictingLibraryArtifacts {
                    file: file.display().to_string(),
                    name,
                });
            }
        }
        Ok(library_keys)
    }

    fn configured_library_address(
        &self,
        id: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<Option<Address>, LinkerError> {
        let (file, name) = self.convert_artifact_id_to_lib_path(id);
        libraries
            .libs
            .get(&file)
            .and_then(|libraries| libraries.get(&name))
            .map(|address| Address::from_str(address).map_err(LinkerError::InvalidAddress))
            .transpose()
    }

    fn libraries_for_artifact<'b>(
        &'b self,
        index: &'b OnceLock<Index>,
        target: &ArtifactId,
        configured: &Libraries,
        addresses: &BTreeMap<&'b ArtifactId, Address>,
    ) -> Result<Libraries, LinkerError> {
        let mut libraries = configured.clone();
        let mut dependencies = BTreeSet::new();
        self.collect_dependencies(index, target, &mut dependencies, &mut BTreeMap::new())?;
        for id in dependencies {
            let Some(&address) = addresses.get(id) else { continue };
            let (file, name) = self.convert_artifact_id_to_lib_path(id);
            let entry = libraries.libs.entry(file.clone()).or_default().entry(name.clone());
            if let std::collections::btree_map::Entry::Occupied(entry) = &entry
                && Address::from_str(entry.get()).map_err(LinkerError::InvalidAddress)? != address
            {
                return Err(LinkerError::ConflictingLibraryArtifacts {
                    file: file.display().to_string(),
                    name,
                });
            }
            entry.or_insert_with(|| address.to_checksum(None));
        }
        Ok(libraries)
    }

    fn artifact_libraries<'b>(
        &'b self,
        index: &'b OnceLock<Index>,
        artifacts: impl IntoIterator<Item = &'b ArtifactId>,
        configured: &Libraries,
        addresses: &BTreeMap<&'b ArtifactId, Address>,
    ) -> Result<BTreeMap<ArtifactId, Libraries>, LinkerError> {
        artifacts
            .into_iter()
            .map(|id| {
                Ok((id.clone(), self.libraries_for_artifact(index, id, configured, addresses)?))
            })
            .collect()
    }

    fn output_libraries(
        &self,
        mut libraries: Libraries,
        addresses: &BTreeMap<&ArtifactId, Address>,
    ) -> Libraries {
        let mut by_key = BTreeMap::<(PathBuf, String), BTreeSet<Address>>::new();
        for (&id, &address) in addresses {
            by_key.entry(self.convert_artifact_id_to_lib_path(id)).or_default().insert(address);
        }
        for ((file, name), addresses) in by_key {
            if let Some(address) = addresses.first().filter(|_| addresses.len() == 1) {
                libraries
                    .libs
                    .entry(file)
                    .or_default()
                    .entry(name)
                    .or_insert_with(|| address.to_checksum(None));
            }
        }
        libraries
    }

    fn assigned_library_addresses(
        &self,
        addresses: &BTreeMap<&ArtifactId, Address>,
    ) -> Vec<Address> {
        addresses.values().copied().collect::<BTreeSet<_>>().into_iter().collect()
    }

    fn artifact_addresses(
        &self,
        addresses: &BTreeMap<&ArtifactId, Address>,
    ) -> BTreeMap<ArtifactId, Address> {
        addresses.iter().map(|(&id, &address)| (id.clone(), address)).collect()
    }

    fn artifact_addresses_from_libraries(
        &self,
        artifacts: impl IntoIterator<Item = &'a ArtifactId>,
        libraries: &Libraries,
    ) -> Result<BTreeMap<ArtifactId, Address>, LinkerError> {
        artifacts
            .into_iter()
            .map(|id| {
                let address = self
                    .configured_library_address(id, libraries)?
                    .ok_or_else(|| LinkerError::LinkingFailed { artifact: id.identifier() })?;
                Ok((id.clone(), address))
            })
            .collect()
    }

    fn ensure_flattened_libraries(
        &self,
        artifact_libraries: &BTreeMap<ArtifactId, Libraries>,
    ) -> Result<(), LinkerError> {
        let mut addresses = BTreeMap::<(PathBuf, String), BTreeSet<Address>>::new();
        for libraries in artifact_libraries.values() {
            for (file, libraries) in &libraries.libs {
                for (name, address) in libraries {
                    addresses
                        .entry((file.clone(), name.clone()))
                        .or_default()
                        .insert(Address::from_str(address).map_err(LinkerError::InvalidAddress)?);
                }
            }
        }
        if let Some(((file, name), _)) =
            addresses.into_iter().find(|(_, addresses)| addresses.len() > 1)
        {
            return Err(LinkerError::ConflictingLibraryArtifacts {
                file: file.display().to_string(),
                name,
            });
        }
        Ok(())
    }

    fn library_addresses(
        &self,
        library_keys: &BTreeSet<(PathBuf, String)>,
        libraries: &Libraries,
    ) -> Result<Vec<Address>, LinkerError> {
        let addresses = library_keys
            .iter()
            .map(|(file, name)| {
                let address =
                    libraries.libs.get(file).and_then(|libraries| libraries.get(name)).ok_or_else(
                        || LinkerError::LinkingFailed {
                            artifact: format!("{}:{name}", file.display()),
                        },
                    )?;
                Address::from_str(address).map_err(LinkerError::InvalidAddress)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(addresses.into_iter().collect())
    }

    /// Returns the resolved addresses of all libraries required by `target`.
    pub fn linked_library_addresses(
        &'a self,
        target: &'a ArtifactId,
        libraries: &Libraries,
    ) -> Result<BTreeSet<Address>, LinkerError> {
        Resolver::new(self).linked_library_addresses(target, libraries)
    }

    fn linked_library_addresses_inner<'b>(
        &'b self,
        index: &'b OnceLock<Index>,
        target: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<BTreeSet<Address>, LinkerError> {
        let mut dependencies = BTreeSet::new();
        let mut references = BTreeMap::new();
        self.collect_dependencies(index, target, &mut dependencies, &mut references)?;
        let mut libraries = libraries.clone();
        self.apply_configured_references(&references, &mut libraries)?;

        dependencies
            .into_iter()
            .map(|id| {
                let (file, name) = self.convert_artifact_id_to_lib_path(id);
                libraries
                    .libs
                    .get(&file)
                    .and_then(|libs| libs.get(&name))
                    .ok_or_else(|| LinkerError::LinkingFailed { artifact: id.identifier() })?
                    .parse()
                    .map_err(LinkerError::InvalidAddress)
            })
            .collect()
    }

    /// Returns the transitive set of libraries referenced by `target`.
    pub fn dependencies(
        &'a self,
        target: &'a ArtifactId,
    ) -> Result<BTreeSet<ArtifactId>, LinkerError> {
        let index = OnceLock::new();
        self.dependencies_inner(&index, target)
    }

    fn dependencies_inner<'b>(
        &'b self,
        index: &'b OnceLock<Index>,
        target: &ArtifactId,
    ) -> Result<BTreeSet<ArtifactId>, LinkerError> {
        let mut dependencies = BTreeSet::new();
        self.collect_dependencies(index, target, &mut dependencies, &mut BTreeMap::new())?;
        Ok(dependencies.into_iter().cloned().collect())
    }

    fn linked_creation_bytecode(
        &self,
        target: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<Bytes, LinkerError> {
        let contract = self.link(target, libraries)?;
        self.ensure_linked(&contract, target)?;
        contract.get_bytecode_bytes().map(|code| code.into_owned()).ok_or_else(|| {
            LinkerError::LinkingFailed { artifact: target.source.to_string_lossy().into_owned() }
        })
    }

    /// Links given artifact with either given library addresses or address computed from sender and
    /// nonce.
    ///
    /// Each key in `libraries` should either be a global path or relative to project root. All
    /// remappings should be resolved.
    ///
    /// When calling for `target` being an external library itself, you should check that `target`
    /// does not appear in `libs_to_deploy` to avoid deploying it twice. It may happen in cases
    /// when there is a dependency cycle including `target`.
    pub fn link_with_nonce_or_address(
        &'a self,
        libraries: Libraries,
        sender: Address,
        nonce: u64,
        targets: impl IntoIterator<Item = &'a ArtifactId>,
    ) -> Result<LinkOutput, LinkerError> {
        let output = self.link_with_nonce_or_address_detailed(libraries, sender, nonce, targets)?;
        self.ensure_flattened_libraries(&output.artifact_libraries)?;
        Ok(output.output)
    }

    /// Links like [`Self::link_with_nonce_or_address`] and includes auto-linked library metadata.
    pub fn link_with_nonce_or_address_detailed(
        &'a self,
        libraries: Libraries,
        sender: Address,
        mut nonce: u64,
        targets: impl IntoIterator<Item = &'a ArtifactId>,
    ) -> Result<DetailedLinkOutput, LinkerError> {
        let index = OnceLock::new();
        // Library paths in `link_references` keys are always stripped, so we have to strip
        // user-provided paths to be able to match them correctly.
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());

        let targets = targets.into_iter().collect::<Vec<_>>();
        let mut needed_libraries = BTreeSet::new();
        let mut references = BTreeMap::new();
        for &target in &targets {
            self.collect_dependencies(&index, target, &mut needed_libraries, &mut references)?;
        }
        self.apply_configured_references(&references, &mut libraries)?;

        let mut addresses = BTreeMap::new();
        let mut libs_to_deploy = Vec::new();
        for &id in &needed_libraries {
            let address = if let Some(address) = self.configured_library_address(id, &libraries)? {
                address
            } else {
                let address = sender.create(nonce);
                nonce += 1;
                libs_to_deploy.push((id, address));
                address
            };
            addresses.insert(id, address);
        }

        let artifact_libraries = self.artifact_libraries(
            &index,
            targets.iter().copied().chain(needed_libraries.iter().copied()),
            &libraries,
            &addresses,
        )?;

        // Link and collect bytecodes for `libs_to_deploy`.
        let linked_libraries = libs_to_deploy
            .into_par_iter()
            .map(|(id, address)| {
                let artifact_libraries = artifact_libraries.get(id).unwrap();
                let bytecode =
                    self.link(id, artifact_libraries)?.get_bytecode_bytes().unwrap().into_owned();
                Ok(LinkedLibrary { id: id.clone(), address, bytecode })
            })
            .collect::<Result<Vec<_>, LinkerError>>()?;
        let libs_to_deploy = linked_libraries.iter().map(|lib| lib.bytecode.clone()).collect();

        let library_addresses = self.assigned_library_addresses(&addresses);
        let artifact_addresses = self.artifact_addresses(&addresses);
        let libraries = self.output_libraries(libraries, &addresses);
        Ok(DetailedLinkOutput {
            output: LinkOutput { libraries, library_addresses, libs_to_deploy },
            artifact_libraries,
            artifact_addresses,
            linked_libraries,
        })
    }

    pub fn link_with_create2(
        &'a self,
        libraries: Libraries,
        sender: Address,
        salt: B256,
        targets: impl IntoIterator<Item = &'a ArtifactId>,
    ) -> Result<LinkOutput, LinkerError> {
        let output = self.link_with_create2_detailed(libraries, sender, salt, targets)?;
        self.ensure_flattened_libraries(&output.artifact_libraries)?;
        Ok(output.output)
    }

    /// Links like [`Self::link_with_create2`] and includes auto-linked library metadata.
    pub fn link_with_create2_detailed(
        &'a self,
        libraries: Libraries,
        sender: Address,
        salt: B256,
        targets: impl IntoIterator<Item = &'a ArtifactId>,
    ) -> Result<DetailedLinkOutput, LinkerError> {
        let index = OnceLock::new();
        // Library paths in `link_references` keys are always stripped, so we have to strip
        // user-provided paths to be able to match them correctly.
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());

        let targets = targets.into_iter().collect::<Vec<_>>();
        let mut needed_libraries = BTreeSet::new();
        let mut references = BTreeMap::new();
        for &target in &targets {
            self.collect_dependencies(&index, target, &mut needed_libraries, &mut references)?;
        }
        self.apply_configured_references(&references, &mut libraries)?;

        let mut addresses = BTreeMap::new();
        let mut pending = Vec::new();
        for &id in &needed_libraries {
            if let Some(address) = self.configured_library_address(id, &libraries)? {
                addresses.insert(id, address);
            } else {
                pending.push(id);
            }
        }
        let mut linked_libraries = Vec::<LinkedLibrary>::new();

        // Iteratively compute addresses and link libraries until we have no unlinked libraries
        // left.
        while !pending.is_empty() {
            let mut deployable = None;
            for (position, &id) in pending.iter().enumerate() {
                let artifact_libraries =
                    self.libraries_for_artifact(&index, id, &libraries, &addresses)?;
                let bytecode = self.link(id, &artifact_libraries)?.bytecode.ok_or_else(|| {
                    LinkerError::LinkingFailed { artifact: id.source.to_string_lossy().into() }
                })?;
                if !bytecode.object.is_unlinked() {
                    deployable = Some((position, id, bytecode));
                    break;
                }
            }
            let Some((position, id, bytecode)) = deployable else {
                return Err(LinkerError::CyclicDependency);
            };
            pending.swap_remove(position);
            let code = bytecode.bytes().ok_or_else(|| LinkerError::LinkingFailed {
                artifact: id.source.to_string_lossy().into(),
            })?;
            let address = sender.create2_from_code(salt, code);
            if linked_libraries.iter().all(|library| library.address != address) {
                linked_libraries.push(LinkedLibrary {
                    id: id.clone(),
                    address,
                    bytecode: code.clone(),
                });
            }
            addresses.insert(id, address);
        }

        let artifact_libraries = self.artifact_libraries(
            &index,
            targets.iter().copied().chain(needed_libraries.iter().copied()),
            &libraries,
            &addresses,
        )?;
        let libs_to_deploy = linked_libraries.iter().map(|lib| lib.bytecode.clone()).collect();
        let library_addresses = self.assigned_library_addresses(&addresses);
        let artifact_addresses = self.artifact_addresses(&addresses);
        let libraries = self.output_libraries(libraries, &addresses);
        Ok(DetailedLinkOutput {
            output: LinkOutput { libraries, library_addresses, libs_to_deploy },
            artifact_libraries,
            artifact_addresses,
            linked_libraries,
        })
    }

    /// Relinks a target while assigning libraries not needed onchain to an isolated deployer.
    pub fn link_with_partition(
        &'a self,
        libraries: Libraries,
        sender: Address,
        mut sender_nonce: u64,
        local_deployer: Address,
        required: &BTreeSet<ArtifactId>,
        target: &'a ArtifactId,
    ) -> Result<(DetailedLinkOutput, Vec<LinkedLibrary>), LinkerError> {
        let index = OnceLock::new();
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());
        let mut needed = BTreeSet::new();
        let mut references = BTreeMap::new();
        self.collect_dependencies(&index, target, &mut needed, &mut references)?;
        self.apply_configured_references(&references, &mut libraries)?;
        let library_keys = self.collect_library_keys(&needed, &libraries)?;
        let mut required_with_dependencies = required.clone();
        for id in required {
            required_with_dependencies.extend(self.dependencies_inner(&index, id)?);
        }
        let mut local_nonce = 0;
        let mut assigned = Vec::new();
        for &id in &needed {
            let (file, name) = self.convert_artifact_id_to_lib_path(id);
            libraries.libs.entry(file).or_default().entry(name).or_insert_with(|| {
                let onchain = required_with_dependencies.contains(id);
                let address = if onchain {
                    let address = sender.create(sender_nonce);
                    sender_nonce += 1;
                    address
                } else {
                    let address = local_deployer.create(local_nonce);
                    local_nonce += 1;
                    address
                };
                assigned.push((id, address, onchain));
                address.to_checksum(None)
            });
        }
        let linked = assigned
            .into_iter()
            .map(|(id, address, onchain)| {
                let bytecode = self.linked_creation_bytecode(id, &libraries)?;
                Ok((LinkedLibrary { id: id.clone(), address, bytecode }, onchain))
            })
            .collect::<Result<Vec<_>, LinkerError>>()?;
        let libs_to_deploy = linked
            .iter()
            .filter(|(_, onchain)| *onchain)
            .map(|(lib, _)| lib.bytecode.clone())
            .collect();
        let local =
            linked.iter().filter(|(_, onchain)| !*onchain).map(|(lib, _)| lib.clone()).collect();
        let linked_libraries = linked.into_iter().map(|(lib, _)| lib).collect();
        let library_addresses = self.library_addresses(&library_keys, &libraries)?;
        let artifact_addresses =
            self.artifact_addresses_from_libraries(needed.iter().copied(), &libraries)?;
        let artifact_libraries = BTreeMap::from([(target.clone(), libraries.clone())]);
        Ok((
            DetailedLinkOutput {
                output: LinkOutput { libraries, library_addresses, libs_to_deploy },
                artifact_libraries,
                artifact_addresses,
                linked_libraries,
            },
            local,
        ))
    }

    /// Relinks a target with CREATE2 onchain assignments and isolated local assignments.
    pub fn link_with_create2_partition(
        &'a self,
        libraries: Libraries,
        create2_deployer: Address,
        salt: B256,
        local_deployer: Address,
        required: &BTreeSet<ArtifactId>,
        target: &'a ArtifactId,
    ) -> Result<(DetailedLinkOutput, Vec<LinkedLibrary>), LinkerError> {
        let index = OnceLock::new();
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());
        let mut needed = BTreeSet::new();
        let mut references = BTreeMap::new();
        self.collect_dependencies(&index, target, &mut needed, &mut references)?;
        self.apply_configured_references(&references, &mut libraries)?;
        let library_keys = self.collect_library_keys(&needed, &libraries)?;
        let mut required_with_dependencies = required.clone();
        for id in required {
            required_with_dependencies.extend(self.dependencies_inner(&index, id)?);
        }
        let mut local_ids = Vec::new();
        let mut required_ids = Vec::new();
        for &id in &needed {
            let (file, name) = self.convert_artifact_id_to_lib_path(id);
            if libraries.libs.get(&file).is_some_and(|libs| libs.contains_key(&name)) {
                continue;
            }
            if required_with_dependencies.contains(id) {
                required_ids.push(id);
            } else {
                let address = local_deployer.create(local_ids.len() as u64);
                libraries.libs.entry(file).or_default().insert(name, address.to_checksum(None));
                local_ids.push((id, address));
            }
        }

        let mut pending = required_ids
            .into_iter()
            .map(|id| {
                let contract = self.link(id, &libraries)?;
                let bytecode = contract.bytecode.ok_or_else(|| LinkerError::LinkingFailed {
                    artifact: id.source.to_string_lossy().into_owned(),
                })?;
                Ok((id, bytecode))
            })
            .collect::<Result<Vec<_>, LinkerError>>()?;
        let mut onchain = Vec::new();
        while !pending.is_empty() {
            let Some(position) = pending.iter().position(|(_, code)| !code.object.is_unlinked())
            else {
                return Err(LinkerError::CyclicDependency);
            };
            let (id, code) = pending.swap_remove(position);
            let bytecode = code.bytes().cloned().ok_or_else(|| LinkerError::LinkingFailed {
                artifact: id.source.to_string_lossy().into_owned(),
            })?;
            let address = create2_deployer.create2_from_code(salt, &bytecode);
            let (file, name) = self.convert_artifact_id_to_lib_path(id);
            libraries
                .libs
                .entry(file.clone())
                .or_default()
                .insert(name.clone(), address.to_checksum(None));
            for (target, pending_code) in &mut pending {
                self.link_bytecode(pending_code.to_mut(), target, &file, &name, address)?;
            }
            onchain.push(LinkedLibrary { id: id.clone(), address, bytecode });
        }

        let local_bytecodes = local_ids
            .iter()
            .map(|(id, _)| self.linked_creation_bytecode(id, &libraries))
            .collect::<Result<Vec<_>, LinkerError>>()?;
        let mut linked_libraries = onchain.clone();
        let local = local_ids
            .into_iter()
            .zip(local_bytecodes)
            .map(|((id, address), bytecode)| LinkedLibrary { id: id.clone(), address, bytecode })
            .collect::<Vec<_>>();
        linked_libraries.extend(local.iter().cloned());
        let libs_to_deploy = onchain.into_iter().map(|lib| lib.bytecode).collect();
        let library_addresses = self.library_addresses(&library_keys, &libraries)?;
        let artifact_addresses =
            self.artifact_addresses_from_libraries(needed.iter().copied(), &libraries)?;
        let artifact_libraries = BTreeMap::from([(target.clone(), libraries.clone())]);
        Ok((
            DetailedLinkOutput {
                output: LinkOutput { libraries, library_addresses, libs_to_deploy },
                artifact_libraries,
                artifact_addresses,
                linked_libraries,
            },
            local,
        ))
    }

    /// Links given artifact with given libraries.
    pub fn link(
        &self,
        target: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<CompactContractBytecodeCow<'a>, LinkerError> {
        let mut contract =
            self.contracts.get(target).ok_or(LinkerError::MissingTargetArtifact)?.clone();
        let libraries = libraries
            .libs
            .iter()
            .flat_map(|(file, libraries)| {
                libraries.iter().map(move |(name, address)| {
                    Ok((
                        file,
                        name,
                        Address::from_str(address).map_err(LinkerError::InvalidAddress)?,
                    ))
                })
            })
            .collect::<Result<Vec<_>, LinkerError>>()?;
        let link = |bytecode: &mut CompactBytecode| -> Result<(), LinkerError> {
            for canonical in [false, true] {
                for &(file, name, address) in &libraries {
                    self.link_bytecode_inner(bytecode, target, file, name, address, canonical)?;
                }
            }
            Ok(())
        };
        if let Some(bytecode) = contract.bytecode.as_mut() {
            link(bytecode.to_mut())?;
        }
        if let Some(deployed_bytecode) =
            contract.deployed_bytecode.as_mut().and_then(|b| b.to_mut().bytecode.as_mut())
        {
            link(deployed_bytecode)?;
        }
        Ok(contract)
    }

    /// Ensures that both initial and deployed bytecode are linked.
    pub fn ensure_linked(
        &self,
        contract: &CompactContractBytecodeCow<'a>,
        target: &ArtifactId,
    ) -> Result<(), LinkerError> {
        if let Some(bytecode) = &contract.bytecode
            && bytecode.object.is_unlinked()
        {
            return Err(LinkerError::LinkingFailed {
                artifact: target.source.to_string_lossy().into(),
            });
        }
        if let Some(deployed_bytecode) = &contract.deployed_bytecode
            && let Some(deployed_bytecode_obj) = &deployed_bytecode.bytecode
            && deployed_bytecode_obj.object.is_unlinked()
        {
            return Err(LinkerError::LinkingFailed {
                artifact: target.source.to_string_lossy().into(),
            });
        }
        Ok(())
    }

    pub fn get_linked_artifacts(
        &self,
        libraries: &Libraries,
    ) -> Result<ArtifactContracts, LinkerError> {
        self.get_linked_artifacts_cow(libraries).map(ArtifactContracts::from_iter)
    }

    pub fn get_linked_artifacts_cow(
        &self,
        libraries: &Libraries,
    ) -> Result<ArtifactContracts<CompactContractBytecodeCow<'a>>, LinkerError> {
        self.get_linked_artifacts_cow_with_artifact_libraries(libraries, &BTreeMap::new())
    }

    pub fn get_linked_artifacts_cow_with_artifact_libraries(
        &self,
        libraries: &Libraries,
        artifact_libraries: &BTreeMap<ArtifactId, Libraries>,
    ) -> Result<ArtifactContracts<CompactContractBytecodeCow<'a>>, LinkerError> {
        self.contracts
            .par_iter()
            .map(|(id, _)| {
                let libraries = artifact_libraries.get(id).unwrap_or(libraries);
                Ok((id.clone(), self.link(id, libraries)?))
            })
            .collect::<Result<_, _>>()
            .map(ArtifactContracts)
    }
}

impl<'a, 'b> Resolver<'a, 'b> {
    /// Creates a resolver that reuses artifact lookups across calls.
    pub const fn new(linker: &'b Linker<'a>) -> Self {
        Self { linker, index: OnceLock::new() }
    }

    /// Returns the resolved addresses of all libraries required by `target`.
    pub fn linked_library_addresses(
        &self,
        target: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<BTreeSet<Address>, LinkerError> {
        self.linker.linked_library_addresses_inner(&self.index, target, libraries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, fixed_bytes, map::HashMap};
    use foundry_compilers::{
        Project, ProjectCompileOutput, ProjectPathsConfig,
        artifacts::BytecodeObject,
        multi::MultiCompiler,
        solc::{Solc, SolcCompiler},
    };
    use semver::Version;

    fn testdata() -> &'static Path {
        static CACHE: OnceLock<PathBuf> = OnceLock::new();
        CACHE.get_or_init(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").canonicalize().unwrap()
        })
    }

    #[must_use]
    struct LinkerTest {
        project: Project,
        output: ProjectCompileOutput,
        dependency_assertions: HashMap<&'static str, Vec<(&'static str, Address)>>,
    }

    impl LinkerTest {
        fn new(path: &Path, strip_prefixes: bool) -> Self {
            assert!(path.exists(), "Path {path:?} does not exist");
            let paths = ProjectPathsConfig::builder()
                .root(testdata())
                .lib(testdata().join("lib"))
                .sources(path)
                .tests(path)
                .build()
                .unwrap();

            let solc = Solc::find_or_install(&Version::new(0, 8, 18)).unwrap();
            let project = Project::builder()
                .paths(paths)
                .ephemeral()
                .no_artifacts()
                .build(MultiCompiler { solc: Some(SolcCompiler::Specific(solc)), vyper: None })
                .unwrap();

            let mut output = project.compile().unwrap();

            if strip_prefixes {
                output = output.with_stripped_file_prefixes(project.root());
            }

            Self { project, output, dependency_assertions: HashMap::default() }
        }

        fn assert_dependencies(
            mut self,
            artifact_id: &'static str,
            deps: &[(&'static str, Address)],
        ) -> Self {
            self.dependency_assertions.insert(artifact_id, deps.to_vec());
            self
        }

        fn test_with_sender_and_nonce(self, sender: Address, initial_nonce: u64) {
            let linker = Linker::new(self.project.root(), self.output.artifact_ids().collect());
            for (id, identifier) in self.iter_linking_targets(&linker) {
                let output = linker
                    .link_with_nonce_or_address(Default::default(), sender, initial_nonce, [id])
                    .expect("Linking failed");
                self.validate_assertions(identifier, output);
            }
        }

        fn test_with_create2(self, sender: Address, salt: B256) {
            let linker = Linker::new(self.project.root(), self.output.artifact_ids().collect());
            for (id, identifier) in self.iter_linking_targets(&linker) {
                let output = linker
                    .link_with_create2(Default::default(), sender, salt, [id])
                    .expect("Linking failed");
                self.validate_assertions(identifier, output);
            }
        }

        fn iter_linking_targets<'a>(
            &'a self,
            linker: &'a Linker<'_>,
        ) -> impl Iterator<Item = (&'a ArtifactId, String)> + 'a {
            self.sanity_check(linker);
            linker.contracts.keys().filter_map(move |id| {
                // If we didn't strip paths, artifacts will have absolute paths.
                // That's expected and we want to ensure that only `libraries` object has relative
                // paths, artifacts should be kept as is.
                let source = id
                    .source
                    .strip_prefix(self.project.root())
                    .unwrap_or(&id.source)
                    .to_string_lossy();
                let identifier = format!("{source}:{}", id.name);

                // Skip test utils as they always have no dependencies.
                if identifier.contains("utils/") {
                    return None;
                }

                Some((id, identifier))
            })
        }

        fn sanity_check(&self, linker: &Linker<'_>) {
            assert!(!self.dependency_assertions.is_empty(), "Dependency assertions are empty");
            assert!(!linker.contracts.is_empty(), "Linker contracts are empty");
        }

        fn validate_assertions(&self, identifier: String, output: LinkOutput) {
            let LinkOutput { libs_to_deploy, libraries, .. } = output;

            let assertions = self
                .dependency_assertions
                .get(identifier.as_str())
                .unwrap_or_else(|| panic!("Unexpected artifact: {identifier}"));

            assert_eq!(
                libs_to_deploy.len(),
                assertions.len(),
                "artifact {identifier} has more/less dependencies than expected ({} vs {}): {:#?}",
                libs_to_deploy.len(),
                assertions.len(),
                libs_to_deploy
            );

            for &(dep_identifier, address) in assertions {
                let (file, name) = dep_identifier.split_once(':').unwrap();
                if let Some(lib_address) =
                    libraries.libs.get(Path::new(file)).and_then(|libs| libs.get(name))
                {
                    assert_eq!(
                        lib_address.parse::<Address>().unwrap(),
                        address,
                        "incorrect library address for dependency {dep_identifier} of {identifier}"
                    );
                } else {
                    panic!("Library {dep_identifier} not found");
                }
            }
        }
    }

    fn link_test(path: impl AsRef<Path>, mut test_fn: impl FnMut(LinkerTest)) {
        fn link_test(path: &Path, test_fn: &mut dyn FnMut(LinkerTest)) {
            test_fn(LinkerTest::new(path, true));
            test_fn(LinkerTest::new(path, false));
        }
        link_test(path.as_ref(), &mut test_fn);
    }

    #[test]
    #[should_panic = "assertions are empty"]
    fn no_assertions() {
        link_test(testdata().join("default/linking/simple"), |linker| {
            linker.test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    #[should_panic = "does not exist"]
    fn unknown_path() {
        link_test("doesnotexist", |linker| {
            linker
                .assert_dependencies("a:b", &[])
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    fn link_simple() {
        link_test(testdata().join("default/linking/simple"), |linker| {
            linker
                .assert_dependencies("default/linking/simple/Simple.t.sol:Lib", &[])
                .assert_dependencies(
                    "default/linking/simple/Simple.t.sol:LibraryConsumer",
                    &[(
                        "default/linking/simple/Simple.t.sol:Lib",
                        address!("0x5a443704dd4b594b382c22a083e2bd3090a6fef3"),
                    )],
                )
                .assert_dependencies(
                    "default/linking/simple/Simple.t.sol:SimpleLibraryLinkingTest",
                    &[(
                        "default/linking/simple/Simple.t.sol:Lib",
                        address!("0x5a443704dd4b594b382c22a083e2bd3090a6fef3"),
                    )],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    fn link_nested() {
        link_test(testdata().join("default/linking/nested"), |linker| {
            linker
                .assert_dependencies("default/linking/nested/Nested.t.sol:Lib", &[])
                .assert_dependencies(
                    "default/linking/nested/Nested.t.sol:NestedLib",
                    &[(
                        "default/linking/nested/Nested.t.sol:Lib",
                        address!("0x5a443704dd4b594b382c22a083e2bd3090a6fef3"),
                    )],
                )
                .assert_dependencies(
                    "default/linking/nested/Nested.t.sol:LibraryConsumer",
                    &[
                        // Lib shows up here twice, because the linker sees it twice, but it should
                        // have the same address and nonce.
                        (
                            "default/linking/nested/Nested.t.sol:Lib",
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "default/linking/nested/Nested.t.sol:NestedLib",
                            Address::from_str("0x47e9Fbef8C83A1714F1951F142132E6e90F5fa5D")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "default/linking/nested/Nested.t.sol:NestedLibraryLinkingTest",
                    &[
                        (
                            "default/linking/nested/Nested.t.sol:Lib",
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "default/linking/nested/Nested.t.sol:NestedLib",
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                    ],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    fn link_duplicate() {
        link_test(testdata().join("default/linking/duplicate"), |linker| {
            linker
                .assert_dependencies("default/linking/duplicate/Duplicate.t.sol:A", &[])
                .assert_dependencies("default/linking/duplicate/Duplicate.t.sol:B", &[])
                .assert_dependencies(
                    "default/linking/duplicate/Duplicate.t.sol:C",
                    &[(
                        "default/linking/duplicate/Duplicate.t.sol:A",
                        address!("0x5a443704dd4b594b382c22a083e2bd3090a6fef3"),
                    )],
                )
                .assert_dependencies(
                    "default/linking/duplicate/Duplicate.t.sol:D",
                    &[(
                        "default/linking/duplicate/Duplicate.t.sol:B",
                        address!("0x5a443704dd4b594b382c22a083e2bd3090a6fef3"),
                    )],
                )
                .assert_dependencies(
                    "default/linking/duplicate/Duplicate.t.sol:E",
                    &[
                        (
                            "default/linking/duplicate/Duplicate.t.sol:A",
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:C",
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "default/linking/duplicate/Duplicate.t.sol:LibraryConsumer",
                    &[
                        (
                            "default/linking/duplicate/Duplicate.t.sol:A",
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:B",
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:C",
                            Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:D",
                            Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:E",
                            Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "default/linking/duplicate/Duplicate.t.sol:DuplicateLibraryLinkingTest",
                    &[
                        (
                            "default/linking/duplicate/Duplicate.t.sol:A",
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:B",
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:C",
                            Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:D",
                            Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782")
                                .unwrap(),
                        ),
                        (
                            "default/linking/duplicate/Duplicate.t.sol:E",
                            Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f")
                                .unwrap(),
                        ),
                    ],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    fn link_cycle() {
        link_test(testdata().join("default/linking/cycle"), |linker| {
            linker
                .assert_dependencies(
                    "default/linking/cycle/Cycle.t.sol:Foo",
                    &[
                        (
                            "default/linking/cycle/Cycle.t.sol:Foo",
                            Address::from_str("0x47e9Fbef8C83A1714F1951F142132E6e90F5fa5D")
                                .unwrap(),
                        ),
                        (
                            "default/linking/cycle/Cycle.t.sol:Bar",
                            Address::from_str("0x5a443704dd4B594B382c22a083e2BD3090A6feF3")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "default/linking/cycle/Cycle.t.sol:Bar",
                    &[
                        (
                            "default/linking/cycle/Cycle.t.sol:Foo",
                            Address::from_str("0x47e9Fbef8C83A1714F1951F142132E6e90F5fa5D")
                                .unwrap(),
                        ),
                        (
                            "default/linking/cycle/Cycle.t.sol:Bar",
                            Address::from_str("0x5a443704dd4B594B382c22a083e2BD3090A6feF3")
                                .unwrap(),
                        ),
                    ],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    #[ignore = "addresses depend on testdata utils internals for some reason"]
    fn link_create2_nested() {
        link_test(testdata().join("default/linking/nested"), |linker| {
            linker
                .assert_dependencies("default/linking/nested/Nested.t.sol:Lib", &[])
                .assert_dependencies(
                    "default/linking/nested/Nested.t.sol:NestedLib",
                    &[(
                        "default/linking/nested/Nested.t.sol:Lib",
                        address!("0x773253227cce756e50c3993ec6366b3ec27786f9"),
                    )],
                )
                .assert_dependencies(
                    "default/linking/nested/Nested.t.sol:LibraryConsumer",
                    &[
                        // Lib shows up here twice, because the linker sees it twice, but it should
                        // have the same address and nonce.
                        (
                            "default/linking/nested/Nested.t.sol:Lib",
                            Address::from_str("0x773253227cce756e50c3993ec6366b3ec27786f9")
                                .unwrap(),
                        ),
                        (
                            "default/linking/nested/Nested.t.sol:NestedLib",
                            Address::from_str("0xac231df03403867b05d092c26fc91b6b83f4bebe")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "default/linking/nested/Nested.t.sol:NestedLibraryLinkingTest",
                    &[
                        (
                            "default/linking/nested/Nested.t.sol:Lib",
                            Address::from_str("0x773253227cce756e50c3993ec6366b3ec27786f9")
                                .unwrap(),
                        ),
                        (
                            "default/linking/nested/Nested.t.sol:NestedLib",
                            Address::from_str("0xac231df03403867b05d092c26fc91b6b83f4bebe")
                                .unwrap(),
                        ),
                    ],
                )
                .test_with_create2(
                    Address::default(),
                    fixed_bytes!(
                        "19bf59b7b67ae8edcbc6e53616080f61fa99285c061450ad601b0bc40c9adfc9"
                    ),
                );
        });
    }

    #[test]
    fn link_samefile_union() {
        link_test(testdata().join("default/linking/samefile_union"), |linker| {
            linker
                .assert_dependencies("default/linking/samefile_union/Libs.sol:LInit", &[])
                .assert_dependencies("default/linking/samefile_union/Libs.sol:LRun", &[])
                .assert_dependencies(
                    "default/linking/samefile_union/SameFileUnion.t.sol:UsesBoth",
                    &[
                        (
                            "default/linking/samefile_union/Libs.sol:LInit",
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "default/linking/samefile_union/Libs.sol:LRun",
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                    ],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    fn link_create2_multiple_targets_deduplicates_shared_dependencies() {
        let test = LinkerTest::new(&testdata().join("default/linking/simple"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let consumer = linker.contracts.keys().find(|id| id.name == "LibraryConsumer").unwrap();
        let test_contract =
            linker.contracts.keys().find(|id| id.name == "SimpleLibraryLinkingTest").unwrap();

        let output = linker
            .link_with_create2(
                Libraries::default(),
                Address::ZERO,
                B256::with_last_byte(1),
                [consumer, test_contract],
            )
            .unwrap();

        assert_eq!(output.libs_to_deploy.len(), 1);
        assert_eq!(output.libraries.libs.values().map(BTreeMap::len).sum::<usize>(), 1);

        let linked_address = output
            .libraries
            .libs
            .values()
            .flat_map(|libraries| libraries.values())
            .next()
            .unwrap()
            .parse::<Address>()
            .unwrap();
        for target in [consumer, test_contract] {
            let bytecode = linker.link(target, &output.libraries).unwrap().bytecode.unwrap();
            assert!(
                bytecode
                    .bytes()
                    .unwrap()
                    .windows(Address::len_bytes())
                    .any(|window| { window == linked_address.as_slice() }),
                "{} was not linked to {linked_address}",
                target.name
            );
        }
    }

    #[test]
    fn detailed_linking_includes_transitive_library_addresses() {
        let test = LinkerTest::new(&testdata().join("default/linking/nested"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let consumer = linker.contracts.keys().find(|id| id.name == "LibraryConsumer").unwrap();
        let resolver = Resolver::new(&linker);

        let create2 = linker
            .link_with_create2_detailed(Libraries::default(), Address::ZERO, B256::ZERO, [consumer])
            .unwrap();
        let libraries = create2.artifact_libraries.get(consumer).unwrap();
        assert_eq!(resolver.linked_library_addresses(consumer, libraries).unwrap().len(), 2);

        let nonce = linker
            .link_with_nonce_or_address_detailed(Libraries::default(), Address::ZERO, 0, [consumer])
            .unwrap();
        let libraries = nonce.artifact_libraries.get(consumer).unwrap();
        assert_eq!(resolver.linked_library_addresses(consumer, libraries).unwrap().len(), 2);
    }

    #[test]
    fn linking_preserves_transitive_profile_identity() {
        let test = LinkerTest::new(&testdata().join("default/linking/profile_nested"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let find = |name| {
            linker
                .contracts
                .iter()
                .find(|(id, _)| id.name == name)
                .map(|(id, contract)| (id.clone(), contract.clone()))
                .unwrap()
        };
        let (inner_id, inner) = find("Inner");
        let (outer_id, outer) = find("Outer");
        let (consumer_id, consumer) = find("Consumer");

        let mut contracts = linker.contracts.clone();
        let mut other_inner_id = inner_id.clone();
        other_inner_id.build_id = "other".to_string();
        other_inner_id.profile = "other".to_string();
        contracts.insert(other_inner_id.clone(), inner);
        let mut other_outer_id = outer_id.clone();
        other_outer_id.build_id = "other".to_string();
        other_outer_id.profile = "other".to_string();
        contracts.insert(other_outer_id.clone(), outer);
        let mut other_consumer_id = consumer_id.clone();
        other_consumer_id.build_id = "other".to_string();
        other_consumer_id.profile = "other".to_string();
        contracts.insert(other_consumer_id.clone(), consumer);

        for id in [&other_inner_id, &other_outer_id] {
            let bytecode = contracts.get_mut(id).unwrap().bytecode.as_mut().unwrap().to_mut();
            match &mut bytecode.object {
                BytecodeObject::Bytecode(bytes) => {
                    let mut distinct = bytes.to_vec();
                    distinct.push(0);
                    *bytes = distinct.into();
                }
                BytecodeObject::Unlinked(code) => code.push_str("00"),
            }
        }

        let linker = Linker::new(test.project.root(), contracts);
        let consumers = [&consumer_id, &other_consumer_id];
        let profiles = [
            (&consumer_id, &outer_id, &inner_id),
            (&other_consumer_id, &other_outer_id, &other_inner_id),
        ];
        let assert_identity = |output: &DetailedLinkOutput| {
            assert_eq!(output.artifact_addresses.len(), 4);
            assert_eq!(
                output.output.libs_to_deploy,
                output
                    .linked_libraries
                    .iter()
                    .map(|library| library.bytecode.clone())
                    .collect::<Vec<_>>()
            );

            for linked in &output.linked_libraries {
                assert_eq!(output.artifact_addresses[&linked.id], linked.address);
                assert_eq!(
                    linker
                        .linked_creation_bytecode(
                            &linked.id,
                            &output.artifact_libraries[&linked.id],
                        )
                        .unwrap(),
                    linked.bytecode
                );
            }

            for (index, &(consumer, outer, inner)) in profiles.iter().enumerate() {
                let sibling_outer = profiles[1 - index].1;
                let sibling_inner = profiles[1 - index].2;
                let outer_address = output.artifact_addresses[outer];
                let inner_address = output.artifact_addresses[inner];

                let consumer_bytecode = linker
                    .link(consumer, &output.artifact_libraries[consumer])
                    .unwrap()
                    .get_bytecode_bytes()
                    .unwrap()
                    .into_owned();
                assert!(
                    consumer_bytecode
                        .windows(Address::len_bytes())
                        .any(|window| { window == outer_address.as_slice() })
                );
                assert!(!consumer_bytecode.windows(Address::len_bytes()).any(|window| {
                    window == output.artifact_addresses[sibling_outer].as_slice()
                }));

                let outer_bytecode = &output
                    .linked_libraries
                    .iter()
                    .find(|library| library.id == *outer)
                    .unwrap()
                    .bytecode;
                assert!(
                    outer_bytecode
                        .windows(Address::len_bytes())
                        .any(|window| window == inner_address.as_slice())
                );
                assert!(!outer_bytecode.windows(Address::len_bytes()).any(|window| {
                    window == output.artifact_addresses[sibling_inner].as_slice()
                }));
            }
        };

        let sender = address!("1000000000000000000000000000000000000000");
        let nonce = 7;
        let output = linker
            .link_with_nonce_or_address_detailed(Libraries::default(), sender, nonce, consumers)
            .unwrap();
        assert_identity(&output);
        for (index, library) in output.linked_libraries.iter().enumerate() {
            assert_eq!(library.address, sender.create(nonce + index as u64));
        }

        let salt = B256::with_last_byte(1);
        let output = linker
            .link_with_create2_detailed(Libraries::default(), sender, salt, consumers)
            .unwrap();
        assert_identity(&output);
        for library in &output.linked_libraries {
            assert_eq!(library.address, sender.create2_from_code(salt, &library.bytecode));
        }
    }

    #[test]
    fn linking_handles_library_key_collisions_across_profiles() {
        let test = LinkerTest::new(&testdata().join("default/linking/simple"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let (library_id, library) = linker
            .contracts
            .iter()
            .find(|(id, _)| id.name == "Lib")
            .map(|(id, contract)| (id.clone(), contract.clone()))
            .unwrap();
        let (consumer_id, consumer) = linker
            .contracts
            .iter()
            .find(|(id, _)| id.name == "LibraryConsumer")
            .map(|(id, contract)| (id.clone(), contract.clone()))
            .unwrap();

        let mut contracts = linker.contracts.clone();
        let mut other_library_id = library_id.clone();
        other_library_id.build_id = "other".to_string();
        other_library_id.profile = "other".to_string();
        contracts.insert(other_library_id.clone(), library);
        let mut other_consumer_id = consumer_id.clone();
        other_consumer_id.build_id = "other".to_string();
        other_consumer_id.profile = "other".to_string();
        contracts.insert(other_consumer_id.clone(), consumer);

        let linker = Linker::new(test.project.root(), contracts.clone());
        let detailed = linker
            .link_with_create2_detailed(
                Libraries::default(),
                Address::ZERO,
                B256::ZERO,
                [&consumer_id, &other_consumer_id],
            )
            .unwrap();
        assert_eq!(detailed.linked_libraries.len(), 1);
        assert_eq!(detailed.artifact_addresses.len(), 2);
        assert_eq!(detailed.artifact_addresses[&library_id], detailed.linked_libraries[0].address);
        assert_eq!(
            detailed.artifact_addresses[&other_library_id],
            detailed.linked_libraries[0].address
        );

        let output = linker
            .link_with_create2(
                Libraries::default(),
                Address::ZERO,
                B256::ZERO,
                [&consumer_id, &other_consumer_id],
            )
            .unwrap();
        assert_eq!(output.libs_to_deploy.len(), 1);

        let Err(err) = linker.link_with_nonce_or_address(
            Libraries::default(),
            Address::ZERO,
            0,
            [&consumer_id, &other_consumer_id],
        ) else {
            panic!("expected conflicting library artifacts");
        };
        assert!(matches!(err, LinkerError::ConflictingLibraryArtifacts { .. }));

        let other_library = contracts.get_mut(&other_library_id).unwrap();
        let bytecode = other_library.bytecode.as_mut().unwrap().to_mut();
        let mut bytes = bytecode.object.as_bytes().unwrap().to_vec();
        bytes.push(0);
        bytecode.object = BytecodeObject::Bytecode(bytes.into());

        let linker = Linker::new(test.project.root(), contracts);
        let output = linker
            .link_with_create2_detailed(
                Libraries::default(),
                Address::ZERO,
                B256::ZERO,
                [&consumer_id, &other_consumer_id],
            )
            .unwrap();
        assert_eq!(output.linked_libraries.len(), 2);
        assert_ne!(output.linked_libraries[0].address, output.linked_libraries[1].address);
        for (target, library) in
            [(&consumer_id, &library_id), (&other_consumer_id, &other_library_id)]
        {
            let expected = output
                .linked_libraries
                .iter()
                .find(|linked| linked.id == *library)
                .unwrap()
                .address;
            let (file, name) = linker.convert_artifact_id_to_lib_path(library);
            let actual =
                output.artifact_libraries[target].libs[&file][&name].parse::<Address>().unwrap();
            assert_eq!(actual, expected);
        }

        let Err(err) = linker.link_with_create2(
            Libraries::default(),
            Address::ZERO,
            B256::ZERO,
            [&consumer_id, &other_consumer_id],
        ) else {
            panic!("expected conflicting library artifacts");
        };
        assert!(matches!(err, LinkerError::ConflictingLibraryArtifacts { .. }));

        let output = linker
            .link_with_nonce_or_address_detailed(
                Libraries::default(),
                Address::ZERO,
                0,
                [&consumer_id, &other_consumer_id],
            )
            .unwrap();
        assert_eq!(output.linked_libraries.len(), 2);
        assert_ne!(output.linked_libraries[0].address, output.linked_libraries[1].address);
        for (target, library) in
            [(&consumer_id, &library_id), (&other_consumer_id, &other_library_id)]
        {
            let expected = output
                .linked_libraries
                .iter()
                .find(|linked| linked.id == *library)
                .unwrap()
                .address;
            let (file, name) = linker.convert_artifact_id_to_lib_path(library);
            let actual =
                output.artifact_libraries[target].libs[&file][&name].parse::<Address>().unwrap();
            assert_eq!(actual, expected);
        }

        let Err(err) = linker.link_with_nonce_or_address(
            Libraries::default(),
            Address::ZERO,
            0,
            [&consumer_id, &other_consumer_id],
        ) else {
            panic!("expected conflicting library artifacts");
        };
        assert!(matches!(err, LinkerError::ConflictingLibraryArtifacts { .. }));

        let configured_address = address!("0000000000000000000000000000000000000001");
        let (file, name) = linker.convert_artifact_id_to_lib_path(&library_id);
        let mut libraries = Libraries::default();
        libraries.libs.entry(file).or_default().insert(name, configured_address.to_checksum(None));

        let output = linker
            .link_with_create2(
                libraries.clone(),
                Address::ZERO,
                B256::ZERO,
                [&consumer_id, &other_consumer_id],
            )
            .unwrap();
        assert!(output.libs_to_deploy.is_empty());
        assert_eq!(output.library_addresses, [configured_address]);

        let output = linker
            .link_with_nonce_or_address(
                libraries,
                Address::ZERO,
                0,
                [&consumer_id, &other_consumer_id],
            )
            .unwrap();
        assert!(output.libs_to_deploy.is_empty());
        assert_eq!(output.library_addresses, [configured_address]);
    }

    #[test]
    fn linking_resolves_same_version_library_from_target_build() {
        let test = LinkerTest::new(&testdata().join("default/linking/simple"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let (library_id, library) = linker
            .contracts
            .iter()
            .find(|(id, _)| id.name == "Lib")
            .map(|(id, contract)| (id.clone(), contract.clone()))
            .unwrap();
        let (consumer_id, consumer) = linker
            .contracts
            .iter()
            .find(|(id, _)| id.name == "LibraryConsumer")
            .map(|(id, contract)| (id.clone(), contract.clone()))
            .unwrap();

        let mut contracts = linker.contracts.clone();
        let mut other_library_id = library_id;
        other_library_id.build_id = "other".to_string();
        other_library_id.profile = "other".to_string();
        contracts.insert(other_library_id, library);
        let mut ambiguous_consumer_id = consumer_id.clone();
        ambiguous_consumer_id.build_id = "ambiguous".to_string();
        ambiguous_consumer_id.profile = "ambiguous".to_string();
        contracts.insert(ambiguous_consumer_id.clone(), consumer);

        let linker = Linker::new(test.project.root(), contracts);
        linker
            .link_with_create2(Libraries::default(), Address::ZERO, B256::ZERO, [&consumer_id])
            .unwrap();

        let Err(err) = linker.link_with_create2(
            Libraries::default(),
            Address::ZERO,
            B256::ZERO,
            [&ambiguous_consumer_id],
        ) else {
            panic!("expected conflicting library artifacts");
        };
        assert!(matches!(err, LinkerError::ConflictingLibraryArtifacts { .. }));
    }

    #[test]
    fn link_output_excludes_unreferenced_configured_libraries() {
        let test = LinkerTest::new(&testdata().join("default/linking/simple"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let consumer = linker.contracts.keys().find(|id| id.name == "LibraryConsumer").unwrap();
        let unrelated = address!("0000000000000000000000000000000000000001");
        let mut libraries = Libraries::default();
        libraries
            .libs
            .entry("src/Unrelated.sol".into())
            .or_default()
            .insert("Unrelated".to_string(), unrelated.to_checksum(None));

        let output =
            linker.link_with_create2(libraries, Address::ZERO, B256::ZERO, [consumer]).unwrap();

        assert_eq!(output.library_addresses.len(), 1);
        assert!(!output.library_addresses.contains(&unrelated));
    }

    #[test]
    fn exact_artifact_match_uses_configured_library_alias() {
        let test = LinkerTest::new(&testdata().join("default/linking/simple"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let mut contracts = linker.contracts.clone();
        let (library_id, library) = contracts
            .iter()
            .find(|(id, _)| id.name == "Lib")
            .map(|(id, contract)| (id.clone(), contract.clone()))
            .unwrap();
        let mut alias_id = library_id.clone();
        alias_id.source = "./default/linking/simple/Simple.t.sol".into();
        contracts.insert(alias_id, library);
        let linker = Linker::new(test.project.root(), contracts);
        let consumer = linker.contracts.keys().find(|id| id.name == "LibraryConsumer").unwrap();
        let configured = address!("0000000000000000000000000000000000000001");
        let alias = PathBuf::from("./default/linking/simple/Simple.t.sol");
        let generated_key = PathBuf::from("default/linking/simple/Simple.t.sol");
        let mut libraries = Libraries::default();
        libraries
            .libs
            .entry(alias.clone())
            .or_default()
            .insert("Lib".to_string(), configured.to_checksum(None));

        let output =
            linker.link_with_nonce_or_address(libraries, Address::ZERO, 1, [consumer]).unwrap();
        let bytecode = linker.link(consumer, &output.libraries).unwrap().bytecode.unwrap();
        let bytecode = bytecode.bytes().unwrap();

        assert!(output.libs_to_deploy.is_empty());
        assert_eq!(output.library_addresses, [configured]);
        assert!(
            bytecode.windows(Address::len_bytes()).any(|window| window == configured.as_slice())
        );
        assert_eq!(output.libraries.libs.len(), 2);
        assert!(output.libraries.libs.contains_key(&alias));
        assert!(output.libraries.libs.contains_key(&generated_key));

        let exact = address!("0000000000000000000000000000000000000002");
        let references = BTreeMap::from([(&library_id, BTreeSet::from([alias.clone()]))]);
        let mut libraries = output.libraries;
        libraries.libs.get_mut(&alias).unwrap().insert("Lib".into(), exact.to_checksum(None));
        linker.apply_configured_references(&references, &mut libraries).unwrap();
        assert_eq!(libraries.libs[&generated_key]["Lib"], exact.to_checksum(None));

        libraries
            .libs
            .get_mut(&generated_key)
            .unwrap()
            .insert("Lib".into(), configured.to_checksum(None));
        let fallback = PathBuf::from("default/linking/simple/../simple/Simple.t.sol");
        let references = BTreeMap::from([(&library_id, BTreeSet::from([fallback]))]);
        let err = linker.apply_configured_references(&references, &mut libraries).unwrap_err();
        assert!(matches!(err, LinkerError::ConflictingLibraryArtifacts { .. }));
    }

    #[test]
    fn partition_with_nonce_assigns_required_and_local_libraries() {
        let test = LinkerTest::new(&testdata().join("default/linking/samefile_union"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let find = |name| linker.contracts.keys().find(|id| id.name == name).unwrap();
        let (target, required_id) = (find("UsesBoth"), find("LInit").clone());
        let sender = address!("0x1000000000000000000000000000000000000000");
        let local_deployer = address!("0x2000000000000000000000000000000000000000");
        let required = BTreeSet::from([required_id.clone()]);
        let (detailed, local) = linker
            .link_with_partition(Libraries::default(), sender, 7, local_deployer, &required, target)
            .unwrap();

        assert_eq!(detailed.linked_libraries.len(), 2);
        assert_eq!(detailed.output.libs_to_deploy.len(), 1);
        assert_eq!(local.len(), 1);
        assert_eq!(
            detailed.linked_libraries.iter().find(|lib| lib.id == required_id).unwrap().address,
            sender.create(7)
        );
        assert_eq!(local[0].address, local_deployer.create(0));
        linker
            .ensure_linked(&linker.link(target, &detailed.output.libraries).unwrap(), target)
            .unwrap();
        for library in &detailed.linked_libraries {
            assert!(!library.bytecode.is_empty());
        }

        let mut configured = Libraries::default();
        let (file, name) = linker.convert_artifact_id_to_lib_path(find("LRun"));
        configured.libs.entry(file).or_default().insert(name, Address::ZERO.to_checksum(None));
        let (configured, local) = linker
            .link_with_partition(configured, sender, 7, local_deployer, &required, target)
            .unwrap();
        assert_eq!(configured.linked_libraries.len(), 1);
        assert!(local.is_empty());
    }

    #[test]
    fn partition_with_create2_assigns_required_and_local_libraries() {
        let test = LinkerTest::new(&testdata().join("default/linking/samefile_union"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let find = |name| linker.contracts.keys().find(|id| id.name == name).unwrap();
        let (target, required_id) = (find("UsesBoth"), find("LInit").clone());
        let required = BTreeSet::from([required_id.clone()]);
        let deployer = address!("0x3000000000000000000000000000000000000000");
        let local_deployer = address!("0x4000000000000000000000000000000000000000");
        let salt = fixed_bytes!("19bf59b7b67ae8edcbc6e53616080f61fa99285c061450ad601b0bc40c9adfc9");
        let (detailed, local) = linker
            .link_with_create2_partition(
                Libraries::default(),
                deployer,
                salt,
                local_deployer,
                &required,
                target,
            )
            .unwrap();

        assert_eq!(detailed.output.libs_to_deploy.len(), 1);
        assert_eq!(local.len(), 1);
        let onchain =
            detailed.linked_libraries.iter().find(|library| library.id == required_id).unwrap();
        assert_eq!(onchain.address, deployer.create2_from_code(salt, &onchain.bytecode));
        assert_eq!(detailed.output.libs_to_deploy[0], onchain.bytecode);
        assert_eq!(local[0].address, local_deployer.create(0));
        linker
            .ensure_linked(&linker.link(target, &detailed.output.libraries).unwrap(), target)
            .unwrap();
    }

    #[test]
    fn partition_with_create2_keeps_transitive_order_deterministic() {
        let test = LinkerTest::new(&testdata().join("default/linking/nested"), true);
        let linker = Linker::new(test.project.root(), test.output.artifact_ids().collect());
        let find = |name| linker.contracts.keys().find(|id| id.name == name).unwrap();
        let target = find("LibraryConsumer");
        let required = BTreeSet::from([find("NestedLib").clone()]);
        let deployer = address!("0x3000000000000000000000000000000000000000");
        let local_deployer = address!("0x4000000000000000000000000000000000000000");
        let salt = fixed_bytes!("19bf59b7b67ae8edcbc6e53616080f61fa99285c061450ad601b0bc40c9adfc9");
        let link = || {
            linker
                .link_with_create2_partition(
                    Libraries::default(),
                    deployer,
                    salt,
                    local_deployer,
                    &required,
                    target,
                )
                .unwrap()
        };
        let (first, local) = link();
        let (second, _) = link();

        assert!(local.is_empty());
        assert_eq!(first.output.libs_to_deploy.len(), 2);
        assert_eq!(
            first.linked_libraries.iter().map(|lib| (&lib.id, lib.address)).collect::<Vec<_>>(),
            second.linked_libraries.iter().map(|lib| (&lib.id, lib.address)).collect::<Vec<_>>()
        );
        assert_eq!(first.linked_libraries[0].id.name, "Lib");
        assert_eq!(first.linked_libraries[1].id.name, "NestedLib");
        for library in &first.linked_libraries {
            assert_eq!(library.address, deployer.create2_from_code(salt, &library.bytecode));
        }
        linker
            .ensure_linked(&linker.link(target, &first.output.libraries).unwrap(), target)
            .unwrap();
    }

    #[test]
    fn linking_failure() {
        let linker = LinkerTest::new(&testdata().join("default/linking/simple"), true);
        let linker_instance =
            Linker::new(linker.project.root(), linker.output.artifact_ids().collect());

        // Create a libraries object with an incorrect library name that won't match any references
        let mut libraries = Libraries::default();
        libraries.libs.entry("default/linking/simple/Simple.t.sol".into()).or_default().insert(
            "NonExistentLib".to_string(),
            "0x5a443704dd4b594b382c22a083e2bd3090a6fef3".to_string(),
        );

        // Try to link the LibraryConsumer contract with incorrect library
        let artifact_id = linker_instance
            .contracts
            .keys()
            .find(|id| id.name == "LibraryConsumer")
            .expect("LibraryConsumer contract not found");

        let contract = linker_instance.contracts.get(artifact_id).unwrap();

        // Verify that the artifact has unlinked bytecode
        assert!(
            linker_instance.ensure_linked(contract, artifact_id).is_err(),
            "Expected artifact to have unlinked bytecode"
        );
    }
}
