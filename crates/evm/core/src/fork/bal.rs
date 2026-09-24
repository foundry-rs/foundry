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
use alloy_primitives::{Address, B256, U256, map::U256Map};
use alloy_provider::Provider;
use eyre::{Result, WrapErr};
use foundry_common::provider::is_rpc_method_not_found;
use foundry_evm_networks::NetworkConfigs;
use foundry_fork_db::cache::MemDb;
use revm::state::{AccountInfo, Bytecode};
use std::time::Duration;

/// Validated values tied to one exact block, configured source, and endpoint context.
#[derive(Debug)]
pub(super) struct PreparedBalSeed {
    fingerprint: B256,
    accounts: Vec<(Address, AccountInfo)>,
    storage: Vec<(Address, Vec<(U256, U256)>)>,
}

/// Prepares optional parent state without mutating a database or propagating BAL failures.
pub(super) async fn prepare<P: Provider<AnyNetwork>>(
    provider: &P,
    resolved: &ResolvedFork,
    block: &AnyRpcBlock,
    already_prewarmed: bool,
) -> Option<PreparedBalSeed> {
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
        let seed = if already_prewarmed {
            None
        } else {
            let bal = match provider
                .raw_request("eth_getBlockAccessList".into(), (resolved.hash(),))
                .await
            {
                Err(error) if is_rpc_method_not_found(&error) => {
                    provider.get_block_access_list_by_hash(resolved.hash()).await
                }
                response => response,
            }
            .ok()??;
            match PreparedBalSeed::new(bal, block, resolved) {
                Ok(seed) => Some(seed),
                Err(err) => {
                    debug!(target: "backend::fork", block_hash = %resolved.hash(), %err, "ignoring invalid fork BAL");
                    return None;
                }
            }
        };
        // Reused caches retain the source checks without downloading the same BAL again.
        // A local node can change state without changing its block hash.
        immutable_source(provider).await.then_some(seed).flatten()
    };

    // Reuse the validated block; optional BAL and source probes share one budget, including
    // retries.
    let seed = tokio::time::timeout(Duration::from_millis(500), prepare).await.ok().flatten();
    if seed.is_none() && !already_prewarmed {
        debug!(target: "backend::fork", block_hash = %resolved.hash(), "fork BAL unavailable or ineligible");
    }
    seed
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

impl PreparedBalSeed {
    fn new(bal: BlockAccessList, block: &AnyRpcBlock, resolved: &ResolvedFork) -> Result<Self> {
        eyre::ensure!(
            block.header.hash == resolved.hash() && block.header.number() == resolved.number(),
            "BAL block identity mismatch"
        );
        validate_block_access_list(&bal, block.transactions.len())
            .wrap_err("invalid BAL structure")?;
        if let Some(expected_hash) = block.header.block_access_list_hash() {
            eyre::ensure!(
                compute_block_access_list_hash(&bal) == expected_hash,
                "BAL hash mismatch"
            );
        }

        let mut accounts = Vec::new();
        let mut storage = Vec::new();
        for account in bal {
            if !account.storage_changes.is_empty() {
                let mut slots = Vec::with_capacity(account.storage_changes.len());
                slots.extend(account.storage_post_states());
                storage.push((account.address, slots));
            }
            let balance = account.balance_post_state();
            let nonce = account.nonce_post_state();
            let mut code = None;
            // Reject invalid code even in earlier changes or incomplete accounts.
            for change in account.code_changes {
                code =
                    Some(Bytecode::new_raw_checked(change.new_code).wrap_err("invalid BAL code")?);
            }
            if let (Some(balance), Some(nonce), Some(code)) = (balance, nonce, code) {
                accounts.push((
                    account.address,
                    AccountInfo {
                        balance,
                        nonce,
                        code_hash: code.hash_slow(),
                        code: Some(code),
                        account_id: None,
                    },
                ));
            }
        }

        Ok(Self { fingerprint: resolved.fingerprint(), accounts, storage })
    }

    /// Seeds the selected remote cache, retaining accounts and slots already present.
    pub(super) fn apply(self, db: &MemDb, resolved: &ResolvedFork) -> bool {
        if self.fingerprint != resolved.fingerprint() {
            debug!(target: "backend::fork", "ignoring fork BAL for a different cache identity");
            return false;
        }

        let mut accounts = db.accounts.write();
        let accounts_before = accounts.len();
        for (address, account) in self.accounts {
            accounts.entry(address).or_insert(account);
        }
        let inserted_accounts = accounts.len() - accounts_before;
        drop(accounts);

        let mut storage = db.storage.write();
        let mut inserted_slots = 0;
        for (address, slots) in self.storage {
            let cached_slots = storage.entry(address).or_insert_with(|| {
                U256Map::with_capacity_and_hasher(slots.len(), Default::default())
            });
            let slots_before = cached_slots.len();
            for (slot, value) in slots {
                cached_slots.entry(slot).or_insert(value);
            }
            inserted_slots += cached_slots.len() - slots_before;
        }
        debug!(target: "backend::fork", block_hash = %resolved.hash(), inserted_accounts, inserted_slots, "prefilled fork cache from BAL");
        true
    }
}

#[cfg(test)]
mod tests;
