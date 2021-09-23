#[cfg(feature = "sputnik")]
/// Abstraction over [Sputnik EVM](https://github.com/rust-blockchain/evm)
pub mod sputnik;

use ethers::{
    abi::{Detokenize, Function, Tokenize},
    core::types::{Address, U256},
};

// TODO: Any reason this should be an async trait?
/// Low-level abstraction layer for interfacing with various EVMs. Once instantiated, one
/// only needs to specify the transaction parameters
pub trait Evm {
    type ReturnReason;

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
    ) -> eyre::Result<(D, Self::ReturnReason, u64)>;

    // TODO: Should we add a "deploy contract" function as well, or should we assume that
    // the EVM is instantiated with a DB that includes any needed contracts?
}
