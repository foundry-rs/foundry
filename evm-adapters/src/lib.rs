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
    utils::CompiledContract,
};

use dapp_utils::get_func;
use eyre::Result;

// TODO: Any reason this should be an async trait?
/// Low-level abstraction layer for interfacing with various EVMs. Once instantiated, one
/// only needs to specify the transaction parameters
pub trait Evm<State> // where
//     State: From<Vec<(Address, Bytes)>>,
{
    /// The returned reason type from an EVM (Success / Revert/ Stopped etc.)
    type ReturnReason: std::fmt::Debug;
    // /// The type of the EVM state (can be an in-memory db, or a state db)
    // type State;
    // /// The EVM environment e.g. timestamp, tx.origin etc.
    // type Environment;

    fn reset(&mut self, state: State);

    fn init_state(&self) -> State
    where
        State: Clone;

    fn load_contract_info(&mut self, contract: CompiledContract);

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
        reason: Self::ReturnReason,
        should_fail: bool,
    ) -> bool;

    // TODO: Should we add a "deploy contract" function as well, or should we assume that
    // the EVM is instantiated with a DB that includes any needed contracts?
}

#[cfg(test)]
mod test_helpers {
    use super::*;
    use dapp_solc::SolcBuilder;
    use ethers::prelude::Lazy;
    use std::collections::HashMap;

    pub static COMPILED: Lazy<HashMap<String, CompiledContract>> =
        Lazy::new(|| SolcBuilder::new("./testdata/*.sol", &[], &[]).unwrap().build_all().unwrap());

    pub fn can_call_vm_directly<S, E: Evm<S>>(mut evm: E, addr: Address) -> Vec<E::ReturnReason> {
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
        return vec![status1, status2]
    }
}
