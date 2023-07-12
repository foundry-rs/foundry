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
    path::PathBuf,
    str::FromStr,
    time::Duration,
};
use tracing::trace;

pub mod abi;
pub mod error;
pub mod glob;
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
    file_name: String,
    file: String,
    key: String,
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
    /// all artifacts grouped by slug
    inner: BTreeMap<String, ArtifactCode>,
}

impl AllArtifactsBySlug {
    /// Finds the code for the target of the artifact and the matching key.
    ///
    /// In multi-version builds the identifier also includes the version number. So we try to find
    /// that artifact first. If there's no matching, versioned artifact we continue with the
    /// `target_slug`
    fn find_code(
        &self,
        artifact: &ArtifactId,
        target_slug: &str,
        key: &str,
    ) -> Option<(String, CompactContractBytecode)> {
        // try to find by versioned slug first to match the exact version
        let major = artifact.version.major;
        let minor = artifact.version.minor;
        let patch = artifact.version.patch;
        let version_slug =
            format!("{key}.{major}.{minor}.{patch}.json:{key}.{major}.{minor}.{patch}");
        let (identifier, code) = if let Some(code) = self.inner.get(&version_slug) {
            (version_slug, code)
        } else {
            let code = self.inner.get(target_slug)?;
            (target_slug.to_string(), code)
        };
        Some((identifier, code.code.clone()))
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
    link_key_construction: impl Fn(String, String) -> (String, String, String),
    post_link: impl Fn(PostLinkInput<T, U>) -> eyre::Result<()>,
) -> Result<()> {
    // create a mapping of fname => Vec<(fname, file, key)>,
    let link_tree: BTreeMap<String, ArtifactDependencies> = contracts
        .iter()
        .map(|(id, contract)| {
            let key = id.slug();
            let references = contract
                .all_link_references()
                .iter()
                .flat_map(|(file, link)| {
                    link.keys().map(|key| link_key_construction(file.to_string(), key.to_string()))
                })
                .map(|(file_name, file, key)| ArtifactDependency { file_name, file, key })
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
                    artifact_id.slug(),
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
                    trace!(target : "forge::link", target=id.slug(), version=?id.version, "unlinked contract");

                    // link needed
                    recurse_link(
                        id.slug(),
                        (&mut target_bytecode, &mut target_bytecode_runtime),
                        &artifacts_by_slug,
                        &link_tree,
                        &mut dependencies,
                        &mut internally_deployed_libraries,
                        &deployed_library_addresses,
                        &mut nonce.clone(),
                        sender,
                    );
                }
                BytecodeObject::Bytecode(ref bytes) => {
                    if bytes.as_ref().is_empty() {
                        // abstract, skip
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
) {
    // check if we have dependencies
    if let Some(dependencies) = dependency_tree.get(&target) {
        trace!(target : "forge::link", ?target, "linking contract");

        // for each dependency, try to link
        dependencies.dependencies.iter().for_each(|dep| {
            let ArtifactDependency { file_name: next_target, file, key } = dep;
            // get the dependency
            trace!(target : "forge::link", dependency = next_target, file, key, version=?dependencies.artifact_id.version,  "get dependency");
            let (next_identifier, artifact) = artifacts
                .find_code(&dependencies.artifact_id, next_target, key)
                .unwrap_or_else(|| panic!("No target contract named {next_target}"))
                ;
            let mut next_target_bytecode = artifact
                .bytecode
                .unwrap_or_else(|| panic!("No bytecode for contract {next_target}"));
            let mut next_target_runtime_bytecode = artifact
                .deployed_bytecode
                .expect("No target runtime bytecode")
                .bytecode
                .expect("No target runtime");

            // make sure dependency is fully linked
            if let Some(deps) = dependency_tree.get(&next_identifier) {
                if !deps.dependencies.is_empty() {
                    // actually link the nested dependencies to this dependency
                    recurse_link(
                        next_identifier,
                        (&mut next_target_bytecode, &mut next_target_runtime_bytecode),
                        artifacts,
                        dependency_tree,
                        deployment,
                        internally_deployed_libraries,
                        deployed_library_addresses,
                        nonce,
                        sender,
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
                // the user specified the library address

                deployed_address
            } else if let Some((cached_nonce, deployed_address)) = internally_deployed_libraries.get(&format!("{file}:{key}")) {
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
                internally_deployed_libraries.insert(format!("{file}:{key}"), (*nonce, computed_address));

                computed_address
            };

            // link the dependency to the target
            target_bytecode.0.link(file.clone(), key.clone(), address);
            target_bytecode.1.link(file.clone(), key.clone(), address);
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
    use ethers::{
        abi::Abi,
        solc::{Project, ProjectPathsConfig},
        types::{Address, Bytes},
    };
    use foundry_common::ContractsByArtifact;

    #[test]
    fn test_linking() {
        let mut contract_names = [
            "DSTest.json:DSTest",
            "Lib.json:Lib",
            "LibraryConsumer.json:LibraryConsumer",
            "LibraryLinkingTest.json:LibraryLinkingTest",
            "NestedLib.json:NestedLib",
        ];
        contract_names.sort_unstable();

        let paths = ProjectPathsConfig::builder()
            .root("../testdata")
            .sources("../testdata/core")
            .build()
            .unwrap();

        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();

        let output = project.compile().unwrap();
        let contracts = output
            .into_artifacts()
            .filter(|(i, _)| contract_names.contains(&i.slug().as_str()))
            .map(|(id, c)| (id, c.into_contract_bytecode()))
            .collect::<ArtifactContracts>();

        let mut known_contracts = ContractsByArtifact::default();
        let mut deployable_contracts: BTreeMap<String, (Abi, Bytes, Vec<Bytes>)> =
            Default::default();

        let mut res = contracts.keys().map(|i| i.slug()).collect::<Vec<String>>();
        res.sort_unstable();
        assert_eq!(&res[..], &contract_names[..]);

        let lib_linked = hex::encode(
            &contracts
                .iter()
                .find(|(i, _)| i.slug() == "Lib.json:Lib")
                .unwrap()
                .1
                .bytecode
                .clone()
                .expect("library had no bytecode")
                .object
                .into_bytes()
                .expect("could not get bytecode as bytes"),
        );
        let nested_lib_unlinked = &contracts
            .iter()
            .find(|(i, _)| i.slug() == "NestedLib.json:NestedLib")
            .unwrap()
            .1
            .bytecode
            .as_ref()
            .expect("nested library had no bytecode")
            .object
            .as_str()
            .expect("could not get bytecode as str")
            .to_string();

        link_with_nonce_or_address(
            contracts,
            &mut known_contracts,
            Default::default(),
            Address::default(),
            U256::one(),
            &mut deployable_contracts,
            |file, key| (format!("{key}.json:{key}"), file, key),
            |post_link_input| {
                match post_link_input.id.slug().as_str() {
                    "DSTest.json:DSTest" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "LibraryLinkingTest.json:LibraryLinkingTest" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(
                            hex::encode(&post_link_input.dependencies[0].bytecode),
                            lib_linked
                        );
                        assert_eq!(
                            hex::encode(&post_link_input.dependencies[1].bytecode),
                            lib_linked
                        );
                        assert_ne!(
                            hex::encode(&post_link_input.dependencies[2].bytecode),
                            *nested_lib_unlinked
                        );
                    }
                    "Lib.json:Lib" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "NestedLib.json:NestedLib" => {
                        assert_eq!(post_link_input.dependencies.len(), 1);
                        assert_eq!(
                            hex::encode(&post_link_input.dependencies[0].bytecode),
                            lib_linked
                        );
                    }
                    "LibraryConsumer.json:LibraryConsumer" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(
                            hex::encode(&post_link_input.dependencies[0].bytecode),
                            lib_linked
                        );
                        assert_eq!(
                            hex::encode(&post_link_input.dependencies[1].bytecode),
                            lib_linked
                        );
                        assert_ne!(
                            hex::encode(&post_link_input.dependencies[2].bytecode),
                            *nested_lib_unlinked
                        );
                    }
                    s => panic!("unexpected slug {s}"),
                }
                Ok(())
            },
        )
        .unwrap();
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
