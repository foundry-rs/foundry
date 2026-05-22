//! # foundry-cheatcodes
//!
//! Foundry cheatcodes implementations.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(elided_lifetimes_in_paths)] // Cheats context uses 3 lifetimes

#[macro_use]
extern crate foundry_common;

#[macro_use]
pub extern crate foundry_cheatcodes_spec as spec;

#[macro_use]
extern crate tracing;

use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::{
    backend::DatabaseExt,
    evm::{FoundryContextFor, FoundryEvmNetwork},
};
use revm::context::{ContextTr, JournalTr};

pub use Vm::ForgeContext;
pub use config::CheatsConfig;
pub use error::{Error, ErrorKind, Result};
pub use foundry_evm_core::evm::NestedEvmClosure;
pub use inspector::{
    BroadcastableTransaction, BroadcastableTransactions, Cheatcodes, CheatcodesExecutor,
};
pub use spec::{CheatcodeDef, Vm};

#[macro_use]
mod error;

mod base64;

mod config;

mod crypto;

mod version;

mod env;
pub use env::set_execution_context;

mod evm;

mod fs;

mod inspector;
pub use inspector::CheatcodeAnalysis;

mod json;

mod script;
pub use script::{Wallets, WalletsInner};

mod string;

mod test;
pub use test::expect::ExpectedCallTracker;

mod toml;

mod utils;

/// Cheatcode implementation.
pub(crate) trait Cheatcode: CheatcodeDef {
    /// Applies this cheatcode to the given state.
    ///
    /// Implement this function if you don't need access to the EVM data.
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let _ = state;
        unimplemented!("{}", Self::CHEATCODE.func.id)
    }

    /// Applies this cheatcode to the given context.
    ///
    /// Implement this function if you need access to the EVM data.
    #[inline(always)]
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        self.apply(ccx.state)
    }

    /// Applies this cheatcode to the given context and executor.
    ///
    /// Implement this function if you need access to the executor.
    #[inline(always)]
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let _ = executor;
        self.apply_stateful(ccx)
    }
}

/// The cheatcode context.
pub struct CheatsCtxt<'a, 'db, FEN: FoundryEvmNetwork + 'db> {
    /// The cheatcodes inspector state.
    pub(crate) state: &'a mut Cheatcodes<FEN>,
    /// The EVM context.
    pub(crate) ecx: &'a mut FoundryContextFor<'db, FEN>,
    /// The original `msg.sender`.
    pub(crate) caller: Address,
    /// Gas limit of the current cheatcode call.
    pub(crate) gas_limit: u64,
}

impl<'a, 'db, FEN: FoundryEvmNetwork> std::ops::Deref for CheatsCtxt<'a, 'db, FEN> {
    type Target = FoundryContextFor<'db, FEN>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.ecx
    }
}

impl<'db, FEN: FoundryEvmNetwork> std::ops::DerefMut for CheatsCtxt<'_, 'db, FEN> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ecx
    }
}

impl<FEN: FoundryEvmNetwork> CheatsCtxt<'_, '_, FEN> {
    /// Returns a mutable reference to the cheatcodes inspector state.
    pub fn state_mut(&mut self) -> &mut Cheatcodes<FEN> {
        self.state
    }

    /// Returns a reference to the cheatcodes inspector state.
    pub fn state(&self) -> &Cheatcodes<FEN> {
        self.state
    }

    /// Returns the original `msg.sender`.
    pub fn caller(&self) -> Address {
        self.caller
    }

    /// Returns the gas limit of the current cheatcode call.
    pub fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    pub(crate) fn ensure_not_precompile(&self, address: &Address) -> Result<()> {
        if self.is_precompile(address) { Err(precompile_error(address)) } else { Ok(()) }
    }

    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.ecx.journal().precompile_addresses().contains(address)
    }
}

#[cold]
fn precompile_error(address: &Address) -> Error {
    fmt_err!("cannot use precompile {address} as an argument")
}

/// The outcome of an external cheatcode call.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExternalCheatcodeOutcome {
    /// This handler does not recognize the selector; try the next handler.
    Unhandled,
    /// The handler recognized the selector and succeeded. Contains ABI-encoded return data.
    Return(Vec<u8>),
    /// The handler recognized the selector but wants to revert with this error.
    Revert(Error),
}

/// Host interface for external cheatcode handlers.
///
/// Provides object-safe, non-generic access to EVM state without exposing
/// Foundry internals or the `FoundryEvmNetwork` type parameter.
pub trait CheatcodeHost {
    /// Returns the original `msg.sender` of the cheatcode call.
    fn caller(&self) -> Address;
    /// Returns the gas limit of the current cheatcode call.
    fn gas_limit(&self) -> u64;

    /// Loads a storage slot value for the given account.
    fn load(&mut self, account: Address, slot: U256) -> Result<U256>;
    /// Returns the balance of the given account.
    fn balance(&mut self, account: Address) -> Result<U256>;

    /// Stores a value into a storage slot. Equivalent to `vm.store`.
    fn store(&mut self, account: Address, slot: U256, value: U256) -> Result<()>;
    /// Sets the balance of an account. Equivalent to `vm.deal`.
    fn set_balance(&mut self, account: Address, value: U256) -> Result<()>;
    /// Sets the runtime bytecode of an account. Equivalent to `vm.etch`.
    fn set_code(&mut self, account: Address, code: Bytes) -> Result<()>;
}

/// Trait for defining custom cheatcodes that extend Foundry's built-in set.
///
/// Implement this trait to add new cheatcodes without forking Foundry. External cheatcodes
/// are dispatched when a call to the cheatcode address (`0x7109...`) does not match any
/// built-in cheatcode selector.
///
/// # Example
///
/// ```ignore
/// use alloy_primitives::Address;
/// use foundry_cheatcodes::{CheatcodeHost, ExternalCheatcode, ExternalCheatcodeOutcome};
///
/// #[derive(Debug)]
/// struct MyCheatcodes;
///
/// impl ExternalCheatcode for MyCheatcodes {
///     fn call(
///         &self,
///         host: &mut dyn CheatcodeHost,
///         calldata: &[u8],
///     ) -> ExternalCheatcodeOutcome {
///         if calldata.len() < 4 {
///             return ExternalCheatcodeOutcome::Unhandled;
///         }
///         // Decode calldata and implement custom logic
///         let balance = host.balance(Address::ZERO);
///         ExternalCheatcodeOutcome::Return(Vec::new())
///     }
/// }
/// ```
pub trait ExternalCheatcode: Send + Sync + std::fmt::Debug + 'static {
    /// Called when an unknown cheatcode selector is encountered.
    ///
    /// `calldata` contains the full ABI-encoded call data (including the 4-byte selector).
    /// Use `host` to read and write EVM state (storage, balances, bytecode).
    fn call(&self, host: &mut dyn CheatcodeHost, calldata: &[u8]) -> ExternalCheatcodeOutcome;
}
