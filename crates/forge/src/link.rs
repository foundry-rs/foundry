use alloy_primitives::Address;
use eyre::Result;
use foundry_compilers::{artifacts::Libraries, contracts::ArtifactContracts, ArtifactId};
use semver::Version;
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

fn find_artifact_id_by_library_path<'a>(
    contracts: &'a ArtifactContracts,
    file: &String,
    name: &String,
    version: Option<&Version>,
) -> &'a ArtifactId {
    for id in contracts.keys() {
        if let Some(version) = version {
            if id.version != *version {
                continue;
            }
        }
        // name is either {LibName} or {LibName}.{version}
        if id.name.split('.').next().unwrap() != name {
            continue;
        }

        if !(id.source.ends_with(file)) {
            continue;
        }

        return id;
    }

    panic!("artifact not found for library {file} {name}");
}

pub fn collect_dependencies<'a>(
    target: &'a ArtifactId,
    contracts: &'a ArtifactContracts,
    deps: &mut HashSet<&'a ArtifactId>,
) {
    let references = contracts.get(target).unwrap().all_link_references();
    for (file, libs) in &references {
        for contract in libs.keys() {
            let id =
                find_artifact_id_by_library_path(contracts, file, contract, Some(&target.version));
            if deps.insert(id) {
                collect_dependencies(id, contracts, deps);
            }
        }
    }
}

