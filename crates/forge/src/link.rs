use alloy_primitives::{Address, Bytes};
use foundry_compilers::{
    artifacts::{CompactContractBytecode, Libraries},
    contracts::ArtifactContracts,
    Artifact, ArtifactId,
};
use semver::Version;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Errors that can occur during linking.
#[derive(Debug, thiserror::Error)]
pub enum LinkerError {
    #[error("wasn't able to find artifact for library {name} at {file}")]
    MissingLibraryArtifact { file: String, name: String },
    #[error("target artifact is not present in provided artifacts set")]
    MissingTargetArtifact,
    #[error(transparent)]
    InvalidAddress(<Address as std::str::FromStr>::Err),
}

pub struct Linker {
    /// Root of the project, used to determine whether artifact/library path can be stripped.
    pub root: PathBuf,
    /// Compilation artifacts.
    pub contracts: ArtifactContracts,
}

/// Output of the `link_with_nonce_or_address`
pub struct LinkOutput {
    /// Resolved library addresses. Contains both user-provided and newly deployed libraries.
    /// It will always contain library paths with stripped path prefixes.
    pub libraries: Libraries,
    /// Vector of libraries that need to be deployed from sender address.
    /// The order in which they appear in the vector is the order in which they should be deployed.
    pub libs_to_deploy: Vec<Bytes>,
}

impl Linker {
    pub fn new(root: impl Into<PathBuf>, contracts: ArtifactContracts) -> Self {
        Linker { root: root.into(), contracts }
    }

    /// Helper method to convert [ArtifactId] to the format in which libraries are stored in
    /// [Libraries] object.
    ///
    /// Strips project root path from source file path.
    fn convert_artifact_id_to_lib_path(&self, id: &ArtifactId) -> (PathBuf, String) {
        let path = id.source.strip_prefix(self.root.as_path()).unwrap_or(&id.source);
        // name is either {LibName} or {LibName}.{version}
        let name = id.name.split('.').next().unwrap();

        (path.to_path_buf(), name.to_owned())
    }

