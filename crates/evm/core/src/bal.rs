//! Transaction prestate reconstructed from a block access list.
//!
//! The caller supplies an unchanged, hash-pinned parent database and checks network applicability.
//! Alloy validates the list; Revm selects writes strictly before the target's one-based access
//! index, including index-zero system writes. Only a fully prepared overlay is committed, so a
//! failed read or validation leaves the parent ready for replay. A header commitment is checked
//! when present; historical lists without one have the same RPC trust boundary as parent state.
//! Accounts that could have been created require an empty parent storage root: CREATE and
//! SELFDESTRUCT can clear slots that the list never mentions. Preparation and root verification
//! are separate so callers can run the fallible RPC phase before committing anything.

use alloy_consensus::{BlockHeader, Transaction, constants::EMPTY_ROOT_HASH};
use alloy_eip7928::{BlockAccessList, compute_block_access_list_hash, validate_block_access_list};
use alloy_eips::{BlockId, BlockNumHash};
use alloy_network::{AnyRpcBlock, AnyRpcTransaction, BlockResponse, Network, TransactionResponse};
use alloy_primitives::{Address, B256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockTransactions;
use eyre::{Result, WrapErr, ensure};
use futures::{StreamExt, TryStreamExt, stream};
use revm::{
    DatabaseRef,
    primitives::hardfork::SpecId,
    state::{
        Account, AccountId, AccountInfo, EvmState, EvmStorageSlot,
        bal::{AccountInfoBal, BalWrites, BlockAccessIndex},
    },
};
use std::time::Duration;
use tokio::time::timeout;

/// An uncommitted overlay whose possible storage resets still need verification.
///
/// Obtain the commit-ready state with [`Self::verify_storage_roots`]. Keeping the state private
/// prevents a caller from accidentally applying it before the full-state safety check succeeds.
#[derive(Debug)]
pub struct PreparedPrestate {
    state: EvmState,
    possible_resets: Vec<(Address, AccountInfo)>,
}

impl PreparedPrestate {
    /// Verifies empty storage roots at the exact parent hash before releasing the overlay.
    ///
    /// A missing proof, nonempty root, inconsistent account or timeout requires replay. Proofs
    /// share the parent state's RPC trust boundary; this does not authenticate Merkle paths.
    pub async fn verify_storage_roots<P, N>(
        self,
        provider: &P,
        parent_hash: B256,
    ) -> Result<EvmState>
    where
        P: Provider<N> + ?Sized,
        N: Network,
    {
        let checks =
            stream::iter(self.possible_resets.into_iter().map(|(address, info)| async move {
                let proof = provider
                    .get_proof(address, Vec::new())
                    .block_id(BlockId::hash(parent_hash))
                    .await?;
                ensure!(
                    proof.address == address
                        && proof.balance == info.balance
                        && proof.nonce == info.nonce
                        && proof.code_hash == info.code_hash,
                    "BAL parent account proof does not match {address}"
                );
                ensure!(
                    proof.storage_hash == EMPTY_ROOT_HASH,
                    "BAL cannot exclude a storage reset for {address}"
                );
                Ok::<_, eyre::Report>(())
            }))
            .buffer_unordered(16)
            .try_collect::<Vec<_>>();
        timeout(Duration::from_millis(500), checks)
            .await
            .wrap_err("BAL storage root checks timed out")??;
        Ok(self.state)
    }
}

/// Checks that the requested transaction and the fork parent belong to the fetched block.
pub fn validate_target(
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

/// Checks the unavoidable sender nonce writes for the ordinary Ethereum transaction prefix.
///
/// Structural BAL validation cannot detect a truncated list. Every ordinary transaction changes
/// its sender nonce, even when execution reverts. EIP-7702 authorizations may increment it again.
pub fn validate_transaction_changes(
    bal: &BlockAccessList,
    transactions: &[AnyRpcTransaction],
) -> Result<()> {
    for (index, tx) in transactions.iter().enumerate() {
        let expected = BlockAccessIndex::new(u64::try_from(index)? + 1);
        let change = bal.binary_search_by_key(&tx.from(), |account| account.address).ok().and_then(
            |account| {
                bal[account]
                    .nonce_changes
                    .binary_search_by_key(&expected, |change| change.block_access_index)
                    .ok()
                    .map(|index| &bal[account].nonce_changes[index])
            },
        );
        ensure!(
            change.is_some_and(|change| change.new_nonce > tx.nonce()),
            "BAL is missing the sender nonce change for transaction {index}"
        );
    }
    Ok(())
}

/// Materializes writes before a transaction without changing the parent database.
///
/// Callers must first bind the target and parent with [`validate_target`] and check the ordinary
/// transaction prefix with [`validate_transaction_changes`]. These checks reject obvious
/// omissions; lists without a header commitment still rely on the provider's completeness.
pub fn prepare_prestate<DB: DatabaseRef>(
    db: &DB,
    bal: BlockAccessList,
    transaction_index: usize,
    transaction_count: usize,
    expected_hash: Option<B256>,
    spec: SpecId,
) -> Result<PreparedPrestate> {
    // Before EIP-6780, SELFDESTRUCT can clear storage that the BAL does not enumerate.
    ensure!(spec.is_enabled_in(SpecId::CANCUN), "BAL prestate requires Cancun or later");
    ensure!(transaction_index < transaction_count, "BAL transaction position is out of bounds");
    ensure!(!bal.is_empty(), "BAL is empty for a block containing transactions");
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
            Ok((changes.address, info, changes.storage_changes))
        })
        .collect::<Result<Vec<_>>>()?;
    let index = BlockAccessIndex::new(
        u64::try_from(transaction_index)?
            .checked_add(1)
            .ok_or_else(|| eyre::eyre!("BAL transaction position overflow"))?,
    );
    let mut state = EvmState::default();
    let mut possible_resets = Vec::new();

    for (account_index, (address, changes, storage_changes)) in accounts.into_iter().enumerate() {
        // Validation guarantees sorted writes, so their first index decides whether an overlay
        // is needed without searching for values or cloning bytecode.
        let has_prior_changes = has_prior_write(&changes.balance, index)
            || has_prior_write(&changes.nonce, index)
            || has_prior_write(&changes.code, index)
            || storage_changes.iter().any(|slot| slot.changes[0].block_access_index < index);
        // There is no transaction prefix for the first transaction. Ethereum's pre-block
        // operations only require the index-zero overlay, not reads of future accounts.
        if transaction_index == 0 && !has_prior_changes {
            continue;
        }

        // Reuse the parent read for both reset detection and Revm's account overlay.
        let info = db.basic_ref(address)?;
        // Even an account with no writes can have been created and destroyed in the prefix.
        // A whole-trie root check covers unlisted slots too, including custom genesis state.
        if transaction_index > 0 && info.as_ref().is_none_or(|info| info.has_no_code_and_nonce()) {
            possible_resets.push((address, info.clone().unwrap_or_default()));
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
    Ok(PreparedPrestate { state, possible_resets })
}

/// Whether a validated, sorted write list contains a value before the target.
fn has_prior_write<T: PartialEq + Clone>(writes: &BalWrites<T>, index: BlockAccessIndex) -> bool {
    writes.writes.first().is_some_and(|(first, _)| *first < index)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifecycle;
