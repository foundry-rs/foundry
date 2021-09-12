use ethers::{
    abi::{self, Detokenize, Function, FunctionExt, Tokenize},
    prelude::{decode_function_data, encode_function_data},
    types::*,
    utils::{keccak256, CompiledContract, Solc},
};

use evm::backend::{MemoryAccount, MemoryBackend, MemoryVicinity};
use evm::executor::{MemoryStackState, StackExecutor, StackSubstateMetadata};
use evm::Config;
use evm::{ExitReason, ExitRevert, ExitSucceed};
use std::collections::{BTreeMap, HashMap};

use eyre::Result;

use crate::utils::get_func;

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
        let executor = StackExecutor::new(state, &config);

        Self {
            executor,
            gas_limit,
        }
    }

    /// builds the contracts & writes the output to out/dapp.out.json
    pub fn build(&self) -> Result<()> {
        // TODO: Set remappings, optimizer runs, config files
        // Set location to read sol contracts from
        let _compiled = Solc::new(&format!("./*.sol")).build()?;
        Ok(())
    }

    /// Runs the selected function
    pub fn call<D: Detokenize, T: Tokenize>(
        &mut self,
        from: Address,
        to: Address,
        func: &Function,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
    ) -> Result<(D, ExitReason)> {
        let data = encode_function_data(&func, args)?;

        let (status, retdata) =
            self.executor
                .transact_call(from, to, value, data.to_vec(), self.gas_limit);

        let retdata = decode_function_data(&func, retdata, false)?;

        Ok((retdata, status))
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

#[derive(Clone, Debug)]
struct TestResult {
    success: bool,
    // TODO: Add gas consumption if possible?
}

struct ContractRunner<'a, S> {
    executor: &'a mut Executor<'a, S>,
    contract: &'a CompiledContract,
    address: Address,
}

impl<'a> ContractRunner<'a, MemoryStackState<'a, 'a, MemoryBackend<'a>>> {
    /// Runs the `setUp()` function call to initiate the contract's state
    fn setup(&mut self) -> Result<()> {
        let (_, status) = self.executor.call::<(), _>(
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
    pub fn test(&mut self) -> Result<HashMap<String, TestResult>> {
        let test_fns = self
            .contract
            .abi
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"));

        // run all tests
        let map = test_fns
            .map(|func| {
                // call the setup function in each test to reset the test's state.
                // if we did this outside the map, we'd not have test isolation
                self.setup()?;

                let result = self.test_func(func);
                println!("{:?}, got {:?}", func.name, result);
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
        let data = func.selector().to_vec();
        let (result, _) = self.executor.executor.transact_call(
            Address::zero(),
            self.address,
            0.into(),
            data.to_vec(),
            self.executor.gas_limit,
        );

        TestResult {
            success: expected == result,
        }
    }
}

fn decode_revert(error: &[u8]) -> Result<String> {
    Ok(abi::decode(&[abi::ParamType::String], &error[4..])?[0].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::utils::id;

    #[test]
    fn can_call_vm_directly() {
        // TODO: Is there a cleaner way to initialize them all together in a function?
        let cfg = Config::istanbul();

        let compiled = Solc::new(&format!("./*.sol")).build().unwrap();
        let compiled = compiled.get("Greeter").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let mut dapp = Executor::new(12_000_000, &cfg, &backend);

        let (_, status) = dapp
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function greet(string greeting) external").unwrap(),
                "hi".to_owned(),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));

        let (retdata, status) = dapp
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

        let compiled = Solc::new(&format!("./*.sol")).build().unwrap();
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
        let (_, status) = dapp
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function setUp() external").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));

        let (_, status) = dapp
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

        let compiled = Solc::new(&format!("./*.sol")).build().unwrap();
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
        );
        assert_eq!(status, ExitReason::Revert(ExitRevert::Reverted));
        assert!(res.is_empty());
    }

    #[test]
    fn failing_solidity_unit_test() {
        let cfg = Config::istanbul();

        let compiled = Solc::new(&format!("./*.sol")).build().unwrap();
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
        let (_, status) = dapp
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
        );
        assert_eq!(status, ExitReason::Revert(ExitRevert::Reverted));
        let reason = decode_revert(&res).unwrap();
        assert_eq!(reason, "not equal to `hi`");
    }

    #[test]
    fn test_runner() {
        let cfg = Config::istanbul();

        let compiled = Solc::new(&format!("./*.sol")).build().unwrap();
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

        let res = runner.test().unwrap();
        assert!(res.iter().all(|(_, result)| result.success == true));
    }
}
