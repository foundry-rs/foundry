#![doc = include_str!("../README.md")]
#[cfg(feature = "sputnik")]
/// Abstraction over [Sputnik EVM](https://github.com/rust-blockchain/evm)
pub mod sputnik;
#[cfg(feature = "sputnik")]
use crate::sputnik::{
    cheatcodes::debugger::DebugArena, sputnik_evm::executor::stack::Log as EvmLog,
};

/// Abstraction over [evmodin](https://github.com/rust-blockchain/evm)
#[cfg(feature = "evmodin")]
pub mod evmodin;

mod blocking_provider;
use crate::call_tracing::CallTraceArena;

pub use blocking_provider::BlockingProvider;

pub mod fuzz;

pub mod call_tracing;

pub mod gas_report;

/// Helpers for easily constructing EVM objects.
pub mod evm_opts;

use ethers::{
    abi::{Abi, Detokenize, Tokenize},
    contract::{decode_function_data, encode_function_data},
    core::types::{Address, Bytes, U256},
};

use foundry_utils::IntoFunction;

use eyre::Result;
use once_cell::sync::Lazy;

/// The account that we use to fund all the deployed contracts
pub static FAUCET_ACCOUNT: Lazy<Address> =
    Lazy::new(|| Address::from_slice(&ethers::utils::keccak256("turbodapp faucet")[12..]));

