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

use alloy_primitives::Address;
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

/// Trait for defining custom cheatcodes that extend Foundry's built-in set.
///
/// Implement this trait to add new cheatcodes without forking Foundry. External cheatcodes
/// are dispatched when a call to the cheatcode address (`0x7109...`) does not match any
/// built-in cheatcode selector.
///
/// # Return value
///
/// The return type uses a tri-state convention:
/// - `Ok(Some(bytes))` — this handler recognized the selector and succeeded; `bytes` is ABI-encoded
///   return data.
/// - `Ok(None)` — this handler does not recognize the selector; try the next handler.
/// - `Err(e)` — this handler recognized the selector but wants to revert with error `e`.
///
/// # Example
///
/// ```ignore
/// use alloy_primitives::Bytes;
/// use foundry_cheatcodes::ExternalCheatcode;
///
/// struct MyCheatcodes;
///
/// impl ExternalCheatcode for MyCheatcodes {
///     fn call(&self, calldata: &[u8]) -> foundry_cheatcodes::Result<Option<Vec<u8>>> {
///         // Return Ok(None) for selectors you don't handle
///         if calldata.len() < 4 {
///             return Ok(None);
///         }
///         // Decode calldata and implement custom logic
///         Ok(Some(Bytes::new().to_vec()))
///     }
/// }
/// ```
pub trait ExternalCheatcode: Send + Sync + std::fmt::Debug + 'static {
    /// Called when an unknown cheatcode selector is encountered.
    ///
    /// `calldata` contains the full ABI-encoded call data (including the 4-byte selector).
    ///
    /// Return `Ok(Some(ret))` on success, `Ok(None)` if unhandled, or `Err(e)` to revert.
    fn call(&self, calldata: &[u8]) -> Result<Option<Vec<u8>>>;
}
