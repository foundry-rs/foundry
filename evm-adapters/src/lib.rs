#[cfg(feature = "sputnik")]
/// Abstraction over [Sputnik EVM](https://github.com/rust-blockchain/evm)
pub mod sputnik;

/// Abstraction over [evmodin](https://github.com/rust-blockchain/evm)
#[cfg(feature = "evmodin")]
pub mod evmodin;

use ethers::{
    abi::{Detokenize, Function, Tokenize},
    core::types::{Address, U256},
    prelude::Bytes,
};

use dapp_utils::get_func;
use eyre::Result;

// TODO: Any reason this should be an async trait?
/// Low-level abstraction layer for interfacing with various EVMs. Once instantiated, one
/// only needs to specify the transaction parameters
pub trait Evm<State> {
    /// The returned reason type from an EVM (Success / Revert/ Stopped etc.)
    type ReturnReason: std::fmt::Debug + PartialEq;

    /// Whether a return reason should be considered successful
    fn is_success(reason: &Self::ReturnReason) -> bool;
    /// Whether a return reason should be considered failing
    fn is_fail(reason: &Self::ReturnReason) -> bool;

    /// Sets the provided contract bytecode at the corresponding addresses
    fn initialize_contracts<I: IntoIterator<Item = (Address, Bytes)>>(&mut self, contracts: I);

    /// Gets a reference to the current state of the EVM
    fn state(&self) -> &State;

    /// Resets the EVM's state to the provided value
    fn reset(&mut self, state: State);

    /// Executes the specified EVM call against the state
    // TODO: Should we just make this take a `TransactionRequest` or other more
    // ergonomic type?
    fn call<D: Detokenize, T: Tokenize>(
        &mut self,
        from: Address,
        to: Address,
        func: &Function,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
    ) -> Result<(D, Self::ReturnReason, u64)>;

    /// Runs the `setUp()` function call to instantiate the contract's state
    fn setup(&mut self, address: Address) -> Result<()> {
        let (_, _, _) = self.call::<(), _>(
            Address::zero(),
            address,
            &get_func("function setUp() external").unwrap(),
            (),
            0.into(),
        )?;
        // debug_assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));
        Ok(())
    }

    /// Runs the `failed()` function call to inspect the test contract's state and
    /// see whether the `failed` state var is set. This is to allow compatibility
    /// with dapptools-style DSTest smart contracts to preserve emiting of logs
    fn failed(&mut self, address: Address) -> Result<bool> {
        let (failed, _, _) = self.call::<bool, _>(
            Address::zero(),
            address,
            &get_func("function failed() returns (bool)").unwrap(),
            (),
            0.into(),
        )?;
        Ok(failed)
    }

    /// Given a smart contract address, the result type and whether it's expected to fail,
    /// it returns the test's success status
    fn check_success(
        &mut self,
        address: Address,
        reason: &Self::ReturnReason,
        should_fail: bool,
    ) -> bool {
        if should_fail {
            if Self::is_success(reason) {
                self.failed(address).unwrap_or(false)
            } else if Self::is_fail(reason) {
                true
            } else {
                tracing::error!(?reason);
                false
            }
        } else {
            Self::is_success(reason)
        }
    }

    // TODO: Should we add a "deploy contract" function as well, or should we assume that
    // the EVM is instantiated with a DB that includes any needed contracts?
}

// Test helpers which are generic over EVM implementation
#[cfg(test)]
mod test_helpers {
    use super::*;
    use dapp_solc::SolcBuilder;
    use ethers::{prelude::Lazy, utils::CompiledContract};
    use std::collections::HashMap;

    pub static COMPILED: Lazy<HashMap<String, CompiledContract>> =
        Lazy::new(|| SolcBuilder::new("./testdata/*.sol", &[], &[]).unwrap().build_all().unwrap());

    pub fn can_call_vm_directly<S, E: Evm<S>>(
        mut evm: E,
        addr: Address,
        compiled: &CompiledContract,
    ) {
        evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let (_, status1, _) = evm
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function greet(string greeting) external").unwrap(),
                "hi".to_owned(),
                0.into(),
            )
            .unwrap();

        let (retdata, status2, _) = evm
            .call::<String, _>(
                Address::zero(),
                addr,
                &get_func("function greeting() public view returns (string)").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(retdata, "hi");

        vec![status1, status2].iter().for_each(|reason| {
            let res = evm.check_success(addr, reason, false);
            assert!(res);
        });
    }

    pub fn solidity_unit_test<S, E: Evm<S>>(
        mut evm: E,
        addr: Address,
        compiled: &CompiledContract,
    ) {
        evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        // call the setup function to deploy the contracts inside the test
        let (_, status1, _) = evm
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function setUp() external").unwrap(),
                (),
                0.into(),
            )
            .unwrap();

        let (_, status2, _) = evm
            .call::<(), _>(
                Address::zero(),
                addr,
                &get_func("function testGreeting()").unwrap(),
                (),
                0.into(),
            )
            .unwrap();

        vec![status1, status2].iter().for_each(|reason| {
            let res = evm.check_success(addr, reason, false);
            assert!(res);
        });

        // TODO: Add testFail
    }
}
