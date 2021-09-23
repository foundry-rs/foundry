use ethers::{
    abi::{Detokenize, Function, Tokenize},
    prelude::{decode_function_data, encode_function_data},
    types::{Address, Bytes, U256},
};

use evm::{
    backend::{MemoryAccount, MemoryBackend},
    executor::{MemoryStackState, StackExecutor, StackState, StackSubstateMetadata},
    Config, ExitReason, Handler,
};
use std::collections::BTreeMap;

use eyre::Result;

use crate::remove_extra_costs;

pub type MemoryState = BTreeMap<Address, MemoryAccount>;

// TODO: Check if we can implement this as the base layer of an ethers-provider
// Middleware stack instead of doing RPC calls.
pub struct Executor<'a, S> {
    pub executor: StackExecutor<'a, S>,
    pub gas_limit: u64,
}

// Concrete implementation over the in-memory backend
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

        Self { executor, gas_limit }
    }
}

impl<'a, S: StackState<'a>> Executor<'a, S> {
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
            self.executor.transact_call(from, to, value, calldata.to_vec(), self.gas_limit, vec![]);

        let gas_after = self.executor.gas_left();
        let gas = remove_extra_costs(gas_before - gas_after, calldata.as_ref());

        let retdata = decode_function_data(func, retdata, false)?;

        Ok((retdata, status, gas.as_u64()))
    }
}

/// given an iterator of contract address to contract bytecode, initializes
/// the state with the contract deployed at the specified address
pub fn initialize_contracts<T: IntoIterator<Item = (Address, Bytes)>>(contracts: T) -> MemoryState {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode_revert,
        test_helpers::{new_backend, new_vicinity, COMPILED},
    };
    use dapp_utils::get_func;

    use ethers::utils::id;
    use evm::{ExitReason, ExitRevert, ExitSucceed};

    #[test]
    fn can_call_vm_directly() {
        let cfg = Config::istanbul();
        let compiled = COMPILED.get("Greeter").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
        let state = initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, state);
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

        let compiled = COMPILED.get("GreeterTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
        let state = initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, state);
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

        let compiled = COMPILED.get("GreeterTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
        let state = initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, state);
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

        let compiled = COMPILED.get("GreeterTest").expect("could not find contract");

        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
        let state = initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, state);
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
}