    /// Finds an [ArtifactId] object in the given [ArtifactContracts] keys which corresponds to the
    /// library path in the form of "./path/to/Lib.sol:Lib"
    ///
    /// Optionally accepts solc version, and if present, only compares artifacts with given version.
    fn find_artifact_id_by_library_path<'a>(
        &'a self,
        file: &str,
        name: &str,
        version: Option<&Version>,
    ) -> Option<&'a ArtifactId> {
        for id in self.contracts.keys() {
            if let Some(version) = version {
                if id.version != *version {
                    continue;
                }
            }
            let (artifact_path, artifact_name) = self.convert_artifact_id_to_lib_path(id);

            if artifact_name == *name && artifact_path == Path::new(file) {
                return Some(id);
            }
        }

        None
    }

    /// Performs DFS on the graph of link references, and populates `deps` with all found libraries.
    fn collect_dependencies<'a>(
        &'a self,
        target: &'a ArtifactId,
        deps: &mut BTreeSet<&'a ArtifactId>,
    ) -> Result<(), LinkerError> {
        let references = self
            .contracts
            .get(target)
            .ok_or(LinkerError::MissingTargetArtifact)?
            .all_link_references();
        for (file, libs) in &references {
            for contract in libs.keys() {
                let id = self
                    .find_artifact_id_by_library_path(file, contract, Some(&target.version))
                    .ok_or_else(|| LinkerError::MissingLibraryArtifact {
                        file: file.to_string(),
                        name: contract.to_string(),
                    })?;
                if deps.insert(id) {
                    self.collect_dependencies(id, deps)?;
                }
            }
        }

        Ok(())
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
    pub fn link_with_nonce_or_address<'a>(
        &'a self,
        libraries: Libraries,
        sender: Address,
        mut nonce: u64,
        target: &'a ArtifactId,
    ) -> Result<LinkOutput, LinkerError> {
        // Library paths in `link_references` keys are always stripped, so we have to strip
        // user-provided paths to be able to match them correctly.
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());

        let mut needed_libraries = BTreeSet::new();
        self.collect_dependencies(target, &mut needed_libraries)?;

        let mut libs_to_deploy = Vec::new();

        // If `libraries` does not contain needed dependency, compute its address and add to
        // `libs_to_deploy`.
        for id in needed_libraries {
            let (lib_path, lib_name) = self.convert_artifact_id_to_lib_path(id);

            libraries.libs.entry(lib_path).or_default().entry(lib_name).or_insert_with(|| {
                let address = sender.create(nonce);
                libs_to_deploy.push((id, address));
                nonce += 1;

                address.to_checksum(None)
            });
        }

        // Link and collect bytecodes for `libs_to_deploy`.
        let libs_to_deploy = libs_to_deploy
            .into_iter()
            .map(|(id, _)| {
                Ok(self.link(id, &libraries)?.get_bytecode_bytes().unwrap().into_owned())
            })
            .collect::<Result<Vec<_>, LinkerError>>()?;

        Ok(LinkOutput { libraries, libs_to_deploy })
    }

    /// Links given artifact with given libraries.
    pub fn link(
        &self,
        target: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<CompactContractBytecode, LinkerError> {
        let mut contract =
            self.contracts.get(target).ok_or(LinkerError::MissingTargetArtifact)?.clone();
        for (file, libs) in &libraries.libs {
            for (name, address) in libs {
                let address = Address::from_str(address).map_err(LinkerError::InvalidAddress)?;
                if let Some(bytecode) = contract.bytecode.as_mut() {
                    bytecode.link(file.to_string_lossy(), name, address);
                }
                if let Some(deployed_bytecode) =
                    contract.deployed_bytecode.as_mut().and_then(|b| b.bytecode.as_mut())
                {
                    deployed_bytecode.link(file.to_string_lossy(), name, address);
                }
            }
        }
        Ok(contract)
    }

    pub fn get_linked_artifacts(
        &self,
        libraries: &Libraries,
    ) -> Result<ArtifactContracts, LinkerError> {
        self.contracts.keys().map(|id| Ok((id.clone(), self.link(id, libraries)?))).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::*;
    use foundry_compilers::{Project, ProjectPathsConfig};

    struct LinkerTest {
        project: Project,
        linker: Linker,
        dependency_assertions: HashMap<String, Vec<(String, Address)>>,
    }

    impl LinkerTest {
        fn new(path: impl Into<PathBuf>, strip_prefixes: bool) -> Self {
            let path = path.into();
            let paths = ProjectPathsConfig::builder()
                .root("../../testdata/linking")
                .lib("../../testdata/lib")
                .sources(path.clone())
                .tests(path)
                .build()
                .unwrap();

            let project =
                Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();

            let mut contracts = project.compile().unwrap();

            if strip_prefixes {
                contracts = contracts.with_stripped_file_prefixes(project.root());
            }

            let contracts = contracts
                .into_artifacts()
                .map(|(id, c)| (id, c.into_contract_bytecode()))
                .collect::<ArtifactContracts>();

            let linker = Linker::new(project.root(), contracts);

            Self { project, linker, dependency_assertions: HashMap::new() }
        }

        fn assert_dependencies(
            mut self,
            artifact_id: String,
            deps: Vec<(String, Address)>,
        ) -> Self {
            self.dependency_assertions.insert(artifact_id, deps);
            self
        }

        fn test_with_sender_and_nonce(self, sender: Address, initial_nonce: u64) {
            for id in self.linker.contracts.keys() {
                // If we didn't strip paths, artifacts will have absolute paths.
                // That's expected and we want to ensure that only `libraries` object has relative
                // paths, artifacts should be kept as is.
                let source = id
                    .source
                    .strip_prefix(self.project.root())
                    .unwrap_or(&id.source)
                    .to_string_lossy();
                let identifier = format!("{source}:{}", id.name);

                // Skip ds-test as it always has no dependencies etc. (and the path is outside root
                // so is not sanitized)
                if identifier.contains("DSTest") {
                    continue;
                }

                let LinkOutput { libs_to_deploy, libraries, .. } = self
                    .linker
                    .link_with_nonce_or_address(Default::default(), sender, initial_nonce, id)
                    .expect("Linking failed");

                let assertions = self
                    .dependency_assertions
                    .get(&identifier)
                    .unwrap_or_else(|| panic!("Unexpected artifact: {identifier}"));

                assert_eq!(
                    libs_to_deploy.len(),
                    assertions.len(),
                    "artifact {identifier} has more/less dependencies than expected ({} vs {}): {:#?}",
                    libs_to_deploy.len(),
                    assertions.len(),
                    libs_to_deploy
                );

                for (dep_identifier, address) in assertions {
                    let (file, name) = dep_identifier.split_once(':').unwrap();
                    if let Some(lib_address) =
                        libraries.libs.get(&PathBuf::from(file)).and_then(|libs| libs.get(name))
                    {
                        assert_eq!(*lib_address, address.to_string(), "incorrect library address for dependency {dep_identifier} of {identifier}");
                    } else {
                        panic!("Library not found")
                    }
                }
            }
        }
    }

    fn link_test(path: impl Into<PathBuf>, test_fn: impl Fn(LinkerTest)) {
        let path = path.into();
        test_fn(LinkerTest::new(path.clone(), true));
        test_fn(LinkerTest::new(path, false));
    }

    #[test]
    fn link_simple() {
        link_test("../../testdata/linking/simple", |linker| {
            linker
                .assert_dependencies("simple/Simple.t.sol:Lib".to_string(), vec![])
                .assert_dependencies(
                    "simple/Simple.t.sol:LibraryConsumer".to_string(),
                    vec![(
                        "simple/Simple.t.sol:Lib".to_string(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    )],
                )
                .assert_dependencies(
                    "simple/Simple.t.sol:SimpleLibraryLinkingTest".to_string(),
                    vec![(
                        "simple/Simple.t.sol:Lib".to_string(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    )],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }

    #[test]
    fn link_nested() {
        link_test("../../testdata/linking/nested", |linker| {
            linker
                .assert_dependencies("nested/Nested.t.sol:Lib".to_string(), vec![])
                .assert_dependencies(
                    "nested/Nested.t.sol:NestedLib".to_string(),
                    vec![(
                        "nested/Nested.t.sol:Lib".to_string(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    )],
                )
                .assert_dependencies(
                    "nested/Nested.t.sol:LibraryConsumer".to_string(),
                    vec![
                        // Lib shows up here twice, because the linker sees it twice, but it should
                        // have the same address and nonce.
                        (
                            "nested/Nested.t.sol:Lib".to_string(),
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "nested/Nested.t.sol:NestedLib".to_string(),
                            Address::from_str("0x47e9Fbef8C83A1714F1951F142132E6e90F5fa5D")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "nested/Nested.t.sol:NestedLibraryLinkingTest".to_string(),
                    vec![
                        (
                            "nested/Nested.t.sol:Lib".to_string(),
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "nested/Nested.t.sol:NestedLib".to_string(),
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
        link_test("../../testdata/linking/duplicate", |linker| {
            linker
                .assert_dependencies("duplicate/Duplicate.t.sol:A".to_string(), vec![])
                .assert_dependencies("duplicate/Duplicate.t.sol:B".to_string(), vec![])
                .assert_dependencies(
                    "duplicate/Duplicate.t.sol:C".to_string(),
                    vec![(
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    )],
                )
                .assert_dependencies(
                    "duplicate/Duplicate.t.sol:D".to_string(),
                    vec![(
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    )],
                )
                .assert_dependencies(
                    "duplicate/Duplicate.t.sol:E".to_string(),
                    vec![
                        (
                            "duplicate/Duplicate.t.sol:A".to_string(),
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:C".to_string(),
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "duplicate/Duplicate.t.sol:LibraryConsumer".to_string(),
                    vec![
                        (
                            "duplicate/Duplicate.t.sol:A".to_string(),
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:B".to_string(),
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:C".to_string(),
                            Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:D".to_string(),
                            Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:E".to_string(),
                            Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "duplicate/Duplicate.t.sol:DuplicateLibraryLinkingTest".to_string(),
                    vec![
                        (
                            "duplicate/Duplicate.t.sol:A".to_string(),
                            Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:B".to_string(),
                            Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:C".to_string(),
                            Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:D".to_string(),
                            Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782")
                                .unwrap(),
                        ),
                        (
                            "duplicate/Duplicate.t.sol:E".to_string(),
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
        link_test("../../testdata/linking/cycle", |linker| {
            linker
                .assert_dependencies(
                    "cycle/Cycle.t.sol:Foo".to_string(),
                    vec![
                        (
                            "cycle/Cycle.t.sol:Foo".to_string(),
                            Address::from_str("0x47e9Fbef8C83A1714F1951F142132E6e90F5fa5D")
                                .unwrap(),
                        ),
                        (
                            "cycle/Cycle.t.sol:Bar".to_string(),
                            Address::from_str("0x5a443704dd4B594B382c22a083e2BD3090A6feF3")
                                .unwrap(),
                        ),
                    ],
                )
                .assert_dependencies(
                    "cycle/Cycle.t.sol:Bar".to_string(),
                    vec![
                        (
                            "cycle/Cycle.t.sol:Foo".to_string(),
                            Address::from_str("0x47e9Fbef8C83A1714F1951F142132E6e90F5fa5D")
                                .unwrap(),
                        ),
                        (
                            "cycle/Cycle.t.sol:Bar".to_string(),
                            Address::from_str("0x5a443704dd4B594B382c22a083e2BD3090A6feF3")
                                .unwrap(),
                        ),
                    ],
                )
                .test_with_sender_and_nonce(Address::default(), 1);
        });
    }
}
