#![doc = include_str!("../README.md")]

use ethers_addressbook::contract;
use ethers_core::types::*;
use ethers_providers::{Middleware, Provider};
use ethers_solc::{
    artifacts::{BytecodeObject, CompactBytecode, CompactContractBytecode, Libraries},
    contracts::ArtifactContracts,
    ArtifactId,
};
use eyre::{Result, WrapErr};
use futures::future::BoxFuture;
use std::{
    collections::{BTreeMap, HashMap},
    env::VarError,
    fmt::{Formatter, Write},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use tracing::trace;

pub mod abi;
pub mod error;
pub mod glob;
pub mod path;
pub mod rpc;

/// Data passed to the post link handler of the linker for each linked artifact.
#[derive(Debug)]
pub struct PostLinkInput<'a, T, U> {
    /// The fully linked bytecode of the artifact
    pub contract: CompactContractBytecode,
    /// All artifacts passed to the linker
    pub known_contracts: &'a mut BTreeMap<ArtifactId, T>,
    /// The ID of the artifact
    pub id: ArtifactId,
    /// Extra data passed to the handler, which can be used as a scratch space.
    pub extra: &'a mut U,
    /// Each dependency of the contract in their resolved form.
    pub dependencies: Vec<ResolvedDependency>,
}

/// Dependencies for an artifact.
#[derive(Debug)]
struct ArtifactDependencies {
    /// All references to dependencies in the artifact's unlinked bytecode.
    dependencies: Vec<ArtifactDependency>,
    /// The ID of the artifact
    artifact_id: ArtifactId,
}

/// A dependency of an artifact.
#[derive(Debug)]
struct ArtifactDependency {
    file: String,
    key: String,
    version: String,
}

struct ArtifactCode {
    code: CompactContractBytecode,
    artifact_id: ArtifactId,
}

impl std::fmt::Debug for ArtifactCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.artifact_id.fmt(f)
    }
}

#[derive(Debug)]
struct AllArtifactsBySlug {
    /// all artifacts grouped by identifier
    inner: BTreeMap<String, ArtifactCode>,
}

impl AllArtifactsBySlug {
    /// Finds the code for the target of the artifact and the matching key.
    fn find_code(&self, identifier: &String, version: &String) -> Option<CompactContractBytecode> {
        trace!(target : "forge::link", identifier, "fetching artifact by identifier");
        let code = self
            .inner
            .get(identifier)
            .or(self.inner.get(&format!("{}.{}", identifier, version)))?;

        Some(code.code.clone())
    }
}

#[derive(Debug)]
pub struct ResolvedDependency {
    /// The address the linker resolved
    pub address: Address,
    /// The nonce used to resolve the dependency
    pub nonce: U256,
    pub id: String,
    pub bytecode: Bytes,
}

impl std::fmt::Display for ResolvedDependency {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @ {} (resolved with nonce {})", self.id, self.address, self.nonce)
    }
}

