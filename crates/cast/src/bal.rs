//! Transaction prestate reconstructed from a block access list.
//!
//! The caller supplies an unchanged, hash-pinned parent database and checks network applicability.
//! Alloy validates the list; Revm selects writes strictly before the target's one-based access
//! index, including index-zero system writes. Only a fully prepared overlay is committed, so a
//! failed read or validation leaves the parent ready for replay. A header commitment is checked
//! when present; historical lists without one have the same RPC trust boundary as parent state.
//! Creation-eligible parent accounts with nonzero BAL-listed storage require replay because a
//! creation can reset those slots without recording a BAL write.

use alloy_consensus::BlockHeader;
use alloy_eip7928::{BlockAccessList, compute_block_access_list_hash, validate_block_access_list};
use alloy_eips::BlockNumHash;
use alloy_network::{AnyRpcBlock, AnyRpcTransaction, BlockResponse, TransactionResponse};
use alloy_primitives::B256;
use alloy_rpc_types::BlockTransactions;
use eyre::{Result, WrapErr, ensure};
use revm::{
    Database, DatabaseRef,
    database_interface::{
        WrapDatabaseRef,
        bal::{BalDatabase, BalState},
    },
    primitives::hardfork::SpecId,
    state::{
        Account, EvmState, EvmStorageSlot,
        bal::{Bal, BlockAccessIndex},
    },
};
use std::sync::Arc;

/// Checks that the requested transaction and the fork parent belong to the fetched block.
pub(crate) fn validate_target(
    tx: &AnyRpcTransaction,
    block: &AnyRpcBlock,
    fork_block: BlockNumHash,
) -> Result<usize> {
    let header = block.header();
    ensure!(
        tx.block_hash() == Some(header.hash) && tx.block_number() == Some(header.number()),
        "BAL transaction block does not match the fetched block"
    );
    ensure!(
        header.number().checked_sub(1) == Some(fork_block.number)
            && header.parent_hash() == fork_block.hash,
        "BAL block does not extend the fork parent"
    );
    let BlockTransactions::Full(transactions) = block.transactions() else {
        eyre::bail!("BAL requires full block transactions")
    };
    let index = tx
        .transaction_index()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| eyre::eyre!("BAL transaction has no usable block position"))?;
    ensure!(
        transactions.get(index).is_some_and(|candidate| candidate.tx_hash() == tx.tx_hash()),
        "BAL transaction position does not match the fetched block"
    );
    ensure!(
        transactions.iter().filter(|candidate| candidate.tx_hash() == tx.tx_hash()).count() == 1,
        "BAL transaction occurs more than once in the fetched block"
    );
    Ok(index)
}

/// Materializes writes before a transaction without changing the parent database.
pub(crate) fn prepare_prestate<DB: DatabaseRef>(
    db: &DB,
    bal: BlockAccessList,
    transaction_index: usize,
    transaction_count: usize,
    expected_hash: Option<B256>,
    spec: SpecId,
) -> Result<EvmState> {
    // Before EIP-6780, SELFDESTRUCT can clear storage that the BAL does not enumerate.
    ensure!(spec.is_enabled_in(SpecId::CANCUN), "BAL prestate requires Cancun or later");
    ensure!(transaction_index < transaction_count, "BAL transaction position is out of bounds");
    validate_block_access_list(&bal, transaction_count).wrap_err("invalid block access list")?;
    if let Some(expected_hash) = expected_hash {
        ensure!(
            compute_block_access_list_hash(&bal) == expected_hash,
            "block access list does not match the block header commitment"
        );
    }

    let bal = Arc::new(Bal::try_from_alloy(bal).wrap_err("invalid BAL bytecode")?);
    let index = BlockAccessIndex::new(
        u64::try_from(transaction_index)?
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("BAL transaction position overflow"))?,
    );
    let mut positioned = BalDatabase {
        bal_state: BalState { bal: Some(bal.clone()), bal_index: index, ..Default::default() },
        db: WrapDatabaseRef(db),
    };
    let mut state = EvmState::default();

    for (&address, changes) in &bal.accounts {
        // CREATE can reset storage without a BAL write, including after create-and-selfdestruct.
        // Check read-only and future-write entries too, before skipping unchanged accounts.
        if !changes.storage.storage.is_empty()
            && db.basic_ref(address)?.is_none_or(|info| info.has_no_code_and_nonce())
        {
            for &slot in changes.storage.storage.keys() {
                ensure!(
                    db.storage_ref(address, slot)?.is_zero(),
                    "BAL cannot reconstruct a possible storage reset for {address}"
                );
            }
        }

        // Read-only entries and writes at or after the target need no overlay.
        let has_prior_changes = changes.balance.get(index).is_some()
            || changes.nonce.get(index).is_some()
            || changes.code.get(index).is_some()
            || changes.storage.storage.values().any(|writes| writes.get(index).is_some());
        if !has_prior_changes {
            continue;
        }

        let info = positioned.basic(address)?.ok_or_else(|| {
            eyre::eyre!("BAL contains storage changes for missing account {address}")
        })?;
        let mut account = Account::from(info);
        account.mark_touch();
        for (&slot, writes) in &changes.storage.storage {
            if writes.get(index).is_some() {
                let value = positioned.storage(address, slot)?;
                account.storage.insert(slot, EvmStorageSlot::new(value, Default::default()));
            }
        }
        state.insert(address, account);
    }

    // The caller commits only this completed overlay. A failed read leaves replay on parent state.
    Ok(state)
}

#[cfg(test)]
mod tests;
