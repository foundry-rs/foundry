//! # foundry-cheatcodes
//!
//! Foundry cheatcodes implementations.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![allow(elided_lifetimes_in_paths)] // Cheats context uses 3 lifetimes

#[macro_use]
pub extern crate foundry_cheatcodes_spec as spec;
#[macro_use]
extern crate tracing;

use alloy_primitives::Address;
use foundry_evm_core::backend::DatabaseExt;
use revm::{ContextPrecompiles, InnerEvmContext};

pub use config::CheatsConfig;
pub use error::{Error, ErrorKind, Result};
pub use inspector::{
    BroadcastableTransaction, BroadcastableTransactions, Cheatcodes, CheatcodesExecutor, Context,
};
pub use spec::{CheatcodeDef, Vm};
pub use Vm::ForgeContext;

#[macro_use]
mod error;

mod base64;

mod config;

mod env;
pub use env::set_execution_context;

mod evm;

mod fs;

mod inspector;

mod json;

mod script;
pub use script::{ScriptWallets, ScriptWalletsInner};

mod string;

mod test;
pub use test::expect::ExpectedCallTracker;

mod toml;

mod utils;

/// Cheatcode implementation.
pub(crate) trait Cheatcode: CheatcodeDef + DynCheatcode {
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
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        self.apply(ccx.state)
    }

    /// Applies this cheatcode to the given context and executor.
    ///
    /// Implement this function if you need access to the executor.
    fn apply_full_with_executor<DB: DatabaseExt, E: CheatcodesExecutor>(
        &self,
        ccx: &mut CheatsCtxt<DB>,
        _executor: &mut E,
    ) -> Result {
        self.apply_full(ccx)
    }

    #[inline]
    fn apply_traced<DB: DatabaseExt, E: CheatcodesExecutor>(
        &self,
        ccx: &mut CheatsCtxt<DB>,
        executor: &mut E,
    ) -> Result {
        let _span = trace_span_and_call(self);
        let result = self.apply_full_with_executor(ccx, executor);
        trace_return(&result);
        return result;

        // Separate and non-generic functions to avoid inline and monomorphization bloat.
        #[inline(never)]
        fn trace_span_and_call(cheat: &dyn DynCheatcode) -> tracing::span::EnteredSpan {
            let span = debug_span!(target: "cheatcodes", "apply");
            if !span.is_disabled() {
                if enabled!(tracing::Level::TRACE) {
                    span.record("cheat", tracing::field::debug(cheat.as_debug()));
                } else {
                    span.record("id", cheat.cheatcode().func.id);
                }
            }
            let entered = span.entered();
            trace!(target: "cheatcodes", "applying");
            entered
        }

        #[inline(never)]
        fn trace_return(result: &Result) {
            trace!(
                target: "cheatcodes",
                return = match result {
                    Ok(b) => hex::encode(b),
                    Err(e) => e.to_string(),
                }
            );
        }
    }
}

pub(crate) trait DynCheatcode {
    fn cheatcode(&self) -> &'static foundry_cheatcodes_spec::Cheatcode<'static>;
    fn as_debug(&self) -> &dyn std::fmt::Debug;
}

impl<T: Cheatcode> DynCheatcode for T {
    fn cheatcode(&self) -> &'static foundry_cheatcodes_spec::Cheatcode<'static> {
        T::CHEATCODE
    }

    fn as_debug(&self) -> &dyn std::fmt::Debug {
        self
    }
}

/// The cheatcode context, used in [`Cheatcode`].
pub(crate) struct CheatsCtxt<'cheats, 'evm, DB: DatabaseExt> {
    /// The cheatcodes inspector state.
    pub(crate) state: &'cheats mut Cheatcodes,
    /// The EVM data.
    pub(crate) ecx: &'evm mut InnerEvmContext<DB>,
    /// The precompiles context.
    pub(crate) precompiles: &'evm mut ContextPrecompiles<DB>,
    /// The original `msg.sender`.
    pub(crate) caller: Address,
    /// Gas limit of the current cheatcode call.
    pub(crate) gas_limit: u64,
}

impl<'cheats, 'evm, 'db, DB: DatabaseExt> std::ops::Deref for CheatsCtxt<'cheats, 'evm, DB> {
    type Target = InnerEvmContext<DB>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.ecx
    }
}

impl<'cheats, 'evm, 'db, DB: DatabaseExt> std::ops::DerefMut for CheatsCtxt<'cheats, 'evm, DB> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.ecx
    }
}

impl<'cheats, 'evm, 'db, DB: DatabaseExt> CheatsCtxt<'cheats, 'evm, DB> {
    #[inline]
    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.precompiles.contains_key(address)
    }
}