/// Links the given artifacts with a link key constructor function, passing the result of each
/// linkage to the given callback.
///
/// This function will recursively link all artifacts until none are unlinked. It does this by:
///
/// 1. Using the specified predeployed library addresses (`deployed_library_addresses`) for known
/// libraries (specified by the user) 2. Otherwise, computing the address the library would live at
/// if deployed by `sender`, given a starting nonce of `nonce`.
///
/// If the library was already deployed previously in step 2, the linker will re-use the previously
/// computed address instead of re-computing it.
///
/// The linker will call `post_link` for each linked artifact, providing:
///
/// 1. User-specified data (`extra`)
/// 2. The linked artifact's bytecode
/// 3. The ID of the artifact
/// 4. The dependencies necessary to deploy the contract
///
/// # Note
///
/// If you want to collect all dependencies of a set of contracts, you cannot just collect the
/// `dependencies` passed to the callback in a `Vec`, since the same library contract (with the
/// exact same address) might show up as a dependency for multiple contracts.
///
/// Instead, you must deduplicate *and* preserve the deployment order by pushing the dependencies to
/// a `Vec` iff it has not been seen before.
///
/// For an example of this, see [here](https://github.com/foundry-rs/foundry/blob/2308972dbc3a89c03488a05aceb3c428bb3e08c0/cli/src/cmd/forge/script/build.rs#L130-L151C9).
#[allow(clippy::too_many_arguments)]
pub fn link_with_nonce_or_address<T, U>(
    contracts: ArtifactContracts,
    known_contracts: &mut BTreeMap<ArtifactId, T>,
    deployed_library_addresses: Libraries,
    sender: Address,
    nonce: U256,
    extra: &mut U,
    post_link: impl Fn(PostLinkInput<T, U>) -> eyre::Result<()>,
    root: impl AsRef<Path>,
) -> Result<()> {
    // create a mapping of fname => Vec<(fname, file, key)>,
    let link_tree: BTreeMap<String, ArtifactDependencies> = contracts
        .iter()
        .map(|(id, contract)| {
            let key = id.identifier();
            let version = id.version.to_string();
            // Check if the version has metadata appended to it, which will be after the semver
            // version with a `+` separator. If so, strip it off.
            let version = match version.find('+') {
                Some(idx) => (version[..idx]).to_string(),
                None => version,
            };
            let references = contract
                .all_link_references()
                .iter()
                .flat_map(|(file, link)| link.keys().map(|key| (file.to_string(), key.to_string())))
                .map(|(file, key)| ArtifactDependency {
                    file,
                    key,
                    version: version.clone().to_owned(),
                })
                .collect();

            let references =
                ArtifactDependencies { dependencies: references, artifact_id: id.clone() };
            (key, references)
        })
        .collect();

    let artifacts_by_slug = AllArtifactsBySlug {
        inner: contracts
            .iter()
            .map(|(artifact_id, c)| {
                (
                    artifact_id.identifier(),
                    ArtifactCode { code: c.clone(), artifact_id: artifact_id.clone() },
                )
            })
            .collect(),
    };

    for (id, contract) in contracts.into_iter() {
        let (abi, maybe_deployment_bytes, maybe_runtime) = (
            contract.abi.as_ref(),
            contract.bytecode.as_ref(),
            contract.deployed_bytecode.as_ref(),
        );
        let mut internally_deployed_libraries = HashMap::new();

        if let (Some(abi), Some(bytecode), Some(runtime)) =
            (abi, maybe_deployment_bytes, maybe_runtime)
        {
            // we are going to mutate, but library contract addresses may change based on
            // the test so we clone
            let mut target_bytecode = bytecode.clone();
            let mut rt = runtime.clone();
            let mut target_bytecode_runtime = rt.bytecode.expect("No target runtime").clone();

            // instantiate a vector that gets filled with library deployment bytecode
            let mut dependencies = vec![];

            match bytecode.object {
                BytecodeObject::Unlinked(_) => {
                    trace!(target : "forge::link", target=id.identifier(), version=?id.version, "unlinked contract");

                    // link needed
                    recurse_link(
                        id.identifier(),
                        (&mut target_bytecode, &mut target_bytecode_runtime),
                        &artifacts_by_slug,
                        &link_tree,
                        &mut dependencies,
                        &mut internally_deployed_libraries,
                        &deployed_library_addresses,
                        &mut nonce.clone(),
                        sender,
                        root.as_ref(),
                    );
                }
                BytecodeObject::Bytecode(ref bytes) => {
                    if bytes.as_ref().is_empty() {
                        // Handle case where bytecode bytes are empty
                        let tc = CompactContractBytecode {
                            abi: Some(abi.clone()),
                            bytecode: None,
                            deployed_bytecode: None,
                        };

                        let post_link_input = PostLinkInput {
                            contract: tc,
                            known_contracts,
                            id,
                            extra,
                            dependencies,
                        };

                        post_link(post_link_input)?;
                        continue
                    }
                }
            }

            rt.bytecode = Some(target_bytecode_runtime);
            let tc = CompactContractBytecode {
                abi: Some(abi.clone()),
                bytecode: Some(target_bytecode),
                deployed_bytecode: Some(rt),
            };

            let post_link_input =
                PostLinkInput { contract: tc, known_contracts, id, extra, dependencies };

            post_link(post_link_input)?;
        }
    }
    Ok(())
}

