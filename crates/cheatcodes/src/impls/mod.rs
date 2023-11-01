//! Cheatcode implementations.

use crate::CheatcodeDef;
use alloy_primitives::Address;
use foundry_evm_core::backend::DatabaseExt;
use revm::EVMData;
use tracing::Level;

#[macro_use]
mod error;
pub use error::{Error, ErrorKind, Result};

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
        unimplemented!("{}", Self::CHEATCODE.func.id)
    }

    /// Applies this cheatcode to the given context.
    ///
    /// Implement this function if you need access to the EVM data.
    #[inline(always)]
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        self.apply(ccx.state)
    }

    #[inline]
    fn apply_traced<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let span = trace_span(self);
        let _enter = span.enter();
        trace_call();
        let result = self.apply_full(ccx);
        trace_return(&result);
        result
    }
}

// Separate functions to avoid inline and monomorphization bloat.
fn trace_span<T: Cheatcode>(cheat: &T) -> tracing::Span {
    if enabled!(Level::TRACE) {
        trace_span!(target: "cheatcodes", "apply", cheat=?cheat)
    } else {
        debug_span!(target: "cheatcodes", "apply", id=%T::CHEATCODE.func.id)
    }
}

fn trace_call() {
    trace!(target: "cheatcodes", "applying");
}

fn trace_return(result: &Result) {
    trace!(
        target: "cheatcodes",
        return = match result {
            Ok(b) => hex::encode(b),
            Err(e) => e.to_string(),
        }
    );
}

/// The cheatcode context, used in [`Cheatcode`].
pub(crate) struct CheatsCtxt<'a, 'b, 'c, DB: DatabaseExt> {
    /// The cheatcodes inspector state.
    pub(crate) state: &'a mut Cheatcodes,
    /// The EVM data.
    pub(crate) data: &'b mut EVMData<'c, DB>,
    /// The original `msg.sender`.
    pub(crate) caller: Address,
}

impl<DB: DatabaseExt> CheatsCtxt<'_, '_, '_, DB> {
    #[inline]
    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.data.precompiles.contains(address)
    }
}
