//! Prefetches a new fork block's remote cache from the keys of an older cached block.
//!
//! A fork of `latest` gets an empty cache on each new head, and every cold account or storage read
//! then blocks the EVM for one RPC round trip. Earlier runs on the same chain read mostly the same
//! accounts and slots, so the keys of the newest older cache file predict the reads. The values are
//! fetched again at the new block and inserted only where the cache has no value, so the cache
//! stays exact for that block. A wrong prediction only costs an RPC request.

use super::Fork;
use alloy_eips::BlockId;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, Bytes, U64, U256, keccak256};
use alloy_provider::{Provider, RootProvider};
use foundry_config::Config;
use foundry_fork_db::{
    AccountFetchPolicy, ForkBlockEnv,
    cache::{JsonBlockCacheDB, MemDb},
};
use futures::{StreamExt, stream};
use revm::{
    primitives::KECCAK_EMPTY,
    state::{AccountInfo, Bytecode},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

/// Prefetch requests in flight at once. The fork backend sends its own cold reads without a limit;
/// this bound keeps a large key set from flooding a public endpoint.
const MAX_CONCURRENT_REQUESTS: usize = 32;

/// Largest number of accounts and slots to prefetch. Older cache files can hold the union of many
/// runs; past this size, a prefetch can cost more requests than it saves.
const MAX_KEYS: usize = 4096;

enum Key {
    Account(Address),
    Slot(Address, U256),
}

/// Starts a background prefetch into `db` if the cache of `fork` is empty.
pub(crate) fn spawn<B: ForkBlockEnv>(fork: &Fork, db: Arc<MemDb>, policy: AccountFetchPolicy) {
    if !db.accounts.read().is_empty() {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let context = fork.context();
    let Some(path) = Config::foundry_chain_cache_dir(context.source_chain_id)
        .and_then(|dir| previous_cache_file(&dir, fork.number()))
    else {
        return;
    };
    let Ok(previous) = JsonBlockCacheDB::<B>::load(&path) else { return };
    let keys = keys(previous.db());
    debug!(target: "fork::prefetch", ?path, keys = keys.len(), "prefetching fork cache");

    let provider = fork.provider::<AnyNetwork>();
    let block = fork.state_block_id();
    runtime.spawn(async move {
        let started = Instant::now();
        let fetched = stream::iter(keys)
            .map(|key| fetch(&provider, block, policy, key))
            .buffer_unordered(MAX_CONCURRENT_REQUESTS)
            .filter_map(|value| async move { value })
            .fold(0usize, |count, value| {
                insert(&db, value);
                async move { count + 1 }
            })
            .await;
        debug!(target: "fork::prefetch", fetched, elapsed = ?started.elapsed(), "prefetched fork cache");
    });
}

/// Returns the cache file of the newest block below `number` in a chain cache directory.
fn previous_cache_file(dir: &Path, number: u64) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let block = entry.file_name().to_str()?.parse::<u64>().ok()?;
            (block < number && entry.file_type().ok()?.is_file()).then_some((block, entry.path()))
        })
        .max_by_key(|(block, _)| *block)
        .map(|(_, path)| path)
}

/// Returns the accounts first, then the slots, up to [`MAX_KEYS`].
fn keys(db: &MemDb) -> Vec<Key> {
    let accounts = db.accounts.read();
    let storage = db.storage.read();
    accounts
        .keys()
        .map(|&address| Key::Account(address))
        .chain(
            storage.iter().flat_map(|(&address, slots)| {
                slots.keys().map(move |&slot| Key::Slot(address, slot))
            }),
        )
        .take(MAX_KEYS)
        .collect()
}

enum Value {
    Account(Address, AccountInfo),
    Slot(Address, U256, U256),
}

/// Fetches one key like the fork backend does. Failures are dropped, so the EVM fetches the key
/// itself if it needs it.
async fn fetch(
    provider: &RootProvider<AnyNetwork>,
    block: BlockId,
    policy: AccountFetchPolicy,
    key: Key,
) -> Option<Value> {
    match key {
        Key::Slot(address, slot) => {
            let value = provider.get_storage_at(address, slot).block_id(block).await.ok()?;
            Some(Value::Slot(address, slot, value))
        }
        Key::Account(address) => {
            let params = (address, block);
            let (balance, nonce, code) = match provider
                .raw_request::<_, alloy_rpc_types::AccountInfo>("eth_getAccountInfo".into(), params)
                .await
            {
                Ok(info) => (info.balance, info.nonce, info.code),
                Err(_) if policy == AccountFetchPolicy::Auto => {
                    let (balance, nonce, code): (U256, U64, Bytes) = futures::future::try_join3(
                        provider.raw_request("eth_getBalance".into(), params),
                        provider.raw_request("eth_getTransactionCount".into(), params),
                        provider.raw_request("eth_getCode".into(), params),
                    )
                    .await
                    .ok()?;
                    (balance, nonce.to(), code)
                }
                Err(_) => return None,
            };
            // Same conversion as the fork backend, so a prefetched account equals a fetched one.
            let code_hash = if code.is_empty() { KECCAK_EMPTY } else { keccak256(&code) };
            let info = AccountInfo {
                balance,
                nonce,
                code_hash,
                code: Some(Bytecode::new_raw(code)),
                account_id: None,
            };
            Some(Value::Account(address, info))
        }
    }
}

/// Inserts a prefetched value, retaining a value the backend already cached.
fn insert(db: &MemDb, value: Value) {
    match value {
        Value::Account(address, info) => {
            db.accounts.write().entry(address).or_insert(info);
        }
        Value::Slot(address, slot, value) => {
            db.storage.write().entry(address).or_default().entry(slot).or_insert(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn picks_newest_cache_file_below_block() {
        let dir = std::env::temp_dir().join(format!("foundry-prefetch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("120")).unwrap();
        for block in ["90", "100", "110", "130", "notes"] {
            std::fs::write(dir.join(block), b"{}").unwrap();
        }

        // `120` is a directory, as anvil writes it, and `130` is not below the block.
        assert_eq!(previous_cache_file(&dir, 125), Some(dir.join("110")));
        assert_eq!(previous_cache_file(&dir, 90), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn insert_retains_cached_values() {
        let db = MemDb::default();
        let address = address!("0x00000000000000000000000000000000000000aa");
        insert(&db, Value::Slot(address, U256::from(1), U256::from(7)));
        insert(&db, Value::Slot(address, U256::from(1), U256::from(8)));
        insert(&db, Value::Slot(address, U256::from(2), U256::from(9)));
        assert_eq!(db.storage.read()[&address][&U256::from(1)], U256::from(7));
        assert_eq!(db.storage.read()[&address][&U256::from(2)], U256::from(9));

        let cached = AccountInfo { nonce: 1, ..Default::default() };
        insert(&db, Value::Account(address, cached.clone()));
        insert(&db, Value::Account(address, AccountInfo { nonce: 2, ..Default::default() }));
        assert_eq!(db.accounts.read()[&address], cached);
    }
}
