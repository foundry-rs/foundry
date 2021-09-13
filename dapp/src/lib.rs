use ethers::{
    abi::{self, Detokenize, Function, FunctionExt, Tokenize},
    prelude::{decode_function_data, encode_function_data},
    types::*,
    utils::{keccak256, CompiledContract, Solc},
};

use evm::backend::{MemoryAccount, MemoryBackend, MemoryVicinity};
use evm::executor::{MemoryStackState, StackExecutor, StackSubstateMetadata};
use evm::{Config, Handler};
use evm::{ExitReason, ExitRevert, ExitSucceed};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

/// Re-export of the Rust EVM for convenience
pub use evm;

use eyre::Result;
use regex::Regex;

use dapp_utils::get_func;

// TODO: Check if we can implement this as the base layer of an ethers-provider
// Middleware stack instead of doing RPC calls.
pub struct Executor<'a, S> {
    executor: StackExecutor<'a, S>,
    gas_limit: u64,
}

type MemoryState = BTreeMap<Address, MemoryAccount>;

impl<'a> Executor<'a, MemoryStackState<'a, 'a, MemoryBackend<'a>>> {
    /// Given a gas limit, vm version, initial chain configuration and initial state
    // TOOD: See if we can make lifetimes better here
    pub fn new(
        gas_limit: u64,
        config: &'a Config,
        backend: &'a MemoryBackend<'a>,
    ) -> Executor<'a, MemoryStackState<'a, 'a, MemoryBackend<'a>>> {
        // setup gasometer
        let metadata = StackSubstateMetadata::new(gas_limit, config);
        // setup state
        let state = MemoryStackState::new(metadata, backend);
        // setup executor
        let executor = StackExecutor::new_with_precompile(state, config, Default::default());

        Self {
            executor,
            gas_limit,
        }
    }

    /// Runs the selected function
    pub fn call<D: Detokenize, T: Tokenize>(
        &mut self,
        from: Address,
        to: Address,
        func: &Function,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
    ) -> Result<(D, ExitReason, u64)> {
        let calldata = encode_function_data(func, args)?;

        let gas_before = self.executor.gas_left();

        let (status, retdata) =
            self.executor
                .transact_call(from, to, value, calldata.to_vec(), self.gas_limit, vec![]);

        let gas_after = self.executor.gas_left();
        let gas = remove_extra_costs(gas_before - gas_after, calldata.as_ref());

        let retdata = decode_function_data(func, retdata, false)?;

        Ok((retdata, status, gas.as_u64()))
    }

    /// given an iterator of contract address to contract bytecode, initializes
    /// the state with the contract deployed at the specified address
    pub fn initialize_contracts<T: IntoIterator<Item = (Address, Bytes)>>(
        contracts: T,
    ) -> MemoryState {
        contracts
            .into_iter()
            .map(|(address, bytecode)| {
                (
                    address,
                    MemoryAccount {
                        nonce: U256::one(),
                        balance: U256::zero(),
                        storage: BTreeMap::new(),
                        code: bytecode.to_vec(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>()
    }

    pub fn new_vicinity() -> MemoryVicinity {
        MemoryVicinity {
            gas_price: U256::zero(),
            origin: H160::default(),
            block_hashes: Vec::new(),
            block_number: Default::default(),
            block_coinbase: Default::default(),
            block_timestamp: Default::default(),
            block_difficulty: Default::default(),
            block_gas_limit: Default::default(),
            chain_id: U256::one(),
        }
    }

    pub fn new_backend(vicinity: &MemoryVicinity, state: MemoryState) -> MemoryBackend<'_> {
        MemoryBackend::new(vicinity, state)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    // TODO: Ensure that this is calculated properly
    pub gas_used: u64,
}

struct ContractRunner<'a, S> {
    executor: &'a mut Executor<'a, S>,
    contract: &'a CompiledContract,
    address: Address,
}

impl<'a> ContractRunner<'a, MemoryStackState<'a, 'a, MemoryBackend<'a>>> {
    /// Runs the `setUp()` function call to initiate the contract's state
    fn setup(&mut self) -> Result<()> {
        let (_, status, _) = self.executor.call::<(), _>(
            Address::zero(),
            self.address,
            &get_func("function setUp() external").unwrap(),
            (),
            0.into(),
        )?;
        debug_assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));
        Ok(())
    }

    /// runs all tests under a contract
    pub fn test(&mut self, regex: &Regex) -> Result<HashMap<String, TestResult>> {
        let test_fns = self
            .contract
            .abi
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"))
            .filter(|func| regex.is_match(&func.name));

        // run all tests
        let map = test_fns
            .map(|func| {
                // call the setup function in each test to reset the test's state.
                // if we did this outside the map, we'd not have test isolation
                self.setup()?;

                let result = self.test_func(func);
                Ok((func.name.clone(), result))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        Ok(map)
    }

    pub fn test_func(&mut self, func: &Function) -> TestResult {
        // the expected result depends on the function name
        let expected = if func.name.contains("testFail") {
            ExitReason::Revert(ExitRevert::Reverted)
        } else {
            ExitReason::Succeed(ExitSucceed::Stopped)
        };

        // set the selector & execute the call
        let calldata = func.selector();

        let gas_before = self.executor.executor.gas_left();
        let (result, _) = self.executor.executor.transact_call(
            Address::zero(),
            self.address,
            0.into(),
            calldata.to_vec(),
            self.executor.gas_limit,
            vec![],
        );
        let gas_after = self.executor.executor.gas_left();

        TestResult {
            success: expected == result,
            // We subtract the calldata & base gas cost from our test's
            // gas consumption
            gas_used: remove_extra_costs(gas_before - gas_after, &calldata).as_u64(),
        }
    }
}

const BASE_TX_COST: u64 = 21000;
fn remove_extra_costs(gas: U256, calldata: &[u8]) -> U256 {
    let mut calldata_cost = 0;
    for i in calldata {
        if *i != 0 {
            // TODO: Check if EVM pre-eip2028 and charge 64
            calldata_cost += 16
        } else {
            calldata_cost += 8;
        }
    }
    gas - calldata_cost - BASE_TX_COST
}

pub fn decode_revert(error: &[u8]) -> Result<String> {
    Ok(abi::decode(&[abi::ParamType::String], &error[4..])?[0].to_string())
}

pub struct MultiContractRunner<'a> {
    pub contracts: HashMap<String, CompiledContract>,
    pub addresses: HashMap<String, Address>,
    pub config: &'a Config,
    /// The blockchain environment (chain_id, gas_price, block gas limit etc.)
    // TODO: The DAPP_XXX env vars should allow instantiating this via the cli
    pub env: MemoryVicinity,
    /// The initial blockchain state. All test contracts get inserted here at
    /// initialization.
    pub init_state: MemoryState,
    pub state: MemoryState,
    pub gas_limit: u64,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DapptoolsArtifact {
    contracts: HashMap<String, HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Contract {
    abi: ethers::abi::Abi,
    evm: Evm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Evm {
    bytecode: Bytecode,
    deployed_bytecode: Bytecode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bytecode {
    #[serde(deserialize_with = "deserialize_bytes")]
    object: Bytes,
}

use serde::Deserializer;

pub fn deserialize_bytes<'de, D>(d: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(d)?;

    Ok(hex::decode(&value)
        .map_err(|e| serde::de::Error::custom(e.to_string()))?
        .into())
}

impl DapptoolsArtifact {
    fn contracts(&self) -> Result<HashMap<String, CompiledContract>> {
        let mut map = HashMap::new();
        for (key, value) in &self.contracts {
            for (contract, data) in value.iter() {
                let data: Contract = serde_json::from_value(data.clone())?;
                let data = CompiledContract {
                    abi: data.abi,
                    bytecode: data.evm.bytecode.object,
                    runtime_bytecode: data.evm.deployed_bytecode.object,
                };
                map.insert(format!("{}:{}", key, contract), data);
            }
        }

        Ok(map)
    }
}

pub fn installed_version_paths() -> Result<Vec<PathBuf>> {
    let home_dir = svm::SVM_HOME.clone();
    let mut versions = vec![];
    for version in std::fs::read_dir(home_dir)? {
        let version = version?;
        versions.push(version.path());
    }

    versions.sort();
    Ok(versions)
}

/// Supports building contracts
struct SolcBuilder<'a> {
    contracts: &'a str,
    remappings: &'a [String],
    lib_paths: &'a [String],
    versions: Vec<Version>,
    releases: Vec<Version>,
}

use semver::{Version, VersionReq};
use std::fs::File;
use std::io::{BufRead, BufReader};

impl<'a> SolcBuilder<'a> {
    pub fn new(
        contracts: &'a str,
        remappings: &'a [String],
        lib_paths: &'a [String],
    ) -> Result<Self> {
        let versions = svm::installed_versions().unwrap_or_default();
        let releases = tokio::runtime::Runtime::new()?.block_on(svm::all_versions())?;
        Ok(Self {
            contracts,
            remappings,
            lib_paths,
            versions,
            releases,
        })
    }

    /// Gets a map of compiler version -> vec[contract paths]
    fn contract_versions(&mut self) -> Result<HashMap<String, Vec<String>>> {
        // Group contracts in the nones with the same version pragma
        let files = glob::glob(self.contracts)?;

        // get all the corresponding contract versions
        Ok(files
            .filter_map(|fname| fname.ok())
            .filter_map(|fname| self.detect_version(fname).ok().flatten())
            .fold(HashMap::new(), |mut map, (version, path)| {
                let entry = map.entry(version.to_string()).or_insert_with(Vec::new);
                entry.push(path);
                map
            }))
    }

    /// Parses the given Solidity file looking for the `pragma` definition and
    /// returns the corresponding SemVer version requirement.
    fn version_req(path: &Path) -> Result<VersionReq> {
        let file = BufReader::new(File::open(path)?);
        let version = file
            .lines()
            .map(|line| line.unwrap())
            .find(|line| line.starts_with("pragma"))
            .ok_or_else(|| eyre::eyre!("{:?} has no version", path))?;
        let version = version
            .replace("pragma solidity ", "")
            .replace(";", "")
            // needed to make it valid semver for things like
            // >=0.4.0 <0.5.0
            .replace(" ", ",");

        Ok(VersionReq::parse(&version)?)
    }

    /// Find a matching local installation for the specified required version
    fn find_matching_installation(
        &self,
        versions: &[Version],
        required_version: &VersionReq,
    ) -> Option<Version> {
        versions
            .iter()
            // filter these out, unneeded artifact from solc-vm-rs
            // .filter(|&version| version != ".global-version")
            .find(|version| required_version.matches(version))
            .cloned()
    }

    /// Given a Solidity file, it detects the latest compiler version which can be used
    /// to build it, and returns it along with its canonicalized path. If the required
    /// compiler version is not installed, it also proceeds to install it.
    fn detect_version(&mut self, fname: PathBuf) -> Result<Option<(Version, String)>> {
        let path = std::fs::canonicalize(&fname)?;

        // detects the required solc version
        let sol_version = Self::version_req(&path)?;

        let path_str = path
            .into_os_string()
            .into_string()
            .map_err(|_| eyre::eyre!("invalid path, maybe not utf-8?"))?;

        // use the installed one, install it if it does not exist
        let res = self
            .find_matching_installation(&self.versions, &sol_version)
            .or_else(|| {
                // Check upstream for a matching install
                self.find_matching_installation(&self.releases, &sol_version)
                    .map(|version| {
                        println!("Installing {}", version);
                        // Blocking call to install it over RPC.
                        tokio::runtime::Runtime::new()
                            .unwrap()
                            .block_on(svm::install(&version))
                            .unwrap();
                        self.versions.push(version.clone());
                        println!("Done!");
                        version
                    })
            })
            .map(|version| (version, path_str));

        Ok(res)
    }

    /// Builds all provided contract files with the specified compiler version.
    /// Assumes that the lib-paths and remappings have already been specified.
    pub fn build(
        &self,
        version: String,
        files: Vec<String>,
    ) -> Result<HashMap<String, CompiledContract>> {
        let mut compiler_path = installed_version_paths()?
            .iter()
            .find(|name| name.to_string_lossy().contains(&version))
            .unwrap()
            .clone();
        compiler_path.push(format!("solc-{}", &version));

        let mut solc = Solc::new_with_paths(files).solc_path(compiler_path);
        let lib_paths = self
            .lib_paths
            .iter()
            .filter(|path| PathBuf::from(path).exists())
            .map(|path| {
                std::fs::canonicalize(path)
                    .unwrap()
                    .into_os_string()
                    .into_string()
                    .unwrap()
            })
            .collect::<Vec<_>>()
            .join(",");
        solc = solc.args(["--allow-paths", &lib_paths]);

        if !self.remappings.is_empty() {
            solc = solc.args(self.remappings)
        }

        Ok(solc.build()?)
    }

    /// Builds all contracts with their corresponding compiler versions
    pub fn build_all(&mut self) -> Result<HashMap<String, CompiledContract>> {
        let contracts_by_version = self.contract_versions()?;
        contracts_by_version
            .into_iter()
            .try_fold(HashMap::new(), |mut map, (version, files)| {
                let res = self.build(version, files)?;
                map.extend(res);
                Ok::<_, eyre::Error>(map)
            })
    }
}

impl<'a> MultiContractRunner<'a> {
    pub fn build(
        contracts: &str,
        remappings: Vec<String>,
        lib_paths: Vec<String>,
        out_path: PathBuf,
        no_compile: bool,
    ) -> Result<HashMap<String, CompiledContract>> {
        // TODO:
        // 1. incremental compilation
        // 2. parallel compilation
        // 3. Hardhat / Truffle-style artifacts
        Ok(if no_compile {
            let out_file = std::fs::read_to_string(out_path)?;
            serde_json::from_str::<DapptoolsArtifact>(&out_file)?.contracts()?
        } else {
            SolcBuilder::new(contracts, &remappings, &lib_paths)?.build_all()?
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        contracts: &str,
        remappings: Vec<String>,
        lib_paths: Vec<String>,
        out_path: PathBuf,
        config: &'a Config,
        gas_limit: u64,
        env: MemoryVicinity,
        no_compile: bool,
    ) -> Result<Self> {
        // 1. compile the contracts
        let contracts = Self::build(contracts, remappings, lib_paths, out_path, no_compile)?;

        // 2. create the initial state
        // TODO: Allow further overriding perhaps?
        let mut addresses = HashMap::new();
        let init_state = contracts
            .iter()
            .map(|(name, compiled)| {
                // make a fake address for the contract, maybe anti-pattern
                let addr = Address::from_slice(&keccak256(&compiled.runtime_bytecode)[..20]);
                addresses.insert(name.clone(), addr);
                (addr, compiled.runtime_bytecode.clone())
            })
            .collect::<Vec<_>>();
        let state = Executor::initialize_contracts(init_state);

        Ok(Self {
            contracts,
            addresses,
            config,
            env,
            init_state: state.clone(),
            state,
            gas_limit,
        })
    }

    /// instantiate an executor with the init state
    // TODO: Is this right? How would we cache results between calls when in
    // forking mode?
    fn backend(&self) -> MemoryBackend<'_> {
        Executor::new_backend(&self.env, self.init_state.clone())
    }

    pub fn test(&self, pattern: Regex) -> Result<HashMap<String, HashMap<String, TestResult>>> {
        // for each compiled contract, get its name, bytecode and address
        // NB: We also have access to the contract's abi. When running the test.
        // Can this be useful for decorating the stacktrace during a revert?
        let contracts = self.contracts.iter();

        let results = contracts
            .map(|(name, contract)| {
                let address = *self
                    .addresses
                    .get(name)
                    .ok_or_else(|| eyre::eyre!("could not find contract address"))?;

                let backend = self.backend();
                let result = self.test_contract(contract, address, backend, &pattern)?;
                Ok((name.clone(), result))
            })
            .filter_map(|x: Result<_>| x.ok())
            .filter_map(|(name, res)| {
                if res.is_empty() {
                    None
                } else {
                    Some((name, res))
                }
            })
            .collect::<HashMap<_, _>>();

        Ok(results)
    }

    fn test_contract(
        &self,
        contract: &CompiledContract,
        address: Address,
        backend: MemoryBackend<'_>,
        pattern: &Regex,
    ) -> Result<HashMap<String, TestResult>> {
        let mut dapp = Executor::new(self.gas_limit, self.config, &backend);
        let mut runner = ContractRunner {
            executor: &mut dapp,
            contract,
            address,
        };

        runner.test(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::utils::id;

    #[test]
    fn can_call_vm_directly() {
        let cfg = Config::istanbul();
        let compiled = SolcBuilder::new("./*.sol", &[], &[])
            .unwrap()
            .build_all()
            .unwrap();
        let compiled = compiled.get("Greeter").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        let (_, status, _) = dapp
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function greet(string greeting) external").unwrap(),
                "hi".to_owned(),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));

        let (retdata, status, _) = dapp
            .call::<String, _>(
                Address::zero(),
                addr,
                &get_func("function greeting() public view returns (string)").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Returned));
        assert_eq!(retdata, "hi");
    }

    #[test]
    fn solidity_unit_test() {
        let cfg = Config::istanbul();

        let compiled = SolcBuilder::new("./*.sol", &[], &[])
            .unwrap()
            .build_all()
            .unwrap();
        let compiled = compiled
            .get("GreeterTest")
            .expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        // call the setup function to deploy the contracts inside the test
        let (_, status, _) = dapp
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function setUp() external").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));

        let (_, status, _) = dapp
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function testGreeting()").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));
    }

    #[test]
    fn failing_with_no_reason_if_no_setup() {
        let cfg = Config::istanbul();

        let compiled = SolcBuilder::new("./*.sol", &[], &[])
            .unwrap()
            .build_all()
            .unwrap();
        let compiled = compiled
            .get("GreeterTest")
            .expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        let (status, res) = dapp.executor.transact_call(
            Address::zero(),
            addr,
            0.into(),
            id("testFailGreeting()").to_vec(),
            dapp.gas_limit,
            vec![],
        );
        assert_eq!(status, ExitReason::Revert(ExitRevert::Reverted));
        assert!(res.is_empty());
    }

    #[test]
    fn failing_solidity_unit_test() {
        let cfg = Config::istanbul();

        let compiled = SolcBuilder::new("./*.sol", &[], &[])
            .unwrap()
            .build_all()
            .unwrap();
        let compiled = compiled
            .get("GreeterTest")
            .expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        // call the setup function to deploy the contracts inside the test
        let (_, status, _) = dapp
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function setUp() external").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));

        let (status, res) = dapp.executor.transact_call(
            Address::zero(),
            addr,
            0.into(),
            id("testFailGreeting()").to_vec(),
            dapp.gas_limit,
            vec![],
        );
        assert_eq!(status, ExitReason::Revert(ExitRevert::Reverted));
        let reason = decode_revert(&res).unwrap();
        assert_eq!(reason, "not equal to `hi`");
    }

    #[test]
    fn test_runner() {
        let cfg = Config::istanbul();

        let compiled = SolcBuilder::new("./*.sol", &[], &[])
            .unwrap()
            .build_all()
            .unwrap();
        let compiled = compiled
            .get("GreeterTest")
            .expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        let mut runner = ContractRunner {
            executor: &mut dapp,
            contract: compiled,
            address: addr,
        };

        let res = runner.test(&".*".parse().unwrap()).unwrap();
        assert!(res.iter().all(|(_, result)| result.success == true));
    }

    #[test]
    fn test_multi_runner() {
        let contracts = "./GreetTest.sol";
        let cfg = Config::istanbul();
        let gas_limit = 12_500_000;
        let env = Executor::new_vicinity();

        let runner = MultiContractRunner::new(
            contracts,
            vec![],
            vec![],
            PathBuf::new(),
            &cfg,
            gas_limit,
            env,
            false,
        )
        .unwrap();
        let results = runner.test(Regex::new(".*").unwrap()).unwrap();
        // 2 contracts
        assert_eq!(results.len(), 2);
        // 3 tests on greeter 1 on gm
        assert_eq!(results["GreeterTest"].len(), 3);
        assert_eq!(results["GmTest"].len(), 1);
        for (_, res) in results {
            assert!(res.iter().all(|(_, result)| result.success == true));
        }

        let only_gm = runner.test(Regex::new("testGm.*").unwrap()).unwrap();
        assert_eq!(only_gm.len(), 1);
        assert_eq!(only_gm["GmTest"].len(), 1);
    }
}