/// Recursively links bytecode given a target contract artifact name, the bytecode(s) to be linked,
/// a mapping of contract artifact name to bytecode, a dependency mapping, a mutable list that
/// will be filled with the predeploy libraries, initial nonce, and the sender.
#[allow(clippy::too_many_arguments)]
fn recurse_link<'a>(
    // target name
    target: String,
    // to-be-modified/linked bytecode
    target_bytecode: (&'a mut CompactBytecode, &'a mut CompactBytecode),
    // All contract artifacts
    artifacts: &'a AllArtifactsBySlug,
    // fname => Vec<(fname, file, key)>
    dependency_tree: &'a BTreeMap<String, ArtifactDependencies>,
    // library deployment vector (file:contract:address, bytecode)
    deployment: &'a mut Vec<ResolvedDependency>,
    // libraries we have already deployed during the linking process.
    // the key is `file:contract` and the value is the address we computed
    internally_deployed_libraries: &'a mut HashMap<String, (U256, Address)>,
    // deployed library addresses fname => adddress
    deployed_library_addresses: &'a Libraries,
    // nonce to start at
    nonce: &mut U256,
    // sender
    sender: Address,
    // project root path
    root: impl AsRef<Path>,
) {
    // check if we have dependencies
    if let Some(dependencies) = dependency_tree.get(&target) {
        trace!(target : "forge::link", ?target, "linking contract");

        // for each dependency, try to link
        dependencies.dependencies.iter().for_each(|dep| {
            let ArtifactDependency {  file, key, version } = dep;
            let next_target = format!("{file}:{key}");
            let root = PathBuf::from(root.as_ref().to_str().unwrap());
            // get the dependency
            trace!(target : "forge::link", dependency = next_target, file, key, version=?dependencies.artifact_id.version,  "get dependency");
            let  artifact = match artifacts
                .find_code(&next_target, version) {
                    Some(artifact) => artifact,
                    None => {
                        // In some project setups, like JS-style workspaces, you might not have node_modules available at the root of the foundry project.
                        // In this case, imported dependencies from outside the root might not have their paths tripped correctly.
                        // Therefore, we fall back to a manual path join to locate the file.
                        let fallback_path =  dunce::canonicalize(root.join(file)).unwrap_or_else(|e| panic!("No artifact for contract \"{next_target}\". Attempted to compose fallback path but got got error {e}"));
                        let fallback_path = fallback_path.to_str().unwrap_or("No artifact for contract \"{next_target}\". Attempted to compose fallback path but could not create valid string");
                        let fallback_target = format!("{fallback_path}:{key}");

                        trace!(target : "forge::link", fallback_dependency = fallback_target, file, key, version=?dependencies.artifact_id.version,  "get dependency with fallback path");

                        match artifacts.find_code(&fallback_target, version) {
                        Some(artifact) => artifact,
                        None => panic!("No artifact for contract {next_target}"),
                    }},
                };
            let mut next_target_bytecode = artifact
                .bytecode
                .unwrap_or_else(|| panic!("No bytecode for contract {next_target}"));
            let mut next_target_runtime_bytecode = artifact
                .deployed_bytecode
                .expect("No target runtime bytecode")
                .bytecode
                .expect("No target runtime");

            // make sure dependency is fully linked
            if let Some(deps) = dependency_tree.get(&format!("{file}:{key}")) {
                if !deps.dependencies.is_empty() {
                    trace!(target : "forge::link", dependency = next_target, file, key, version=?dependencies.artifact_id.version,  "dependency has dependencies");

                    // actually link the nested dependencies to this dependency
                    recurse_link(
                        format!("{file}:{key}"),
                        (&mut next_target_bytecode, &mut next_target_runtime_bytecode),
                        artifacts,
                        dependency_tree,
                        deployment,
                        internally_deployed_libraries,
                        deployed_library_addresses,
                        nonce,
                        sender,
                        root,
                    );
                }
            }

            let mut deployed_address = None;

            if let Some(library_file) = deployed_library_addresses
                .libs
                .get(&PathBuf::from_str(file).expect("Invalid library path."))
            {
                if let Some(address) = library_file.get(key) {
                    deployed_address =
                        Some(Address::from_str(address).expect("Invalid library address passed."));
                }
            }

            let address = if let Some(deployed_address) = deployed_address {
                trace!(target : "forge::link", dependency = next_target, file, key, "dependency has pre-defined address");

                // the user specified the library address
                deployed_address
            } else if let Some((cached_nonce, deployed_address)) = internally_deployed_libraries.get(&format!("{file}:{key}")) {
                trace!(target : "forge::link", dependency = next_target, file, key, "dependency was previously deployed");

                // we previously deployed the library
                let library = format!("{file}:{key}:0x{}", hex::encode(deployed_address));

                // push the dependency into the library deployment vector
                deployment.push(ResolvedDependency {
                    id: library,
                    address: *deployed_address,
                    nonce: *cached_nonce,
                    bytecode: next_target_bytecode.object.into_bytes().unwrap_or_else(|| panic!( "Bytecode should be linked for {next_target}")),
                });
                *deployed_address
            } else {
                trace!(target : "forge::link", dependency = next_target, file, key, "dependency has to be deployed");

                // we need to deploy the library
                let used_nonce = *nonce;
                let computed_address = ethers_core::utils::get_contract_address(sender, used_nonce);
                *nonce += 1.into();
                let library = format!("{file}:{key}:0x{}", hex::encode(computed_address));

                // push the dependency into the library deployment vector
                deployment.push(ResolvedDependency {
                    id: library,
                    address: computed_address,
                    nonce: used_nonce,
                    bytecode: next_target_bytecode.object.into_bytes().unwrap_or_else(|| panic!( "Bytecode should be linked for {next_target}")),
                });

                // remember this library for later
                internally_deployed_libraries.insert(format!("{file}:{key}"), (used_nonce, computed_address));

                computed_address
            };

            // link the dependency to the target
            target_bytecode.0.link(file.clone(), key.clone(), address);
            target_bytecode.1.link(file.clone(), key.clone(), address);
            trace!(target : "forge::link", ?target, dependency = next_target, file, key, "linking dependency done");
        });
    }
}

