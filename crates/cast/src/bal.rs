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
    DatabaseRef,
    primitives::hardfork::SpecId,
    state::{
        Account, AccountId, EvmState, EvmStorageSlot,
        bal::{AccountInfoBal, BalWrites, BlockAccessIndex},
    },
};

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

    // Validate every bytecode entry before reading the parent, including future writes.
    // Keep account fields in Revm's representation and consume storage slots directly.
    let accounts = bal
        .into_iter()
        .map(|changes| {
            let info = AccountInfoBal {
                nonce: changes.nonce_changes.into(),
                balance: changes.balance_changes.into(),
                code: BalWrites::try_from(changes.code_changes).wrap_err("invalid BAL bytecode")?,
            };
            Ok((changes.address, info, changes.storage_changes, changes.storage_reads))
        })
        .collect::<Result<Vec<_>>>()?;
    let index = BlockAccessIndex::new(
        u64::try_from(transaction_index)?
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("BAL transaction position overflow"))?,
    );
    let mut state = EvmState::default();

    for (account_index, (address, changes, storage_changes, storage_reads)) in
        accounts.into_iter().enumerate()
    {
        // Validation guarantees sorted writes, so their first index decides whether an overlay
        // is needed without searching for values or cloning bytecode.
        let has_prior_changes = has_prior_write(&changes.balance, index)
            || has_prior_write(&changes.nonce, index)
            || has_prior_write(&changes.code, index)
            || storage_changes.iter().any(|slot| slot.changes[0].block_access_index < index);
        let has_storage = !storage_changes.is_empty() || !storage_reads.is_empty();
        if !has_prior_changes && !has_storage {
            continue;
        }

        // Reuse the parent read for both reset detection and Revm's account overlay.
        let info = db.basic_ref(address)?;
        // CREATE can reset storage without a BAL write, including after create-and-selfdestruct.
        // Check read-only and future-write entries too, before skipping unchanged accounts.
        if has_storage && info.as_ref().is_none_or(|info| info.has_no_code_and_nonce()) {
            for slot in storage_changes.iter().map(|slot| slot.slot).chain(storage_reads) {
                ensure!(
                    db.storage_ref(address, slot)?.is_zero(),
                    "BAL cannot reconstruct a possible storage reset for {address}"
                );
            }
        }

        // Read-only entries and writes at or after the target need no overlay.
        if !has_prior_changes {
            continue;
        }

        let was_missing = info.is_none();
        let mut info = info.unwrap_or_default();
        let changed = changes.populate_account_info(index, &mut info);
        ensure!(
            !was_missing || changed,
            "BAL contains storage changes for missing account {address}"
        );
        info.account_id = Some(AccountId::new(account_index).expect("too many bals"));
        let mut account = Account::from(info);
        account.mark_touch();
        for slot in storage_changes {
            if let Some(value) = BalWrites::from(slot.changes).get(index) {
                account.storage.insert(slot.slot, EvmStorageSlot::new(value, Default::default()));
            }
        }
        state.insert(address, account);
    }

    // The caller commits only this completed overlay. A failed read leaves replay on parent state.
    Ok(state)
}

/// Whether a validated, sorted write list contains a value before the target.
fn has_prior_write<T: PartialEq + Clone>(writes: &BalWrites<T>, index: BlockAccessIndex) -> bool {
    writes.writes.first().is_some_and(|(first, _)| *first < index)
}

#[cfg(test)]
mod tests;
