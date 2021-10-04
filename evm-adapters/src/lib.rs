#[cfg(feature = "sputnik")]
/// Abstraction over [Sputnik EVM](https://github.com/rust-blockchain/evm)
pub mod sputnik;

/// Abstraction over [evmodin](https://github.com/rust-blockchain/evm)
#[cfg(feature = "evmodin")]
pub mod evmodin;

mod blocking_provider;
pub use blocking_provider::BlockingProvider;

pub mod fuzz;

use ethers::{
    abi::{Detokenize, Tokenize},
    core::types::{Address, U256},
    prelude::{decode_function_data, encode_function_data, AbiError, Bytes},
};

use dapp_utils::IntoFunction;

use eyre::Result;

#[derive(thiserror::Error, Debug)]
pub enum EvmError {
    #[error(transparent)]
    Eyre(#[from] eyre::Error),
    #[error("Execution reverted: {reason}, (gas: {gas_used})")]
    Execution { reason: String, gas_used: u64 },
    #[error(transparent)]
    AbiError(#[from] ethers::contract::AbiError),
}

// TODO: Any reason this should be an async trait?
/// Low-level abstraction layer for interfacing with various EVMs. Once instantiated, one
/// only needs to specify the transaction parameters
pub trait Evm<State> {
    /// The returned reason type from an EVM (Success / Revert/ Stopped etc.)
    type ReturnReason: std::fmt::Debug + PartialEq;

    /// Gets the revert reason type
    fn revert() -> Self::ReturnReason;

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
    fn call<D: Detokenize, T: Tokenize, F: IntoFunction>(
        &mut self,
        from: Address,
        to: Address,
        func: F,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
    ) -> std::result::Result<(D, Self::ReturnReason, u64), EvmError> {
        let func = func.into();
        let calldata = encode_function_data(&func, args)?;
        #[allow(deprecated)]
        let is_static = func.constant ||
            matches!(
                func.state_mutability,
                ethers::abi::StateMutability::View | ethers::abi::StateMutability::Pure
            );
        let (retdata, status, gas) = self.call_raw(from, to, calldata, value, is_static)?;

        if Self::is_fail(&status) {
            let reason = dapp_utils::decode_revert(retdata.as_ref()).map_err(AbiError::from)?;
            Err(EvmError::Execution { reason, gas_used: gas })
        } else {
            let retdata = decode_function_data(&func, retdata, false)?;
            Ok((retdata, status, gas))
        }
    }

    fn call_raw(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        is_static: bool,
    ) -> Result<(Bytes, Self::ReturnReason, u64)>;

    /// Runs the `setUp()` function call to instantiate the contract's state
    fn setup(&mut self, address: Address) -> Result<Self::ReturnReason> {
        let (_, status, _) =
            self.call::<(), _, _>(Address::zero(), address, "setUp()", (), 0.into())?;
        // debug_assert_eq!(status, ExitReason::Succeed(ExitSucceed::Stopped));
        Ok(status)
    }

    /// Runs the `failed()` function call to inspect the test contract's state and
    /// see whether the `failed` state var is set. This is to allow compatibility
    /// with dapptools-style DSTest smart contracts to preserve emiting of logs
    fn failed(&mut self, address: Address) -> Result<bool> {
        let (failed, _, _) =
            self.call::<bool, _, _>(Address::zero(), address, "failed()(bool)", (), 0.into())?;
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
            .call::<(), _, _>(Address::zero(), addr, "greet(string)", "hi".to_owned(), 0.into())
            .unwrap();

        let (retdata, status2, _) = evm
            .call::<String, _, _>(Address::zero(), addr, "greeting()(string)", (), 0.into())
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
        let status1 = evm.setup(addr).unwrap();

        let (_, status2, _) =
            evm.call::<(), _, _>(Address::zero(), addr, "testGreeting()", (), 0.into()).unwrap();

        vec![status1, status2].iter().for_each(|reason| {
            let res = evm.check_success(addr, reason, false);
            assert!(res);
        });

        // TODO: Add testFail
    }
}
