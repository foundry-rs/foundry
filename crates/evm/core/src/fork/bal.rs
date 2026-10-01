//! Validates and caches BAL post-state, and prepares transaction forks' parent-block BALs.

use super::ResolvedFork;
use crate::opts::ForkContext;
use alloy_chains::{Chain, NamedChain};
use alloy_consensus::BlockHeader;
use alloy_eips::{
    BlockId,
    eip7928::{BlockAccessList, compute_block_access_list_hash, validate_block_access_list},
};
use alloy_hardforks::EthereumHardfork;
use alloy_network::{AnyNetwork, AnyRpcBlock};
use alloy_primitives::{
    B256, U256,
    map::{AddressHashMap, U256Map},
};
use alloy_provider::Provider;
use eyre::{Result, WrapErr};
use foundry_common::provider::is_rpc_method_not_found;
use foundry_evm_networks::NetworkConfigs;
use foundry_fork_db::cache::MemDb;
use revm::state::{AccountInfo, Bytecode};
use std::time::Duration;

/// Fetches and validates a parent BAL without mutating a database or propagating BAL failures.
pub(super) async fn prepare<P: Provider<AnyNetwork>>(
    provider: &P,
    resolved: &ResolvedFork,
    block: &AnyRpcBlock,
) -> Option<BlockAccessList> {
    if !eligible_source(resolved.context())
        || block.header.hash != resolved.hash()
        || block.header.number() != resolved.number()
        || !EthereumHardfork::from_chain_and_timestamp(
            Chain::from_id(resolved.context().source_chain_id),
            block.header.timestamp(),
        )
        .is_some_and(|hardfork| hardfork >= EthereumHardfork::Cancun)
    {
        return None;
    }

    let prepare = async {
        // An inconclusive discovery probe is not proof of an immutable source.
        if !immutable_source(provider).await {
            return None;
        }
        let bal = provider.get_block_access_list(BlockId::hash(resolved.hash())).await.ok()??;
        if let Err(err) =
            validate_bal(&bal, block.transactions.len(), block.header.block_access_list_hash())
        {
            debug!(target: "backend::fork", block_hash = %resolved.hash(), %err, "ignoring invalid fork BAL");
            return None;
        }
        // A local node can change state without changing its block hash.
        immutable_source(provider).await.then_some(bal)
    };

    // Reuse the validated block; optional BAL and source probes share one budget, including
    // retries.
    let bal = tokio::time::timeout(Duration::from_millis(500), prepare).await.ok().flatten();
    if bal.is_none() {
        debug!(target: "backend::fork", block_hash = %resolved.hash(), "fork BAL unavailable or ineligible");
    }
    bal
}

fn eligible_source(context: ForkContext) -> bool {
    context.network.is_ethereum()
        && context.network_profile.canonical_execution_profile() == NetworkConfigs::default()
        && context.hardfork.is_none()
        && context.instance_id.is_none()
        && context.source_fork_block_number.is_none()
        && context.source_fork_block_hash.is_none()
        && matches!(
            NamedChain::try_from(context.source_chain_id),
            Ok(NamedChain::Mainnet | NamedChain::Sepolia | NamedChain::Holesky | NamedChain::Hoodi)
        )
}

async fn immutable_source<P: Provider<AnyNetwork>>(provider: &P) -> bool {
    matches!(
        provider.raw_request::<_, serde_json::Value>("anvil_nodeInfo".into(), ()).await,
        Err(error) if is_rpc_method_not_found(&error)
    )
}

/// Validates the entire BAL before any values can enter the cache.
///
/// Callers must separately establish that the BAL and cache belong to the same immutable block.
pub fn validate_bal(
    bal: &BlockAccessList,
    transaction_count: usize,
    expected_hash: Option<B256>,
) -> Result<()> {
    validate_block_access_list(bal, transaction_count).wrap_err("invalid BAL structure")?;
    if let Some(expected_hash) = expected_hash {
        eyre::ensure!(compute_block_access_list_hash(bal) == expected_hash, "BAL hash mismatch");
    }
    for account in bal {
        // Reject invalid code even in earlier changes or incomplete accounts.
        for change in &account.code_changes {
            Bytecode::new_raw_checked(change.new_code.clone()).wrap_err("invalid BAL code")?;
        }
    }
    Ok(())
}

/// Inserts a validated BAL's post-state into its selected remote cache, retaining existing values.
///
/// The BAL must pass [`validate_bal`] before this call, and the cache must belong to its immutable
/// source block. Account and storage locks are acquired separately; insertion is not atomic across
/// the two maps.
pub fn cache_bal(db: &MemDb, bal: BlockAccessList) {
    let mut accounts = db.accounts.write();
    let inserted_accounts = cache_bal_accounts(&mut accounts, &bal);
    drop(accounts);

    let mut storage = db.storage.write();
    let inserted_slots = cache_bal_storage(&mut storage, &bal);
    drop(storage);
    debug!(target: "backend::fork", inserted_accounts, inserted_slots, "prefilled fork cache from BAL");
}

/// Inserts complete account post-states, retaining existing accounts, and returns the added count.
fn cache_bal_accounts(accounts: &mut AddressHashMap<AccountInfo>, bal: &BlockAccessList) -> usize {
    let accounts_before = accounts.len();
    for account in bal {
        if let (Some(balance), Some(nonce), Some(code)) =
            (account.balance_post_state(), account.nonce_post_state(), account.code_changes.last())
        {
            accounts.entry(account.address).or_insert_with(|| {
                let code = Bytecode::new_raw(code.new_code.clone());
                AccountInfo {
                    balance,
                    nonce,
                    code_hash: code.hash_slow(),
                    code: Some(code),
                    account_id: None,
                }
            });
        }
    }
    accounts.len() - accounts_before
}

/// Inserts final slot writes, retaining cached values and leaving read-only slots unknown.
///
/// Returns the number of added slots.
fn cache_bal_storage(storage: &mut AddressHashMap<U256Map<U256>>, bal: &BlockAccessList) -> usize {
    let mut inserted_slots = 0;
    for account in bal {
        if !account.storage_changes.is_empty() {
            let cached_slots = storage.entry(account.address).or_insert_with(|| {
                U256Map::with_capacity_and_hasher(account.storage_changes.len(), Default::default())
            });
            let slots_before = cached_slots.len();
            for (slot, value) in account.storage_post_states() {
                cached_slots.entry(slot).or_insert(value);
            }
            inserted_slots += cached_slots.len() - slots_before;
        }
    }
    inserted_slots
}

#[cfg(test)]
mod tests;
