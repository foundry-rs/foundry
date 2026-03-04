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

use alloy_evm::eth::EthEvmContext;
use alloy_primitives::Address;
use foundry_evm_core::backend::DatabaseExt;
use revm::context::{ContextTr, JournalTr};

pub use Vm::ForgeContext;
pub use config::CheatsConfig;
pub use error::{Error, ErrorKind, Result};
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
pub(crate) trait Cheatcode<CTX>: CheatcodeDef {
    /// Applies this cheatcode to the given state.
    ///
    /// Implement this function if you don't need access to the EVM data.
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let _ = state;
        unimplemented!("{}", Self::CHEATCODE.func.id)
    }

    /// Applies this cheatcode to the given context.
    ///
    /// Implement this function if you need access to the EVM data.
    #[inline(always)]
    fn apply_stateful(&self, ccx: &mut CheatsCtxt<'_, CTX>) -> Result {
        self.apply(ccx.state)
    }

    /// Applies this cheatcode to the given context and executor.
    ///
    /// Implement this function if you need access to the executor.
    #[inline(always)]
    fn apply_full(
        &self,
        ccx: &mut CheatsCtxt<'_, CTX>,
        executor: &mut dyn CheatcodesExecutor,
    ) -> Result {
        let _ = executor;
        self.apply_stateful(ccx)
    }
}

/// The cheatcode context.
pub struct CheatsCtxt<'a, CTX> {
    /// The cheatcodes inspector state.
    pub(crate) state: &'a mut Cheatcodes,
    /// The EVM context.
    pub(crate) ecx: &'a mut CTX,
    /// The original `msg.sender`.
    pub(crate) caller: Address,
    /// Gas limit of the current cheatcode call.
    pub(crate) gas_limit: u64,
}

/// Concrete [`CheatsCtxt`] for the Ethereum EVM context.
pub type EthCheatsCtxt<'a, 'db, 'db2> =
    CheatsCtxt<'a, EthEvmContext<&'db mut (dyn DatabaseExt + 'db2)>>;

/// Placeholder context type for cheatcodes that don't need EVM context access
/// (i.e., they only use `apply`, not `apply_stateful` or `apply_full`).
#[cfg(test)]
pub(crate) type AnyCtx = ();

impl<CTX> std::ops::Deref for CheatsCtxt<'_, CTX> {
    type Target = CTX;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.ecx
    }
}

impl<CTX> std::ops::DerefMut for CheatsCtxt<'_, CTX> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ecx
    }
}

impl<CTX: ContextTr> CheatsCtxt<'_, CTX> {
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