/// Errors related to the EVM call execution
#[derive(thiserror::Error, Debug)]
pub enum EvmError {
    #[error("Execution reverted: {reason}, (gas: {gas_used})")]
    // TODO: Add proper log printing.
    /// Error which occurred during execution of an EVM transaction
    Execution { reason: String, gas_used: u64, logs: Vec<String> },
    #[error(transparent)]
    /// Error which occurred during ABI encoding / decoding of data
    AbiError(#[from] ethers::contract::AbiError),
    #[error(transparent)]
    /// Any other generic error
    Eyre(#[from] eyre::Error),
}

/// Return type from having executed a call on the EVM state.
#[derive(Debug)]
pub struct CallOutput<D, R> {
    /// Data returned from the call. `Bytes` for a call, `Address` for a deploy.
    pub retdata: D,
    /// Execution status.
    pub status: R,
    /// Gas used by the call.
    pub gas: u64,
    /// Typed logs from the EVM call.
    pub evm_logs: Vec<EvmLog>,
    /// String-encoded logs from the EVM call.
    pub logs: Vec<String>,
}

// TODO: Any reason this should be an async trait?
/// Low-level abstraction layer for interfacing with various EVMs. Once instantiated, one
/// only needs to specify the transaction parameters
pub trait Evm<State> {
    /// The returned reason type from an EVM (Success / Revert/ Stopped etc.)
    type ReturnReason: std::fmt::Debug + PartialEq;

    /// Gets the revert reason type
    fn revert() -> Self::ReturnReason;

    fn expected_revert(&self) -> Option<&[u8]>;

    /// Whether a return reason should be considered successful
    fn is_success(reason: &Self::ReturnReason) -> bool;
    /// Whether a return reason should be considered failing
    fn is_fail(reason: &Self::ReturnReason) -> bool;

    /// Sets the provided contract bytecode at the corresponding addresses
    fn initialize_contracts<I: IntoIterator<Item = (Address, Bytes)>>(&mut self, contracts: I);

    /// Gets a reference to the current state of the EVM
    fn state(&self) -> &State;

    fn code(&self, address: Address) -> Vec<u8>;

    /// Gets the balance at the specified address
    fn get_balance(&self, address: Address) -> U256;

    /// Gets the nonce at the specified address
    fn get_nonce(&self, address: Address) -> U256;

    /// Sets the balance at the specified address
    fn set_balance(&mut self, address: Address, amount: U256);

    /// Resets the EVM's state to the provided value
    fn reset(&mut self, state: State);

    /// Turns on/off tracing, returning the previously set value
    fn set_tracing_enabled(&mut self, enabled: bool) -> bool;

    /// Returns whether tracing is enabled
    fn tracing_enabled(&self) -> bool;

    /// Grabs debug steps
    #[cfg(feature = "sputnik")]
    fn debug_calls(&self) -> Vec<DebugArena>;

    /// Gets all logs from the execution, regardless of reverts
    fn all_logs(&self) -> Vec<String>;

    /// Performs a [`call_unchecked`](Self::call_unchecked), checks if execution reverted, and
    /// proceeds to return the decoded response to the user.
    fn call<D: Detokenize, T: Tokenize, F: IntoFunction>(
        &mut self,
        from: Address,
        to: Address,
        func: F,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
        abi: Option<&Abi>,
    ) -> std::result::Result<CallOutput<D, Self::ReturnReason>, EvmError> {
        let func = func.into();
        let call_output = self.call_unchecked(from, to, &func, args, value)?;
        if Self::is_fail(&call_output.status) {
            let reason =
                foundry_utils::decode_revert(call_output.retdata.as_ref(), abi).unwrap_or_default();
            Err(EvmError::Execution { reason, gas_used: call_output.gas, logs: call_output.logs })
        } else {
            let retdata = decode_function_data(&func, call_output.retdata, false)?;
            Ok(CallOutput {
                retdata,
                status: call_output.status,
                gas: call_output.gas,
                evm_logs: call_output.evm_logs,
                logs: call_output.logs,
            })
        }
    }

    fn traces(&self) -> Vec<CallTraceArena> {
        vec![]
    }

    fn reset_traces(&mut self) {}
    /// Executes the specified EVM call against the state
    // TODO: Should we just make this take a `TransactionRequest` or other more
    // ergonomic type?
    #[tracing::instrument(skip_all, fields(from, to, func = %func.name))]
    fn call_unchecked<T: Tokenize>(
        &mut self,
        from: Address,
        to: Address,
        func: &ethers::abi::Function,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
    ) -> Result<CallOutput<Bytes, Self::ReturnReason>> {
        let calldata = encode_function_data(func, args)?;
        #[allow(deprecated)]
        let is_static = func.constant.unwrap_or_default() ||
            matches!(
                func.state_mutability,
                ethers::abi::StateMutability::View | ethers::abi::StateMutability::Pure
            );
        self.call_raw(from, to, calldata, value, is_static)
    }

    fn call_raw(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        is_static: bool,
    ) -> Result<CallOutput<Bytes, Self::ReturnReason>>;

    /// Deploys the provided contract bytecode and returns the address
    fn deploy(
        &mut self,
        from: Address,
        calldata: Bytes,
        value: U256,
    ) -> Result<CallOutput<Address, Self::ReturnReason>>;

    /// Runs the `setUp()` function call to instantiate the contract's state
    fn setup(&mut self, address: Address) -> Result<(Self::ReturnReason, Vec<String>)> {
        let span = tracing::trace_span!("setup", ?address);
        let _enter = span.enter();
        let output =
            self.call::<(), _, _>(Address::zero(), address, "setUp()", (), 0.into(), None)?;
        Ok((output.status, output.logs))
    }

    /// Runs the `failed()` function call to inspect the test contract's state and
    /// see whether the `failed` state var is set. This is to allow compatibility
    /// with dapptools-style DSTest smart contracts to preserve emitting of logs
    fn failed(&mut self, address: Address) -> Result<bool> {
        let output = self.call::<bool, _, _>(
            Address::zero(),
            address,
            "failed()(bool)",
            (),
            0.into(),
            None,
        )?;
        Ok(output.retdata)
    }

    /// Given a smart contract address, the result type and whether it's expected to fail,
    /// it returns the test's success status
    fn check_success(
        &mut self,
        address: Address,
        reason: &Self::ReturnReason,
        should_fail: bool,
    ) -> bool {
        // Check if the call is successful
        let mut success = Self::is_success(reason);
        // for successful calls, we should also check the ds-test `failed`
        // value
        if success {
            if let Ok(failed) = self.failed(address) {
                success = !failed;
            }
        }
        // check if there is a remaining expected revert
        if self.expected_revert().is_some() {
            success = false;
        }

        // Check Success output: Should Fail vs Success
        //
        //                           Success
        //                -----------------------
        //               |       | false | true  |
        //               | ----------------------|
        // Should Fail   | false | false | true  |
        //               | ----------------------|
        //               | true  | true  | false |
        //                -----------------------
        (should_fail && !success) || (!should_fail && success)
    }

    // TODO: Should we add a "deploy contract" function as well, or should we assume that
    // the EVM is instantiated with a DB that includes any needed contracts?
}

// Test helpers which are generic over EVM implementation
#[cfg(test)]
mod test_helpers {
    use super::*;
    use ethers::{
        prelude::Lazy,
        solc::{artifacts::CompactContractRef, CompilerOutput, Project, ProjectPathsConfig},
    };

    pub static COMPILED: Lazy<CompilerOutput> = Lazy::new(|| {
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        let res = project.compile().unwrap();
        if res.has_compiler_errors() {
            panic!("{}", res);
        }
        res.output()
    });

    pub fn can_call_vm_directly<S, E: Evm<S>>(mut evm: E, compiled: CompactContractRef) {
        let deploy_output =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        let call_output_1 = evm
            .call::<String, _, _>(
                Address::zero(),
                deploy_output.retdata,
                "greet(string)",
                "hi".to_owned(),
                0.into(),
                compiled.abi,
            )
            .unwrap();

        let call_output_2 = evm
            .call::<String, _, _>(
                Address::zero(),
                deploy_output.retdata,
                "greeting()(string)",
                (),
                0.into(),
                compiled.abi,
            )
            .unwrap();
        assert_eq!(call_output_1.retdata, "hi".to_string());

        vec![call_output_1.status, call_output_2.status].iter().for_each(|reason| {
            let res = evm.check_success(deploy_output.retdata, reason, false);
            assert!(res);
        });
    }

    pub fn solidity_unit_test<S, E: Evm<S>>(mut evm: E, compiled: CompactContractRef) {
        let deploy_output =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        // call the setup function to deploy the contracts inside the test
        let status1 = evm.setup(deploy_output.retdata).unwrap().0;

        let call_output_2 = evm
            .call::<(), _, _>(
                Address::zero(),
                deploy_output.retdata,
                "testGreeting()",
                (),
                0.into(),
                compiled.abi,
            )
            .unwrap();

        vec![status1, call_output_2.status].iter().for_each(|reason| {
            let res = evm.check_success(deploy_output.retdata, reason, false);
            assert!(res);
        });

        // TODO: Add testFail
    }
}
