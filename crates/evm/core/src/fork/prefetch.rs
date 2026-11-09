//! Prefetches a new fork block's remote cache from the keys of an older cached block.
//!
//! A fork of `latest` gets an empty cache on each new head, and every cold account or storage read
//! then blocks the EVM for one RPC round trip. Earlier runs on the same chain read mostly the same
//! accounts and slots, so the keys of the newest older cache file predict the reads. The keys are
//! read again at the new block through the fork backend, so the cache stays exact for that block.
//! The cache file keeps only the keys the EVM then reads, so a wrong prediction only costs an RPC
//! request and does not carry over to the next block.

use super::Fork;
use crate::constants::CHEATCODE_ADDRESS;
use alloy_network::Network;
use alloy_primitives::{Address, B256, Bytes, U256, map::B256HashMap};
use alloy_rpc_types::state::{AccountOverride, StateOverride};
use foundry_config::Config;
use foundry_fork_db::{
    ForkBlockEnv, SharedBackend,
    backend::BlockingMode,
    cache::{JsonBlockCacheDB, MemDb},
};
use parking_lot::Mutex;
use revm::database::{AccountState, CacheDB};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
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

/// Starts a background prefetch through `backend` if its cache is empty.
///
/// Each key is read through the backend like an EVM read, so the backend fetches and caches it with
/// its own request path, and a key it already has costs no request. Loading the previous cache file
/// runs in the background too, so it does not block the fork handler.
pub(crate) fn spawn<N: Network, B: ForkBlockEnv>(
    fork: &Fork,
    backend: &SharedBackend<N, B>,
) -> Option<Prefetch> {
    if !backend.data().accounts.read().is_empty() {
        return None;
    }
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let (chain, number) = (fork.context().source_chain_id, fork.number());
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done) = mpsc::channel::<()>();
    let backend = backend.with_blocking_mode(BlockingMode::Block);
    let coordinator_stop = Arc::clone(&stop);
    runtime.clone().spawn_blocking(move || {
        let stop = coordinator_stop;
        if let Some(path) =
            Config::foundry_chain_cache_dir(chain).and_then(|dir| previous_cache_file(&dir, number))
            && let Ok(previous) = JsonBlockCacheDB::<B>::load(&path)
            && !stop.load(Ordering::Relaxed)
        {
            let keys = keys(previous.db());
            debug!(target: "fork::prefetch", ?path, keys = keys.len(), "prefetching fork cache");
            // Workers take the next key without a lock held during a read.
            let keys = Arc::new(Mutex::new(keys.into_iter()));
            for _ in 0..MAX_CONCURRENT_REQUESTS {
                let (keys, stop, done_tx) = (Arc::clone(&keys), Arc::clone(&stop), done_tx.clone());
                let backend = backend.clone();
                runtime.spawn_blocking(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let Some(key) = keys.lock().next() else { break };
                        // A prefetched key stays out of the cache file until the EVM reads it.
                        let read = match key {
                            Key::Account(address) => backend.prefetch_basic(address),
                            Key::Slot(address, slot) => backend.prefetch_storage(address, slot),
                        };
                        // The EVM fetches any key it still needs, so stop at the first failure.
                        if read.is_err() {
                            break;
                        }
                    }
                    // Drop the backend clone before reporting done, as below.
                    drop(backend);
                    drop(done_tx);
                });
            }
        }
        // The last backend clone flushes the disk cache, so drop it before reporting done.
        drop(backend);
        drop(done_tx);
    });
    Some(Prefetch { stop, done })
}

/// Background prefetch workers of one fork backend.
pub(crate) struct Prefetch {
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
}

impl Prefetch {
    /// Stops the workers and waits until they and their coordinator exit, so whichever holds the
    /// last backend clone flushes the disk cache before shutdown completes.
    pub(crate) fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        // Returns once every worker has dropped its sender.
        let _ = self.done.recv();
    }
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

/// Runtime code that returns one zero word, standing in for the cheatcode contract in a
/// simulation on the fork's endpoint: `PUSH1 0x20 PUSH1 0x00 RETURN`.
const CHEATCODE_STUB: Bytes = Bytes::from_static(&[0x60, 0x20, 0x60, 0x00, 0xf3]);

/// Largest local state, in storage slots, sent as overrides with a simulated call.
const MAX_OVERRIDE_SLOTS: usize = 8192;

/// Collects the state of a fork that differs from its endpoint as overrides for a simulation on
/// the endpoint, see [`SharedBackend::prefetch_access_list`].
///
/// Accounts and slots the fork loaded from the endpoint are left out, so the overrides hold
/// contracts deployed locally, such as the test contract, and local changes to remote accounts,
/// such as balances and storage set with cheatcodes. The cheatcode contract gets a stub that
/// returns one zero word, so the simulation runs past most cheatcode calls. Returns `None` when
/// the local state is too large to send.
pub fn state_override<N: Network, B: ForkBlockEnv>(
    db: &CacheDB<SharedBackend<N, B>>,
) -> Option<StateOverride> {
    let remote = db.db.data();
    let remote_accounts = remote.accounts.read();
    let remote_storage = remote.storage.read();
    let mut overrides = StateOverride::default();
    let mut slots = 0;
    for (address, account) in &db.cache.accounts {
        if account.account_state == AccountState::NotExisting {
            continue;
        }
        let remote_account = remote_accounts.get(address);
        let mut account_override = AccountOverride::default();
        if remote_account.is_none_or(|remote| remote.code_hash() != account.info.code_hash()) {
            let code = account
                .info
                .code
                .as_ref()
                .or_else(|| db.cache.contracts.get(&account.info.code_hash))
                .map(|code| code.original_bytes())
                .unwrap_or_default();
            account_override.code = Some(code);
        }
        if remote_account.is_none_or(|remote| remote.balance != account.info.balance) {
            account_override.balance = Some(account.info.balance);
        }
        if remote_account.is_none_or(|remote| remote.nonce != account.info.nonce) {
            account_override.nonce = Some(account.info.nonce);
        }
        let remote_slots = remote_storage.get(address);
        let diff = account
            .storage
            .iter()
            .filter(|(slot, value)| remote_slots.and_then(|slots| slots.get(slot)) != Some(value))
            .map(|(slot, value)| (B256::from(*slot), B256::from(*value)))
            .collect::<B256HashMap<_>>();
        slots += diff.len();
        if slots > MAX_OVERRIDE_SLOTS {
            return None;
        }
        if !diff.is_empty() {
            account_override.state_diff = Some(diff);
        }
        if account_override != AccountOverride::default() {
            overrides.insert(*address, account_override);
        }
    }
    overrides.entry(CHEATCODE_ADDRESS).or_default().code = Some(CHEATCODE_STUB);
    Some(overrides)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