pub struct LinkOutput<'a> {
    pub contracts: ArtifactContracts,
    pub predeployed_libs: Vec<(&'a ArtifactId, Address)>,
    pub libs_to_deploy: Vec<(&'a ArtifactId, Address)>,
}

pub fn link_with_nonce_or_address<'a>(
    contracts: &'a ArtifactContracts,
    deployed_library_addresses: &Libraries,
    sender: Address,
    mut nonce: u64,
    target: &'a ArtifactId,
) -> Result<LinkOutput<'a>> {
    let mut needed_libraries = HashSet::new();
    collect_dependencies(target, contracts, &mut needed_libraries);

    let mut predeployed_libs = HashMap::new();

    // Populate predeployed libs firstly
    for (path, libs) in &deployed_library_addresses.libs {
        let path = path.to_string_lossy().to_string();
        for (name, address) in libs {
            let artifact_id = find_artifact_id_by_library_path(contracts, &path, name, None);
            if needed_libraries.contains(artifact_id) {
                let address = Address::from_str(address)?;
                predeployed_libs.insert(artifact_id, address);
            }
        }
    }

    let mut libs_to_deploy = Vec::new();

    for id in needed_libraries {
        if !predeployed_libs.contains_key(id) {
            libs_to_deploy.push((id, sender.create(nonce)));
            nonce += 1;
        }
    }

    let predeployed_libs = predeployed_libs.into_iter().collect::<Vec<_>>();

    // Link contracts
    let contracts = contracts
        .iter()
        .map(|(id, contract)| {
            let mut contract = contract.clone();

            for (id, address) in libs_to_deploy.iter().chain(predeployed_libs.iter()) {
                if let Some(bytecode) = contract.bytecode.as_mut() {
                    bytecode.link(id.source.to_string_lossy(), &id.name, *address);
                }
                if let Some(deployed_bytecode) =
                    contract.deployed_bytecode.as_mut().and_then(|b| b.bytecode.as_mut())
                {
                    deployed_bytecode.link(id.source.to_string_lossy(), &id.name, *address);
                }
            }
            (id.clone(), contract)
        })
        .collect::<ArtifactContracts>();

    Ok(LinkOutput { contracts, predeployed_libs, libs_to_deploy })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use foundry_compilers::{Project, ProjectPathsConfig};

    struct LinkerTest {
        contracts: ArtifactContracts,
        dependency_assertions: HashMap<String, Vec<(String, u64, Address)>>,
    }

    impl LinkerTest {
        fn new(path: impl Into<PathBuf>) -> Self {
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
            let contracts = project
                .compile()
                .unwrap()
                .with_stripped_file_prefixes(project.root())
                .into_artifacts()
                .map(|(id, c)| (id, c.into_contract_bytecode()))
                .collect::<ArtifactContracts>();

            Self { contracts, dependency_assertions: HashMap::new() }
        }

        fn assert_dependencies(
            mut self,
            artifact_id: String,
            deps: Vec<(String, u64, Address)>,
        ) -> Self {
            self.dependency_assertions.insert(artifact_id, deps);
            self
        }

        fn test_with_sender_and_nonce(self, sender: Address, initial_nonce: u64) {
            for id in self.contracts.keys() {
                let identifier = id.identifier();

                // Skip ds-test as it always has no dependencies etc. (and the path is outside root
                // so is not sanitized)
                if identifier.contains("DSTest") {
                    continue;
                }

                let LinkOutput { libs_to_deploy, .. } = link_with_nonce_or_address(
                    &self.contracts,
                    &Default::default(),
                    sender,
                    initial_nonce,
                    id,
                )
                .expect("Linking failed");

                let assertions = self
                    .dependency_assertions
                    .get(&identifier)
                    .unwrap_or_else(|| panic!("Unexpected artifact: {identifier}"));

                let expected_libs =
                    assertions.iter().map(|(identifier, _, _)| identifier).collect::<HashSet<_>>();

                assert_eq!(
                    libs_to_deploy.len(),
                    expected_libs.len(),
                    "artifact {identifier} has more/less dependencies than expected ({} vs {}): {:#?}",
                    libs_to_deploy.len(),
                    assertions.len(),
                    libs_to_deploy
                );

                let identifiers =
                    libs_to_deploy.iter().map(|(id, _)| id.identifier()).collect::<HashSet<_>>();

                for lib in expected_libs {
                    assert!(identifiers.contains(lib));
                }

                let unique_libs =
                    libs_to_deploy.iter().map(|(_, addr)| addr).collect::<HashSet<_>>();

                assert_eq!(
                    unique_libs.len(),
                    libs_to_deploy.len(),
                    "not all libraries are unqiue: {:#?}",
                    libs_to_deploy
                );
            }
        }
    }

    #[test]
    fn link_simple() {
        LinkerTest::new("../../testdata/linking/simple")
            .assert_dependencies("simple/Simple.t.sol:Lib".to_string(), vec![])
            .assert_dependencies(
                "simple/Simple.t.sol:LibraryConsumer".to_string(),
                vec![(
                    "simple/Simple.t.sol:Lib".to_string(),
                    1,
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .assert_dependencies(
                "simple/Simple.t.sol:SimpleLibraryLinkingTest".to_string(),
                vec![(
                    "simple/Simple.t.sol:Lib".to_string(),
                    1,
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .test_with_sender_and_nonce(Address::default(), 1);
    }

    #[test]
    fn link_nested() {
        LinkerTest::new("../../testdata/linking/nested")
            .assert_dependencies("nested/Nested.t.sol:Lib".to_string(), vec![])
            .assert_dependencies(
                "nested/Nested.t.sol:NestedLib".to_string(),
                vec![(
                    "nested/Nested.t.sol:Lib".to_string(),
                    1,
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
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:Lib".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:NestedLib".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                ],
            )
            .assert_dependencies(
                "nested/Nested.t.sol:NestedLibraryLinkingTest".to_string(),
                vec![
                    (
                        "nested/Nested.t.sol:Lib".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:Lib".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:NestedLib".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                ],
            )
            .test_with_sender_and_nonce(Address::default(), 1);
    }

    /// This test ensures that complicated setups with many libraries, some of which are referenced
    /// in more than one place, result in correct linking.
    ///
    /// Each `assert_dependencies` should be considered in isolation, i.e. read it as "if I wanted
    /// to deploy this contract, I would have to deploy the dependencies in this order with this
    /// nonce".
    ///
    /// A library may show up more than once, but it should *always* have the same nonce and address
    /// with respect to the single `assert_dependencies` call. There should be no gaps in the nonce
    /// otherwise, i.e. whenever a new dependency is encountered, the nonce should be a single
    /// increment larger than the previous largest nonce.
    #[test]
    fn link_duplicate() {
        LinkerTest::new("../../testdata/linking/duplicate")
            .assert_dependencies("duplicate/Duplicate.t.sol:A".to_string(), vec![])
            .assert_dependencies("duplicate/Duplicate.t.sol:B".to_string(), vec![])
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:C".to_string(),
                vec![(
                    "duplicate/Duplicate.t.sol:A".to_string(),
                    1,
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:D".to_string(),
                vec![(
                    "duplicate/Duplicate.t.sol:B".to_string(),
                    1,
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:E".to_string(),
                vec![
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                ],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:LibraryConsumer".to_string(),
                vec![
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        3,
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:D".to_string(),
                        4,
                        Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        3,
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:E".to_string(),
                        5,
                        Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f").unwrap(),
                    ),
                ],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:DuplicateLibraryLinkingTest".to_string(),
                vec![
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        3,
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        2,
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:D".to_string(),
                        4,
                        Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        1,
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        3,
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:E".to_string(),
                        5,
                        Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f").unwrap(),
                    ),
                ],
            )
            .test_with_sender_and_nonce(Address::default(), 1);
    }
}