/// Given a k/v serde object, it pretty prints its keys and values as a table.
pub fn to_table(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(map) => {
            let mut s = String::new();
            for (k, v) in map.iter() {
                writeln!(&mut s, "{k: <20} {v}\n").expect("could not write k/v to table");
            }
            s
        }
        _ => "".to_owned(),
    }
}

/// Resolves an input to [`NameOrAddress`]. The input could also be a contract/token name supported
/// by
/// [`ethers-addressbook`](https://github.com/gakonst/ethers-rs/tree/master/ethers-addressbook).
pub fn resolve_addr<T: Into<NameOrAddress>>(to: T, chain: Option<Chain>) -> Result<NameOrAddress> {
    Ok(match to.into() {
        NameOrAddress::Address(addr) => NameOrAddress::Address(addr),
        NameOrAddress::Name(contract_or_ens) => {
            if let Some(contract) = contract(&contract_or_ens) {
                let chain = chain
                    .ok_or_else(|| eyre::eyre!("resolving contract requires a known chain"))?;
                NameOrAddress::Address(contract.address(chain).ok_or_else(|| {
                    eyre::eyre!(
                        "contract: {} not found in addressbook for network: {}",
                        contract_or_ens,
                        chain
                    )
                })?)
            } else {
                NameOrAddress::Name(contract_or_ens)
            }
        }
    })
}

