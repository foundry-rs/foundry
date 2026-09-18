//! Block access list (BAL) reads for executing one transaction against its parent block's state.

use super::{Backend, DatabaseError, DatabaseResult};
use crate::evm::FoundryEvmNetwork;
use alloy_primitives::{Address, U256};
use revm::{
    database_interface::bal::BalState,
    state::{
        AccountInfo,
        bal::{Bal, BlockAccessIndex},
    },
};
use std::sync::Arc;

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Serves reads of state the block wrote before `index` from `bal`, and everything else from
    /// the underlying database.
    ///
    /// Index `0` holds the pre-block system writes and transaction `i` is index `i + 1`, so
    /// positioning the reads at a transaction's own index yields its prestate without replaying
    /// the earlier transactions. Committing the transaction's state removes the list again, so
    /// the backend must not execute further transactions of that block afterwards.
    pub fn set_bal(&mut self, bal: Arc<Bal>, index: BlockAccessIndex) {
        self.bal = Some(BalState {
            bal: Some(bal),
            bal_index: index,
            allow_db_fallback: true,
            ..Default::default()
        });
    }

    pub(super) fn apply_bal_account(
        &self,
        address: Address,
        account: &mut Option<AccountInfo>,
    ) -> DatabaseResult<()> {
        if let Some(bal) = &self.bal {
            bal.basic(address, account)
                .map_err(|err| DatabaseError::GetAccount(address, Arc::new(err.into())))?;
        }
        Ok(())
    }

    pub(super) fn bal_storage(
        &self,
        address: Address,
        index: U256,
    ) -> DatabaseResult<Option<U256>> {
        let Some(bal) = &self.bal else { return Ok(None) };
        bal.storage(&address, index)
            .map_err(|err| DatabaseError::GetStorage(address, index, Arc::new(err.into())))
    }
}

#[cfg(test)]
mod tests;
