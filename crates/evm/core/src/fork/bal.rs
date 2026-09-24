//! Prefills immutable parent-block state before replaying a transaction fork's prefix.

use super::ResolvedFork;
use crate::opts::ForkContext;
use alloy_chains::{Chain, NamedChain};
use alloy_consensus::BlockHeader;
use alloy_eips::eip7928::{
    BlockAccessList, compute_block_access_list_hash, validate_block_access_list,
};
use alloy_hardforks::EthereumHardfork;
use alloy_network::{AnyNetwork, AnyRpcBlock};
use alloy_primitives::map::U256Map;
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
        let bal =
            match provider.raw_request("eth_getBlockAccessList".into(), (resolved.hash(),)).await {
                Err(error) if is_rpc_method_not_found(&error) => {
                    provider.get_block_access_list_by_hash(resolved.hash()).await
                }
                response => response,
            }
            .ok()??;
        if let Err(err) = validate(&bal, block) {
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
fn validate(bal: &BlockAccessList, block: &AnyRpcBlock) -> Result<()> {
    validate_block_access_list(bal, block.transactions.len()).wrap_err("invalid BAL structure")?;
    if let Some(expected_hash) = block.header.block_access_list_hash() {
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
pub(super) fn cache(db: &MemDb, bal: BlockAccessList) {
    let mut accounts = db.accounts.write();
    let accounts_before = accounts.len();
    for account in &bal {
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
    let inserted_accounts = accounts.len() - accounts_before;
    drop(accounts);

    let mut storage = db.storage.write();
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
    debug!(target: "backend::fork", inserted_accounts, inserted_slots, "prefilled fork cache from BAL");
}

#[cfg(test)]
mod tests;
