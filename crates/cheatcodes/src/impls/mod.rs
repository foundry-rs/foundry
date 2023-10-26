//! Cheatcode implementations.

use crate::CheatcodeDef;
use alloy_primitives::Address;
use revm::EVMData;

#[macro_use]
mod error;
pub use error::{Error, ErrorKind, Result};

pub use foundry_evm_core::{
    backend::{DatabaseError, DatabaseExt, DatabaseResult, LocalForkId, RevertDiagnostic},
    fork::CreateFork,
};

mod config;
pub use config::CheatsConfig;

mod inspector;
pub use inspector::{BroadcastableTransaction, BroadcastableTransactions, Cheatcodes, Context};

mod env;
mod evm;
mod fs;
mod json;
mod script;
mod string;
mod test;
mod utils;

pub use test::expect::ExpectedCallTracker;

/// Cheatcode implementation.
pub(crate) trait Cheatcode: CheatcodeDef {
    /// Applies this cheatcode to the given state.
    ///
    /// Implement this function if you don't need access to the EVM data.
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let _ = state;
        unimplemented!("{}", Self::CHEATCODE.id)
    }

    /// Applies this cheatcode to the given context.
    ///
    /// Implement this function if you need access to the EVM data.
    #[inline(always)]
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        self.apply(ccx.state)
    }

    #[instrument(target = "cheatcodes", name = "apply", level = "trace", skip(ccx), ret)]
    #[inline]
    fn apply_traced<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        debug!("applying {}", Self::CHEATCODE.id);
        self.apply_full(ccx)
    }
}

/// The cheatcode context, used in [`Cheatcode`].
pub(crate) struct CheatsCtxt<'a, 'b, 'c, DB: DatabaseExt> {
    pub(crate) state: &'a mut Cheatcodes,
    pub(crate) data: &'b mut EVMData<'c, DB>,
    pub(crate) caller: Address,
}

impl<DB: DatabaseExt> CheatsCtxt<'_, '_, '_, DB> {
    #[inline]
    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.data.precompiles.contains(address)
    }
}