/// Reads the `ETHERSCAN_API_KEY` env variable
pub fn etherscan_api_key() -> eyre::Result<String> {
    std::env::var("ETHERSCAN_API_KEY").map_err(|err| match err {
        VarError::NotPresent => {
            eyre::eyre!(
                r#"
  You need an Etherscan Api Key to verify contracts.
  Create one at https://etherscan.io/myapikey
  Then export it with \`export ETHERSCAN_API_KEY=xxxxxxxx'"#
            )
        }
        VarError::NotUnicode(err) => {
            eyre::eyre!("Invalid `ETHERSCAN_API_KEY`: {:?}", err)
        }
    })
}

/// A type that keeps track of attempts
#[derive(Debug, Clone)]
pub struct Retry {
    retries: u32,
    delay: Option<u32>,
}

/// Sample retry logic implementation
impl Retry {
    pub fn new(retries: u32, delay: Option<u32>) -> Self {
        Self { retries, delay }
    }

    fn handle_err(&mut self, err: eyre::Report) {
        self.retries -= 1;
        tracing::warn!(
            "erroneous attempt ({} tries remaining): {}",
            self.retries,
            err.root_cause()
        );
        if let Some(delay) = self.delay {
            std::thread::sleep(Duration::from_secs(delay.into()));
        }
    }

    pub fn run<T, F>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> eyre::Result<T>,
    {
        loop {
            match callback() {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            }
        }
    }

    pub async fn run_async<'a, T, F>(mut self, mut callback: F) -> eyre::Result<T>
    where
        F: FnMut() -> BoxFuture<'a, eyre::Result<T>>,
    {
        loop {
            match callback().await {
                Err(e) if self.retries > 0 => self.handle_err(e),
                res => return res,
            };
        }
    }
}

pub async fn next_nonce(
    caller: Address,
    provider_url: &str,
    block: Option<BlockId>,
) -> Result<U256> {
    let provider = Provider::try_from(provider_url)
        .wrap_err_with(|| format!("Bad fork_url provider: {provider_url}"))?;
    Ok(provider.get_transaction_count(caller, block).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers_core::types::Address;
    use ethers_solc::{Project, ProjectPathsConfig};
    use foundry_common::ContractsByArtifact;

    struct LinkerTest {
        contracts: ArtifactContracts,
        dependency_assertions: HashMap<String, Vec<(String, U256, Address)>>,
        project: Project,
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

            Self { contracts, dependency_assertions: HashMap::new(), project }
        }

        fn assert_dependencies(
            mut self,
            artifact_id: String,
            deps: Vec<(String, U256, Address)>,
        ) -> Self {
            self.dependency_assertions.insert(artifact_id, deps);
            self
        }

        fn test_with_sender_and_nonce(self, sender: Address, initial_nonce: U256) {
            let mut called_once = false;
            link_with_nonce_or_address(
                self.contracts,
                &mut ContractsByArtifact::default(),
                Default::default(),
                sender,
                initial_nonce,
                &mut called_once,
                |post_link_input| {
                    *post_link_input.extra = true;
                    let identifier = post_link_input.id.identifier();

                    // Skip ds-test as it always has no dependencies etc. (and the path is outside root so is not sanitized)
                    if identifier.contains("DSTest") {
                        return Ok(())
                    }

                    let assertions = self
                        .dependency_assertions
                        .get(&identifier)
                        .unwrap_or_else(|| panic!("Unexpected artifact: {identifier}"));

                    assert_eq!(
                        post_link_input.dependencies.len(),
                        assertions.len(),
                        "artifact {identifier} has more/less dependencies than expected ({} vs {}): {:#?}",
                        post_link_input.dependencies.len(),
                        assertions.len(),
                        post_link_input.dependencies
                    );

                    for (expected, actual) in assertions.iter().zip(post_link_input.dependencies.iter()) {
                        let expected_lib_id = format!("{}:{:?}", expected.0, expected.2);
                        assert_eq!(expected_lib_id, actual.id, "unexpected dependency, expected: {}, got: {}", expected_lib_id, actual.id);
                        assert_eq!(actual.nonce, expected.1, "nonce wrong for dependency, expected: {}, got: {}", expected.1, actual.nonce);
                        assert_eq!(actual.address, expected.2, "address wrong for dependency, expected: {}, got: {}", expected.2, actual.address);
                    }

                    Ok(())
                },
                self.project.root(),
            )
            .expect("Linking failed");

            assert!(called_once, "linker did nothing");
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
                    U256::one(),
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .assert_dependencies(
                "simple/Simple.t.sol:SimpleLibraryLinkingTest".to_string(),
                vec![(
                    "simple/Simple.t.sol:Lib".to_string(),
                    U256::one(),
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .test_with_sender_and_nonce(Address::default(), U256::one());
    }

    #[test]
    fn link_nested() {
        LinkerTest::new("../../testdata/linking/nested")
            .assert_dependencies("nested/Nested.t.sol:Lib".to_string(), vec![])
            .assert_dependencies(
                "nested/Nested.t.sol:NestedLib".to_string(),
                vec![(
                    "nested/Nested.t.sol:Lib".to_string(),
                    U256::one(),
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
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:Lib".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:NestedLib".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                ],
            )
            .assert_dependencies(
                "nested/Nested.t.sol:NestedLibraryLinkingTest".to_string(),
                vec![
                    (
                        "nested/Nested.t.sol:Lib".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:Lib".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "nested/Nested.t.sol:NestedLib".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                ],
            )
            .test_with_sender_and_nonce(Address::default(), U256::one());
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
                    U256::one(),
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:D".to_string(),
                vec![(
                    "duplicate/Duplicate.t.sol:B".to_string(),
                    U256::one(),
                    Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                )],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:E".to_string(),
                vec![
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                ],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:LibraryConsumer".to_string(),
                vec![
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        U256::from(3),
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:D".to_string(),
                        U256::from(4),
                        Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        U256::from(3),
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:E".to_string(),
                        U256::from(5),
                        Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f").unwrap(),
                    ),
                ],
            )
            .assert_dependencies(
                "duplicate/Duplicate.t.sol:DuplicateLibraryLinkingTest".to_string(),
                vec![
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        U256::from(3),
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:B".to_string(),
                        U256::from(2),
                        Address::from_str("0x47e9fbef8c83a1714f1951f142132e6e90f5fa5d").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:D".to_string(),
                        U256::from(4),
                        Address::from_str("0x47c5e40890bce4a473a49d7501808b9633f29782").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:A".to_string(),
                        U256::one(),
                        Address::from_str("0x5a443704dd4b594b382c22a083e2bd3090a6fef3").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:C".to_string(),
                        U256::from(3),
                        Address::from_str("0x8be503bcded90ed42eff31f56199399b2b0154ca").unwrap(),
                    ),
                    (
                        "duplicate/Duplicate.t.sol:E".to_string(),
                        U256::from(5),
                        Address::from_str("0x29b2440db4a256b0c1e6d3b4cdcaa68e2440a08f").unwrap(),
                    ),
                ],
            )
            .test_with_sender_and_nonce(Address::default(), U256::one());
    }

    #[test]
    fn test_resolve_addr() {
        use std::str::FromStr;

        // DAI:mainnet exists in ethers-addressbook (0x6b175474e89094c44da98b954eedeac495271d0f)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Some(Chain::Mainnet)).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x6b175474e89094c44da98b954eedeac495271d0f").unwrap()
            ))
        );

        // DAI:goerli exists in ethers-adddressbook (0x11fE4B6AE13d2a6055C8D9cF65c55bac32B5d844)
        assert_eq!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Some(Chain::Goerli)).ok(),
            Some(NameOrAddress::Address(
                Address::from_str("0x11fE4B6AE13d2a6055C8D9cF65c55bac32B5d844").unwrap()
            ))
        );

        // DAI:moonbean does not exist in addressbook
        assert!(
            resolve_addr(NameOrAddress::Name("dai".to_string()), Some(Chain::MoonbeamDev)).is_err()
        );

        // If not present in addressbook, gets resolved to an ENS name.
        assert_eq!(
            resolve_addr(
                NameOrAddress::Name("contractnotpresent".to_string()),
                Some(Chain::Mainnet)
            )
            .ok(),
            Some(NameOrAddress::Name("contractnotpresent".to_string())),
        );

        // Nothing to resolve for an address.
        assert_eq!(
            resolve_addr(NameOrAddress::Address(Address::zero()), Some(Chain::Mainnet)).ok(),
            Some(NameOrAddress::Address(Address::zero())),
        );
    }
}
