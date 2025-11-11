//! # foundry-linking
//!
//! EVM bytecode linker.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use alloy_primitives::{Address, B256, Bytes};
use foundry_compilers::{
    Artifact, ArtifactId,
    artifacts::{CompactBytecode, CompactContractBytecodeCow, Libraries},
    contracts::ArtifactContracts,
};
use rayon::prelude::*;
use semver::Version;
use std::{
    collections::{BTreeMap, BTreeSet},
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
    #[error("cyclic dependency found, can't link libraries via CREATE2")]
    CyclicDependency,
    #[error("failed linking {artifact}")]
    LinkingFailed { artifact: String },
}

pub struct Linker<'a> {
    /// Root of the project, used to determine whether artifact/library path can be stripped.
    pub root: PathBuf,
    /// Compilation artifacts.
    pub contracts: ArtifactContracts<CompactContractBytecodeCow<'a>>,
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
        let path = id.source.strip_prefix(self.root.as_path()).unwrap_or(&id.source);
        // name is either {LibName} or {LibName}.{version}
        let name = id.name.split('.').next().unwrap();

        (path.to_path_buf(), name.to_owned())
    }

    /// Finds an [ArtifactId] object in the given [ArtifactContracts] keys which corresponds to the
    /// library path in the form of "./path/to/Lib.sol:Lib"
    ///
    /// Optionally accepts solc version, and if present, only compares artifacts with given version.
    fn find_artifact_id_by_library_path(
        &'a self,
        file: &str,
        name: &str,
        version: Option<&Version>,
    ) -> Option<&'a ArtifactId> {
        for id in self.contracts.keys() {
            if let Some(version) = version
                && id.version != *version
            {
                continue;
            }
            let (artifact_path, artifact_name) = self.convert_artifact_id_to_lib_path(id);

            if artifact_name == *name && artifact_path == Path::new(file) {
                return Some(id);
            }
        }

        None
    }

    /// Performs DFS on the graph of link references, and populates `deps` with all found libraries.
    fn collect_dependencies(
        &'a self,
        target: &'a ArtifactId,
        deps: &mut BTreeSet<&'a ArtifactId>,
    ) -> Result<(), LinkerError> {
        let contract = self.contracts.get(target).ok_or(LinkerError::MissingTargetArtifact)?;

        let mut references: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut extend = |bytecode: &CompactBytecode| {
            for (file, libs) in &bytecode.link_references {
                references.entry(file.clone()).or_default().extend(libs.keys().cloned());
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

        for (file, libs) in references {
            for name in libs {
                let id = self
                    .find_artifact_id_by_library_path(&file, &name, Some(&target.version))
                    .ok_or_else(|| LinkerError::MissingLibraryArtifact {
                        file: file.clone(),
                        name,
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
    pub fn link_with_nonce_or_address(
        &'a self,
        libraries: Libraries,
        sender: Address,
        mut nonce: u64,
        targets: impl IntoIterator<Item = &'a ArtifactId>,
    ) -> Result<LinkOutput, LinkerError> {
        // Library paths in `link_references` keys are always stripped, so we have to strip
        // user-provided paths to be able to match them correctly.
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());

        let mut needed_libraries = BTreeSet::new();
        for target in targets {
            self.collect_dependencies(target, &mut needed_libraries)?;
        }

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
            .into_par_iter()
            .map(|(id, _)| {
                Ok(self.link(id, &libraries)?.get_bytecode_bytes().unwrap().into_owned())
            })
            .collect::<Result<Vec<_>, LinkerError>>()?;

        Ok(LinkOutput { libraries, libs_to_deploy })
    }

    pub fn link_with_create2(
        &'a self,
        libraries: Libraries,
        sender: Address,
        salt: B256,
        target: &'a ArtifactId,
    ) -> Result<LinkOutput, LinkerError> {
        // Library paths in `link_references` keys are always stripped, so we have to strip
        // user-provided paths to be able to match them correctly.
        let mut libraries = libraries.with_stripped_file_prefixes(self.root.as_path());

        let mut needed_libraries = BTreeSet::new();
        self.collect_dependencies(target, &mut needed_libraries)?;

        let mut needed_libraries = needed_libraries
            .into_par_iter()
            .filter(|id| {
                // Filter out already provided libraries.
                let (file, name) = self.convert_artifact_id_to_lib_path(id);
                !libraries.libs.contains_key(&file) || !libraries.libs[&file].contains_key(&name)
            })
            .map(|id| {
                // Link library with provided libs and extract bytecode object (possibly unlinked).
                let bytecode = self.link(id, &libraries).unwrap().bytecode.unwrap();
                (id, bytecode)
            })
            .collect::<Vec<_>>();

        let mut libs_to_deploy = Vec::new();

        // Iteratively compute addresses and link libraries until we have no unlinked libraries
        // left.
        while !needed_libraries.is_empty() {
            // Find any library which is fully linked.
            let deployable = needed_libraries
                .iter()
                .enumerate()
                .find(|(_, (_, bytecode))| !bytecode.object.is_unlinked());

            // If we haven't found any deployable library, it means we have a cyclic dependency.
            let Some((index, &(id, _))) = deployable else {
                return Err(LinkerError::CyclicDependency);
            };
            let (_, bytecode) = needed_libraries.swap_remove(index);
            let code = bytecode.bytes().unwrap();
            let address = sender.create2_from_code(salt, code);
            libs_to_deploy.push(code.clone());

            let (file, name) = self.convert_artifact_id_to_lib_path(id);

            needed_libraries.par_iter_mut().for_each(|(_, bytecode)| {
                bytecode.to_mut().link(&file.to_string_lossy(), &name, address);
            });

            libraries.libs.entry(file).or_default().insert(name, address.to_checksum(None));
        }

        Ok(LinkOutput { libraries, libs_to_deploy })
    }

    /// Links given artifact with given libraries.
    pub fn link(
        &self,
        target: &ArtifactId,
        libraries: &Libraries,
    ) -> Result<CompactContractBytecodeCow<'a>, LinkerError> {
        let mut contract =
            self.contracts.get(target).ok_or(LinkerError::MissingTargetArtifact)?.clone();
        for (file, libs) in &libraries.libs {
            for (name, address) in libs {
                let address = Address::from_str(address).map_err(LinkerError::InvalidAddress)?;
                if let Some(bytecode) = contract.bytecode.as_mut() {
                    bytecode.to_mut().link(&file.to_string_lossy(), name, address);
                }
                if let Some(deployed_bytecode) =
                    contract.deployed_bytecode.as_mut().and_then(|b| b.to_mut().bytecode.as_mut())
                {
                    deployed_bytecode.link(&file.to_string_lossy(), name, address);
                }
            }
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
        self.contracts
            .par_iter()
            .map(|(id, _)| Ok((id.clone(), self.link(id, libraries)?)))
            .collect::<Result<_, _>>()
            .map(ArtifactContracts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, fixed_bytes, map::HashMap};
    use foundry_compilers::{
        Project, ProjectCompileOutput, ProjectPathsConfig,
        multi::MultiCompiler,
        solc::{Solc, SolcCompiler},
    };
    use std::sync::OnceLock;

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
                    .link_with_create2(Default::default(), sender, salt, id)
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
            let LinkOutput { libs_to_deploy, libraries } = output;

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
