//! Native BAL reads for a single transaction executed against its unchanged parent state.

use super::{Backend, DatabaseError, DatabaseResult};
use crate::evm::FoundryEvmNetwork;
use alloy_primitives::{Address, U256};
use revm::{
    DatabaseRef,
    database_interface::bal::BalState,
    state::{
        AccountInfo,
        bal::{Bal, BlockAccessIndex},
    },
};
use std::sync::Arc;

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Positions native BAL reads before a transaction without changing the parent database.
    ///
    /// The caller must supply a validated BAL for Cancun or later and use `transaction_index + 1`.
    /// Install it only for a single transaction on an unchanged parent. Committing the transaction
    /// removes BAL and preserves accessed account code for trace decoding. Discard the backend
    /// afterward: skipped prefix storage is not materialized for subsequent transactions.
    /// Accounts that could have been created in the skipped prefix require ordinary replay
    /// when their storage is read: BAL does not enumerate every slot cleared by CREATE.
    pub fn set_bal(&mut self, bal: Option<Arc<Bal>>, index: BlockAccessIndex) {
        self.bal =
            bal.map(|bal| BalState { bal: Some(bal), bal_index: index, ..Default::default() });
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
        // Inspect raw parent account metadata, not BAL-overlaid nonce or code. A skipped CREATE
        // can clear unlisted storage, including in index-zero system execution. Reject even when
        // BAL has a prior slot write, since a later creation might have cleared that value.
        let parent = if let Some(db) = self.active_fork_db() {
            db.basic_ref(address)?
        } else {
            self.mem_db.basic_ref(address)?
        };
        if parent.is_none_or(|account| account.has_no_code_and_nonce()) {
            return Err(DatabaseError::GetStorage(
                address,
                index,
                Arc::new(eyre::eyre!(
                    "BAL cannot exclude a storage reset for {address}; replay required"
                )),
            ));
        }
        bal.storage(&address, index)
            .map_err(|err| DatabaseError::GetStorage(address, index, Arc::new(err.into())))
    }
}

#[cfg(test)]
mod tests;
