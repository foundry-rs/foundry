use ethers::{
    abi::{self, Detokenize, Function, FunctionExt, Tokenize},
    prelude::{decode_function_data, encode_function_data},
    types::*,
    utils::{CompiledContract, Solc},
};

use evm::backend::{MemoryAccount, MemoryBackend, MemoryVicinity};
use evm::executor::{self, MemoryStackState, StackSubstateMetadata};
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

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use stack_executor::StackExecutor;

mod stack_executor {
    use evm::executor::StackState;

    use super::*;

    #[cfg(feature = "parallel")]
    use std::sync::Mutex;

    /// Thread-safe wrapper around the StackExecutor which can be triggered with a
    /// `parallel` feature flag to compare parallel/serial performance
    pub struct StackExecutor<'a, S> {
        #[cfg(feature = "parallel")]
        executor: Mutex<executor::StackExecutor<'a, S>>,
        #[cfg(not(feature = "parallel"))]
        executor: executor::StackExecutor<'a, S>,
    }

    impl<'a, S: StackState<'a>> StackExecutor<'a, S> {
        pub fn new(state: S, config: &'a Config) -> Self {
            let executor = executor::StackExecutor::new(state, config);
            #[cfg(feature = "parallel")]
            let executor = Mutex::new(executor);
            Self { executor }
        }

        #[cfg(not(feature = "parallel"))]
        pub fn transact_call(
            &mut self,
            caller: H160,
            address: H160,
            value: U256,
            data: Vec<u8>,
            gas_limit: u64,
        ) -> (ExitReason, Vec<u8>) {
            self.executor
                .transact_call(caller, address, value, data.to_vec(), gas_limit)
        }

        #[cfg(feature = "parallel")]
        pub fn transact_call(
            &self,
            caller: H160,
            address: H160,
            value: U256,
            data: Vec<u8>,
            gas_limit: u64,
        ) -> (ExitReason, Vec<u8>) {
            let mut executor = self.executor.lock().unwrap();
            executor.transact_call(caller, address, value, data.to_vec(), gas_limit)
        }
    }
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
    #[cfg(feature = "parallel")]
    pub fn call<D: Detokenize, T: Tokenize>(
        &self,
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

    #[cfg(not(feature = "parallel"))]
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
    ) -> BTreeMap<Address, MemoryAccount> {
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
pub struct TestResult {
    success: bool,
    // TODO: Add gas consumption if possible?
}

pub struct ContractRunner<'a, S> {
    #[cfg(feature = "parallel")]
    executor: &'a Executor<'a, S>,
    #[cfg(not(feature = "parallel"))]
    executor: &'a mut Executor<'a, S>,

    contract: &'a CompiledContract,
    address: Address,
}

#[cfg(not(feature = "parallel"))]
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
            .filter(|func| func.name.starts_with("test"))
            .collect::<Vec<_>>();

        #[cfg(feature = "parallel")]
        let test_fns = test_fns.par_iter();
        #[cfg(not(feature = "parallel"))]
        let test_fns = test_fns.iter();

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

#[cfg(feature = "parallel")]
impl<'a> ContractRunner<'a, MemoryStackState<'a, 'a, MemoryBackend<'a>>> {
    /// Runs the `setUp()` function call to initiate the contract's state
    fn setup(&self) -> Result<()> {
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
    pub fn test(&self) -> Result<HashMap<String, TestResult>> {
        let test_fns = self
            .contract
            .abi
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"))
            .collect::<Vec<_>>();

        #[cfg(feature = "parallel")]
        let test_fns = test_fns.par_iter();
        #[cfg(not(feature = "parallel"))]
        let test_fns = test_fns.iter();

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

    pub fn test_func(&self, func: &Function) -> TestResult {
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

pub fn decode_revert(error: &[u8]) -> Result<String> {
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
        let compiled = compiled.get("Greet").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let dapp = Executor::new(12_000_000, &cfg, &backend);
        #[cfg(not(feature = "parallel"))]
        let mut dapp = dapp;

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
        let compiled = compiled.get("GreetTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let dapp = Executor::new(12_000_000, &cfg, &backend);
        #[cfg(not(feature = "parallel"))]
        let mut dapp = dapp;

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
        let compiled = compiled.get("GreetTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let dapp = Executor::new(12_000_000, &cfg, &backend);
        #[cfg(not(feature = "parallel"))]
        let mut dapp = dapp;

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
        let compiled = compiled.get("GreetTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let dapp = Executor::new(12_000_000, &cfg, &backend);
        #[cfg(not(feature = "parallel"))]
        let mut dapp = dapp;

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
    // TODO: This still fails.
    fn test_runner() {
        let cfg = Config::istanbul();

        let compiled = Solc::new(&format!("./*.sol")).build().unwrap();
        let compiled = compiled.get("GreetTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let state = Executor::initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = Executor::new_vicinity();
        let backend = Executor::new_backend(&vicinity, state);
        let dapp = Executor::new(12_000_000, &cfg, &backend);
        #[cfg(not(feature = "parallel"))]
        let mut dapp = dapp;

        let runner = ContractRunner {
            #[cfg(feature = "parallel")]
            executor: &dapp,
            #[cfg(not(feature = "parallel"))]
            executor: &mut dapp,
            contract: compiled,
            address: addr,
        };
        #[cfg(not(feature = "parallel"))]
        let mut runner = runner;

        let res = runner.test().unwrap();
        assert!(res.iter().all(|(_, result)| result.success == true));
    }
}
