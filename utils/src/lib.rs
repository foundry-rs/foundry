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
    collections::BTreeMap, env::VarError, fmt::Write, path::PathBuf, str::FromStr, time::Duration,
};
use tracing::trace;

pub mod abi;
pub mod rpc;

#[derive(Debug)]
pub struct PostLinkInput<'a, T, U> {
    pub contract: CompactContractBytecode,
    pub known_contracts: &'a mut BTreeMap<ArtifactId, T>,
    pub id: ArtifactId,
    pub extra: &'a mut U,
    pub dependencies: Vec<(String, Bytes)>,
}

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
) -> eyre::Result<()> {
    // create a mapping of fname => Vec<(fname, file, key)>,
    let link_tree: BTreeMap<String, Vec<(String, String, String)>> = contracts
        .iter()
        .map(|(id, contract)| {
            (
                id.slug(),
                contract
                    .all_link_references()
                    .iter()
                    .flat_map(|(file, link)| {
                        link.keys()
                            .map(|key| link_key_construction(file.to_string(), key.to_string()))
                    })
                    .collect::<Vec<(String, String, String)>>(),
            )
        })
        .collect();

    let contracts_by_slug = contracts
        .iter()
        .map(|(i, c)| (i.slug(), c.clone()))
        .collect::<BTreeMap<String, CompactContractBytecode>>();

    for (id, contract) in contracts.into_iter() {
        let (abi, maybe_deployment_bytes, maybe_runtime) = (
            contract.abi.as_ref(),
            contract.bytecode.as_ref(),
            contract.deployed_bytecode.as_ref(),
        );

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
                    // link needed
                    recurse_link(
                        id.slug(),
                        (&mut target_bytecode, &mut target_bytecode_runtime),
                        &contracts_by_slug,
                        &link_tree,
                        &mut dependencies,
                        &deployed_library_addresses,
                        nonce,
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
pub fn recurse_link<'a>(
    // target name
    target: String,
    // to-be-modified/linked bytecode
    target_bytecode: (&'a mut CompactBytecode, &'a mut CompactBytecode),
    // Contracts
    contracts: &'a BTreeMap<String, CompactContractBytecode>,
    // fname => Vec<(fname, file, key)>
    dependency_tree: &'a BTreeMap<String, Vec<(String, String, String)>>,
    // library deployment vector (file:contract:address, bytecode)
    deployment: &'a mut Vec<(String, Bytes)>,
    // deployed library addresses fname => adddress
    deployed_library_addresses: &'a Libraries,
    // nonce to start at
    init_nonce: U256,
    // sender
    sender: Address,
) {
    // check if we have dependencies
    if let Some(dependencies) = dependency_tree.get(&target) {
        trace!(?target, target = "forge::link", "linking contract");
        // for each dependency, try to link
        dependencies.iter().for_each(|(next_target, file, key)| {
            // get the dependency
            trace!(dependency = next_target, file, key, target = "forge::link", "get dependency");
            let contract = contracts
                .get(next_target)
                .unwrap_or_else(|| panic!("No target contract named {}", next_target))
                .clone();
            let mut next_target_bytecode = contract
                .bytecode
                .unwrap_or_else(|| panic!("No bytecode for contract {}", next_target));
            let mut next_target_runtime_bytecode = contract
                .deployed_bytecode
                .expect("No target runtime bytecode")
                .bytecode
                .expect("No target runtime");

            // make sure dependency is fully linked
            if let Some(deps) = dependency_tree.get(next_target) {
                if !deps.is_empty() {
                    // actually link the nested dependencies to this dependency
                    recurse_link(
                        next_target.to_string(),
                        (&mut next_target_bytecode, &mut next_target_runtime_bytecode),
                        contracts,
                        dependency_tree,
                        deployment,
                        deployed_library_addresses,
                        init_nonce,
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

            let address = deployed_address.unwrap_or_else(|| {
                ethers_core::utils::get_contract_address(sender, init_nonce + deployment.len())
            });

            // link the dependency to the target
            target_bytecode.0.link(file.clone(), key.clone(), address);
            target_bytecode.1.link(file.clone(), key.clone(), address);

            if deployed_address.is_none() {
                let library = format!("{}:{}:0x{}", file, key, hex::encode(address));

                // push the dependency into the library deployment vector
                deployment.push((
                    library,
                    next_target_bytecode.object.into_bytes().expect("Bytecode should be linked"),
                ));
            }
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
                writeln!(&mut s, "{: <20} {}\n", k, v).expect("could not write k/v to table");
            }
            s
        }
        _ => "".to_owned(),
    }
}

/// Resolves an input to [`NameOrAddress`]. The input could also be a contract/token name supported
/// by
/// [`ethers-addressbook`](https://github.com/gakonst/ethers-rs/tree/master/ethers-addressbook).
pub fn resolve_addr<T: Into<NameOrAddress>>(
    to: T,
    chain: Option<Chain>,
) -> eyre::Result<NameOrAddress> {
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

/// Enables tracing
#[cfg(any(feature = "test"))]
pub fn init_tracing_subscriber() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
}

pub async fn next_nonce(
    caller: Address,
    provider_url: &str,
    block: Option<BlockId>,
) -> Result<U256> {
    let provider = Provider::try_from(provider_url)
        .wrap_err_with(|| format!("Bad fork_url provider: {}", provider_url))?;
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
                        assert_eq!(hex::encode(&post_link_input.dependencies[0].1), lib_linked);
                        assert_eq!(hex::encode(&post_link_input.dependencies[1].1), lib_linked);
                        assert_ne!(
                            hex::encode(&post_link_input.dependencies[2].1),
                            *nested_lib_unlinked
                        );
                    }
                    "Lib.json:Lib" => {
                        assert_eq!(post_link_input.dependencies.len(), 0);
                    }
                    "NestedLib.json:NestedLib" => {
                        assert_eq!(post_link_input.dependencies.len(), 1);
                        assert_eq!(hex::encode(&post_link_input.dependencies[0].1), lib_linked);
                    }
                    "LibraryConsumer.json:LibraryConsumer" => {
                        assert_eq!(post_link_input.dependencies.len(), 3);
                        assert_eq!(hex::encode(&post_link_input.dependencies[0].1), lib_linked);
                        assert_eq!(hex::encode(&post_link_input.dependencies[1].1), lib_linked);
                        assert_ne!(
                            hex::encode(&post_link_input.dependencies[2].1),
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
