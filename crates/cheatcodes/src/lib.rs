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
use spec::Status;

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

mod crypto;

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
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        self.apply(ccx.state)
    }

    /// Applies this cheatcode to the given context and executor.
    ///
    /// Implement this function if you need access to the executor.
    #[inline(always)]
    fn apply_full<DB: DatabaseExt, E: CheatcodesExecutor>(
        &self,
        ccx: &mut CheatsCtxt<DB>,
        executor: &mut E,
    ) -> Result {
        let _ = executor;
        self.apply_stateful(ccx)
    }
}

pub(crate) trait DynCheatcode {
    fn name(&self) -> &'static str;
    fn id(&self) -> &'static str;
    fn signature(&self) -> &'static str;
    fn status(&self) -> &Status<'static>;
    fn as_debug(&self) -> &dyn std::fmt::Debug;
}

impl<T: Cheatcode> DynCheatcode for T {
    fn name(&self) -> &'static str {
        T::CHEATCODE.func.signature.split('(').next().unwrap()
    }
    fn id(&self) -> &'static str {
        T::CHEATCODE.func.id
    }
    fn signature(&self) -> &'static str {
        T::CHEATCODE.func.signature
    }
    fn status(&self) -> &Status<'static> {
        &T::CHEATCODE.status
    }
    fn as_debug(&self) -> &dyn std::fmt::Debug {
        self
    }
}

/// The cheatcode context, used in `Cheatcode`.
pub struct CheatsCtxt<'cheats, 'evm, DB: DatabaseExt> {
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

impl<'cheats, 'evm, DB: DatabaseExt> std::ops::Deref for CheatsCtxt<'cheats, 'evm, DB> {
    type Target = InnerEvmContext<DB>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.ecx
    }
}

impl<'cheats, 'evm, DB: DatabaseExt> std::ops::DerefMut for CheatsCtxt<'cheats, 'evm, DB> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.ecx
    }
}

impl<'cheats, 'evm, DB: DatabaseExt> CheatsCtxt<'cheats, 'evm, DB> {
    #[inline]
    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.precompiles.contains(address)
    }
}
