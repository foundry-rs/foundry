//! Block access list (BAL) reads for executing one transaction against its parent block's state.

use super::Backend;
use crate::evm::{FoundryEvmFor, FoundryEvmNetwork};
use evm2::evm::bal::{Bal, BlockAccessIndex};
use std::sync::Arc;

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Serves reads of state the block wrote before `index` from `bal`, and everything else from
    /// the underlying database.
    ///
    /// Index `0` holds the pre-block system writes and transaction `i` is index `i + 1`, so
    /// positioning the reads at a transaction's own index yields its prestate without replaying
    /// the earlier transactions. The list applies to EVMs created afterwards; parking an EVM
    /// removes it again, so the backend must not execute further transactions of that block
    /// afterwards.
    pub fn set_bal(&mut self, bal: Arc<Bal>, index: BlockAccessIndex) {
        self.bal = Some((bal, index));
    }

    /// Attaches the pending block access list to a newly created EVM.
    pub(super) fn apply_bal(&self, evm: &mut FoundryEvmFor<'_, FEN>) {
        if let Some((bal, index)) = &self.bal {
            let state = evm.state_mut();
            state.set_bal(Arc::clone(bal));
            state.set_bal_index(*index);
            state.set_allow_bal_db_fallback(true);
        }
    }
}

#[cfg(test)]
mod tests;
