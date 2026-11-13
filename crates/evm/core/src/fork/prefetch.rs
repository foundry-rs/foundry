//! Prefetches a new fork block's remote cache from the keys of an older cached block.
//!
//! A fork of `latest` gets an empty cache on each new head, and every cold account or storage read
//! then blocks the EVM for one RPC round trip. Earlier runs on the same chain read mostly the same
//! accounts and slots, so the keys of the newest older cache file predict the reads. The keys are
//! read again at the new block through the fork backend, so the cache stays exact for that block.
//! The cache file keeps only the keys the EVM then reads, so a wrong prediction only costs an RPC
//! request and does not carry over to the next block.
//!
//! A call is predicted more precisely by simulating it on the endpoint, see [`prefetch_call`].

use super::Fork;
use crate::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    precompiles::P256_VERIFY,
};
use alloy_network::Network;
use alloy_primitives::{Address, B256, Bytes, KECCAK256_EMPTY, U256, map::B256HashMap};
use alloy_rpc_types::{TransactionRequest, state::StateOverride};
use evm2::{bytecode::Bytecode, evm::Cache};
use foundry_config::Config;
use foundry_fork_db::{
    ForkBlockEnv, SharedBackend,
    backend::BlockingMode,
    cache::{JsonBlockCacheDB, MemDb},
};
use parking_lot::Mutex;
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

/// Runtime code that returns one zero word, standing in for the cheatcode contract in a
/// simulation on the fork's endpoint: `PUSH1 0x20 PUSH1 0x00 RETURN`.
const CHEATCODE_STUB: Bytes = Bytes::from_static(&[0x60, 0x20, 0x60, 0x00, 0xf3]);

/// Largest local state, in bytes of code and storage, sent as overrides with a simulated call.
const MAX_OVERRIDE_BYTES: usize = 512 * 1024;

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

/// Returns the local state of a fork context that differs from `remote`, the state the fork loaded
/// from its endpoint, as overrides for a simulation there, with a stub for the cheatcode contract.
/// Such state is the test contract and balances set with cheatcodes.
///
/// Returns `None` when nothing differs, so endpoints without override support still simulate, or
/// when the state is larger than [`MAX_OVERRIDE_BYTES`].
pub(crate) fn state_override(remote: &MemDb, local: &Cache) -> Option<StateOverride> {
    let remote_accounts = remote.accounts.read();
    let remote_storage = remote.storage.read();
    let mut overrides = StateOverride::default();
    let mut bytes = 0;
    for (address, info) in &local.accounts {
        // An absent account keeps its remote state; the simulation only predicts reads.
        let Some(info) = info else { continue };
        let remote = remote_accounts.get(address);
        // Code that is neither inline nor in the contracts is unknown and not overridden, unless
        // its hash is the empty code hash.
        let code = remote
            .is_none_or(|remote| remote.code_hash != info.code_hash)
            .then(|| {
                let code = info.code.as_ref().or_else(|| local.contracts.get(&info.code_hash));
                code.map(Bytecode::original_bytes)
                    .or_else(|| info.is_empty_code_hash().then(Bytes::new))
            })
            .flatten();
        let balance = remote.is_none_or(|remote| remote.balance != info.balance);
        let nonce = remote.is_none_or(|remote| remote.nonce != info.nonce);
        if code.is_some() || balance || nonce {
            bytes += code.as_ref().map_or(0, |code| code.len());
            let account = overrides.entry(*address).or_default();
            account.code = code;
            account.balance = balance.then_some(info.balance);
            account.nonce = nonce.then_some(info.nonce);
        }
    }
    for (address, storage) in &local.storage {
        // Wiped storage replaces the whole remote storage.
        let remote = remote_storage.get(address).filter(|_| !storage.wiped);
        let slots = storage
            .slots
            .iter()
            .filter(|&(slot, value)| remote.and_then(|remote| remote.get(slot)) != Some(value))
            .map(|(slot, value)| (B256::from(*slot), B256::from(*value)))
            .collect::<B256HashMap<_>>();
        bytes += slots.len() * 64;
        if bytes > MAX_OVERRIDE_BYTES {
            return None;
        }
        if storage.wiped {
            overrides.entry(*address).or_default().state = Some(slots);
        } else if !slots.is_empty() {
            overrides.entry(*address).or_default().state_diff = Some(slots);
        }
    }
    if overrides.is_empty() || bytes > MAX_OVERRIDE_BYTES {
        return None;
    }
    overrides.entry(CHEATCODE_ADDRESS).or_default().code = Some(CHEATCODE_STUB);
    Some(overrides)
}

/// Simulates a call on the fork's endpoint and prefetches the state it touches, see
/// [`SharedBackend::prefetch_access_list`]. With `overrides`, the `local` state over the fork goes
/// along as overrides; without, a call into an account without code on the endpoint, or one the
/// context holds but the fork never loaded from it, is skipped. Calls to precompiles, up to
/// `P256VERIFY` at `0x100`, the cheatcode contract and the console are skipped.
pub(crate) fn prefetch_call<N: Network, B: ForkBlockEnv>(
    backend: &SharedBackend<N, B>,
    caller: Address,
    target: Address,
    value: U256,
    input: Bytes,
    local: Option<&Cache>,
    overrides: bool,
) {
    if target <= P256_VERIFY || target == CHEATCODE_ADDRESS || target == HARDHAT_CONSOLE_ADDRESS {
        return;
    }
    let remote = backend.data();
    let remote_code = remote.accounts.read().get(&target).map(|account| account.code_hash);
    let local_account = local.and_then(|local| local.accounts.get(&target)?.as_ref());
    // Local code tells apart contracts at one address, such as the test contracts of two suites.
    let code_hash = local_account
        .map(|account| account.code_hash)
        .filter(|&code_hash| remote_code != Some(code_hash))
        .unwrap_or_default();
    let tx = TransactionRequest::default().from(caller).to(target).value(value).input(input.into());
    // Checked first, so a call that is not simulated does not build its overrides.
    if !backend.wants_access_list(&tx, code_hash, overrides) {
        return;
    }
    let overrides = local.filter(|_| overrides).and_then(|local| state_override(&remote, local));
    // Without overrides, the endpoint runs its own code at the target, so a call is skipped when
    // the target has no code there, such as an account created on the fork, or when the context
    // holds the target but never loaded it from the endpoint, such as an account created before
    // the fork was selected.
    if overrides.is_none()
        && remote_code.map_or(local_account.is_some(), |remote| remote == KECCAK256_EMPTY)
    {
        return;
    }
    backend.prefetch_access_list(tx, overrides, code_hash);
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
