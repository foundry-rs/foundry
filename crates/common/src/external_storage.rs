//! Storage layouts for contracts that are not part of the local project.
//!
//! When a forked test touches a contract that isn't in the local artifacts, its storage slots can
//! still be decoded by compiling the verified source a block explorer has for it.
//!
//! Finding that source is the caller's job — `foundry-evm-traces` already knows how to ask
//! Sourcify and Etherscan for it. This module owns what happens next: compiling it for a storage
//! layout, which costs a full `solc` invocation, and making sure that cost is paid once.
//! `forge test` runs test contracts in parallel, so lookups are:
//!
//! - deduplicated process-wide by [`LOOKUPS`], so concurrent tests touching the same contract
//!   compile it once instead of racing each other;
//! - persisted to disk once resolved, so later runs skip straight to the layout.
//!
//! Only resolved layouts reach the disk cache. A contract with no usable layout is remembered for
//! the rest of the run but looked up again on the next one, so a contract that gets verified after
//! the fact is picked up without a `forge cache clean`. This is the same tradeoff the signature
//! cache makes for unknown selectors.

use crate::{
    compile::{ProjectCompiler, add_storage_layout_output, etherscan_project},
    fs,
};
use alloy_chains::Chain;
use alloy_primitives::{Address, map::AddressMap};
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::artifacts::StorageLayout;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

/// First solc release that emits `storageLayout`. Matches the floor `cast storage` enforces.
const MIN_STORAGE_LAYOUT_SOLC: semver::Version = semver::Version::new(0, 6, 5);

/// A resolved storage layout, or `None` if the contract has no layout we can use.
///
/// A `None` means the lookup concluded that there is nothing to decode with: the contract is
/// unverified, is Vyper, failed to compile, or compiled to an empty layout. It is memoized for
/// the rest of the run, but never written to the disk cache.
type ExternalStorageLayout = Option<(String, Arc<StorageLayout>)>;

/// A single address' lookup slot: `None` until the lookup completes, then its memoized result.
type LookupSlot = Arc<Mutex<Option<ExternalStorageLayout>>>;

/// Per-(chain, address) lookup slots, shared across the whole process.
///
/// The outer mutex only guards the map; the work itself happens while holding the inner mutex of a
/// single entry, so lookups for different addresses still run concurrently while lookups for the
/// same address wait for the first one to finish and then reuse its result.
static LOOKUPS: LazyLock<Mutex<std::collections::HashMap<(u64, Address), LookupSlot>>> =
    LazyLock::new(Default::default);

/// Resolves the storage layouts of `addresses` on `chain`, compiling verified sources as needed.
///
/// `fetch_sources` is only called for the addresses still unknown after the in-process and
/// on-disk caches have been consulted, so a warm run makes no network requests at all. For each
/// address it is handed, it returns the contract implementing it — the address itself, or the end
/// of its proxy chain — and that contract's verified source, or `None` if there is none.
///
/// An address `fetch_sources` leaves out is one it reached no conclusion about. Those are left
/// unresolved rather than remembered as having no layout, so a later call tries again instead of
/// letting one explorer outage disable decoding for the rest of the run.
///
/// Addresses without a usable layout are absent from the returned map.
pub fn fetch_external_storage_layouts(
    chain: Chain,
    addresses: impl IntoIterator<Item = Address>,
    fetch_sources: impl FnOnce(&[Address]) -> AddressMap<Option<(Address, Metadata)>>,
) -> AddressMap<(String, Arc<StorageLayout>)> {
    let cache_dir = Config::foundry_etherscan_chain_cache_dir(chain);
    resolve(chain.id(), cache_dir.as_deref(), addresses, fetch_sources)
}

/// [`fetch_external_storage_layouts`] with the cache location supplied, so tests can point it
/// somewhere other than the user's home directory.
fn resolve(
    chain_id: u64,
    cache_dir: Option<&Path>,
    addresses: impl IntoIterator<Item = Address>,
    fetch_sources: impl FnOnce(&[Address]) -> AddressMap<Option<(Address, Metadata)>>,
) -> AddressMap<(String, Arc<StorageLayout>)> {
    let mut resolved = AddressMap::default();

    // Claim a lookup slot per address. Sorting keeps the acquisition order identical in every
    // thread, so holding several slots at once cannot deadlock.
    let mut addresses = addresses.into_iter().collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    let slots = {
        let mut lookups = lock(&LOOKUPS);
        addresses
            .into_iter()
            .map(|address| (address, lookups.entry((chain_id, address)).or_default().clone()))
            .collect::<Vec<_>>()
    };

    let mut pending = Vec::new();
    for (address, slot) in &slots {
        // Waits for a concurrent lookup of the same address to finish, if any.
        let guard = lock(slot);
        if let Some(cached) = &*guard {
            if let Some((name, layout)) = cached {
                resolved.insert(*address, (name.clone(), layout.clone()));
            }
            continue;
        }
        pending.push((*address, guard));
    }

    if pending.is_empty() {
        return resolved;
    }

    // Serve whatever the disk cache already resolved, so the remaining work is only for addresses
    // this machine has no layout for yet.
    pending.retain_mut(|(address, guard)| {
        let Some((name, layout)) = read_cached_layout(cache_dir, *address) else {
            return true;
        };
        resolved.insert(*address, (name.clone(), layout.clone()));
        **guard = Some(Some((name, layout)));
        false
    });

    if pending.is_empty() {
        return resolved;
    }

    let sources = fetch_sources(&pending.iter().map(|(address, _)| *address).collect::<Vec<_>>());

    for (address, mut guard) in pending {
        let Some(source) = sources.get(&address) else {
            // The lookup reached no conclusion. Leave the slot unresolved so the next call
            // retries, rather than recording "no layout" on the strength of an outage.
            continue;
        };

        let layout = source.as_ref().and_then(|(implementation, metadata)| {
            compile_storage_layout(cache_dir, *implementation, metadata)
        });
        if let Some((name, layout)) = &layout {
            resolved.insert(address, (name.clone(), layout.clone()));
            write_cached_layout(cache_dir, address, name, layout);
            // Cache the implementation under its own address too, so a second proxy pointing at
            // it resolves without another fetch.
            if let Some((implementation, _)) = source
                && *implementation != address
            {
                write_cached_layout(cache_dir, *implementation, name, layout);
            }
        }
        // A contract with no usable layout is memoized for the rest of the run, but deliberately
        // not persisted: it may well be verified by the time of the next run.
        *guard = Some(layout);
    }

    resolved
}

/// Compiles a verified source with `storageLayout` output enabled and extracts the layout.
fn compile_storage_layout(
    cache_dir: Option<&Path>,
    address: Address,
    metadata: &Metadata,
) -> Option<(String, Arc<StorageLayout>)> {
    if metadata.is_vyper() {
        trace!(target: "external-storage", %address, "skipping vyper contract");
        return None;
    }

    // Older solc has no `storageLayout` output at all, so compiling would cost a full solc run to
    // produce nothing. `cast storage` bumps such contracts to `MIN_SOLC`; here there is no user
    // asking about one specific contract, so just leave them undecoded.
    match metadata.compiler_version() {
        Ok(version) if version < MIN_STORAGE_LAYOUT_SOLC => {
            trace!(target: "external-storage", %address, %version, "solc too old for storage layouts");
            return None;
        }
        Ok(_) => {}
        Err(err) => {
            warn!(target: "external-storage", %address, %err, "could not read compiler version");
            return None;
        }
    }

    let name = metadata.contract_name.clone();
    let root = sources_dir(cache_dir, address)?;

    let mut project = match etherscan_project(metadata, &root) {
        Ok(project) => project,
        Err(err) => {
            warn!(target: "external-storage", %address, %name, %err, "failed to create project from source");
            return None;
        }
    };
    add_storage_layout_output(&mut project);

    let output = match ProjectCompiler::new().quiet(true).compile(&project) {
        Ok(output) => output,
        Err(err) => {
            warn!(target: "external-storage", %address, %name, %err, "failed to compile contract");
            return None;
        }
    };

    let layout = output
        .artifacts()
        .find(|(artifact_name, _)| *artifact_name == name)
        .and_then(|(_, artifact)| artifact.storage_layout.clone())
        .filter(|layout| !layout.storage.is_empty());

    let Some(layout) = layout else {
        warn!(target: "external-storage", %address, %name, "no storage layout in compiled artifacts");
        return None;
    };

    Some((name, Arc::new(layout)))
}

/// Directory the verified sources of `address` are checked out into before compiling.
///
/// This is the same per-chain `sources` directory the block explorer client caches into, so
/// `cast storage` and a `forge test` lookup of the same contract share one checkout.
fn sources_dir(cache_dir: Option<&Path>, address: Address) -> Option<PathBuf> {
    let root = match cache_dir {
        Some(cache_dir) => cache_dir.join("sources").join(address.to_string()),
        // Without a cache directory there is nowhere durable to put the checkout, so fall back to
        // a per-address temporary directory.
        None => std::env::temp_dir().join(format!("foundry-storage-{address}")),
    };
    if let Err(err) = std::fs::create_dir_all(&root) {
        warn!(target: "external-storage", %address, %err, "failed to create sources directory");
        return None;
    }
    Some(root)
}

/// Disk representation of a resolved lookup. Cleared by `forge cache clean`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedStorageLayout {
    /// Bumped whenever a change makes previously written entries wrong. Entries written by any
    /// other version are ignored, so a fix doesn't need users to clear their cache by hand.
    version: u32,
    contract_name: String,
    storage_layout: StorageLayout,
}

/// Current [`CachedStorageLayout`] format.
const CACHE_VERSION: u32 = 1;

/// Path of the cache entry for `address`.
fn cache_path(cache_dir: &Path, address: Address) -> PathBuf {
    cache_dir.join("storage_layouts").join(format!("{address}.json"))
}

/// Reads the layout a previous run resolved for `address`, if there is one.
fn read_cached_layout(
    cache_dir: Option<&Path>,
    address: Address,
) -> Option<(String, Arc<StorageLayout>)> {
    let path = cache_path(cache_dir?, address);
    let cached: CachedStorageLayout = fs::read_json_file(&path).ok()?;
    if cached.version != CACHE_VERSION {
        trace!(target: "external-storage", %address, cached.version, "ignoring stale cache entry");
        return None;
    }
    trace!(target: "external-storage", %address, "using cached storage layout");
    Some((cached.contract_name, Arc::new(cached.storage_layout)))
}

/// Persists a resolved layout so later runs can skip the fetch and the compile.
///
/// The entry is written to a temporary file and renamed into place, so parallel tests writing the
/// same address cannot leave a reader with a half-written file.
fn write_cached_layout(
    cache_dir: Option<&Path>,
    address: Address,
    name: &str,
    layout: &StorageLayout,
) {
    let Some(cache_dir) = cache_dir else { return };
    let path = cache_path(cache_dir, address);
    let Some(parent) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(parent) {
        warn!(target: "external-storage", %address, %err, "failed to create storage layout cache");
        return;
    }

    let cached = CachedStorageLayout {
        version: CACHE_VERSION,
        contract_name: name.to_string(),
        storage_layout: layout.clone(),
    };

    // `{address}.json.{pid}.tmp` keeps concurrent writers from sharing a temporary file, while
    // staying in the destination directory so the rename is atomic.
    let tmp_path = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let write = fs::write_json_file(&tmp_path, &cached).and_then(|()| {
        std::fs::rename(&tmp_path, &path)
            .map_err(|err| crate::errors::FsPathError::write(err, path.as_path()))
    });
    if let Err(err) = write {
        warn!(target: "external-storage", %address, %err, "failed to cache storage layout");
        let _ = std::fs::remove_file(&tmp_path);
    }
}

/// A panic while a lookup is in flight leaves its slot poisoned. A slot only ever holds a
/// memoized result, so recovering one just means redoing that lookup.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// [`LOOKUPS`] is keyed by chain, so giving every test its own chain id keeps them from
    /// seeing each other's memoized results.
    fn next_chain_id() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn round_trips_a_resolved_layout() {
        let cache_dir = tempfile::tempdir().unwrap();
        let address = Address::with_last_byte(1);

        // Nothing cached yet.
        assert!(read_cached_layout(Some(cache_dir.path()), address).is_none());

        let layout = StorageLayout::default();
        write_cached_layout(Some(cache_dir.path()), address, "Counter", &layout);

        let (name, cached) = read_cached_layout(Some(cache_dir.path()), address).unwrap();
        assert_eq!(name, "Counter");
        assert_eq!(*cached, layout);
    }

    #[test]
    fn ignores_cache_entries_written_by_another_version() {
        let cache_dir = tempfile::tempdir().unwrap();
        let address = Address::with_last_byte(1);
        write_cached_layout(Some(cache_dir.path()), address, "Counter", &StorageLayout::default());

        let path = cache_path(cache_dir.path(), address);
        let stale = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&format!("\"version\":{CACHE_VERSION}"), "\"version\":0");
        std::fs::write(&path, stale).unwrap();

        assert!(read_cached_layout(Some(cache_dir.path()), address).is_none());
    }

    #[test]
    fn write_leaves_no_temporary_files_behind() {
        let cache_dir = tempfile::tempdir().unwrap();
        let address = Address::with_last_byte(1);
        write_cached_layout(Some(cache_dir.path()), address, "Counter", &StorageLayout::default());

        let entries = std::fs::read_dir(cache_dir.path().join("storage_layouts"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, [format!("{address}.json").as_str()]);
    }

    #[test]
    fn serves_the_disk_cache_without_looking_anything_up() {
        let cache_dir = tempfile::tempdir().unwrap();
        let chain_id = next_chain_id();
        let cached = Address::with_last_byte(1);
        let fresh = Address::with_last_byte(2);
        write_cached_layout(Some(cache_dir.path()), cached, "Counter", &StorageLayout::default());

        let mut asked_for = Vec::new();
        let resolved = resolve(chain_id, Some(cache_dir.path()), [cached, fresh], |addresses| {
            asked_for.extend_from_slice(addresses);
            AddressMap::default()
        });

        // Only the uncached address reaches the lookup, and the cached one still comes back.
        assert_eq!(asked_for, [fresh]);
        assert_eq!(resolved.keys().copied().collect::<Vec<_>>(), [cached]);
        assert_eq!(resolved[&cached].0, "Counter");
    }

    #[test]
    fn remembers_a_conclusive_miss_for_the_rest_of_the_run() {
        let cache_dir = tempfile::tempdir().unwrap();
        let chain_id = next_chain_id();
        let address = Address::with_last_byte(1);

        let mut lookups = 0;
        let mut unverified = |addresses: &[Address]| {
            lookups += 1;
            addresses.iter().map(|address| (*address, None)).collect::<AddressMap<_>>()
        };

        assert!(resolve(chain_id, Some(cache_dir.path()), [address], &mut unverified).is_empty());
        assert!(resolve(chain_id, Some(cache_dir.path()), [address], &mut unverified).is_empty());
        assert_eq!(lookups, 1, "an unverified contract should only be looked up once");

        // ...but the verdict is not written to disk, so a later run picks it up once verified.
        assert!(!cache_path(cache_dir.path(), address).exists());
    }

    #[test]
    fn retries_an_address_the_lookup_reached_no_conclusion_about() {
        let cache_dir = tempfile::tempdir().unwrap();
        let chain_id = next_chain_id();
        let address = Address::with_last_byte(1);

        // An explorer outage: the lookup answers for nothing it was asked about.
        let mut lookups = 0;
        let mut unavailable = |_: &[Address]| {
            lookups += 1;
            AddressMap::default()
        };

        assert!(resolve(chain_id, Some(cache_dir.path()), [address], &mut unavailable).is_empty());
        assert!(resolve(chain_id, Some(cache_dir.path()), [address], &mut unavailable).is_empty());
        assert_eq!(lookups, 2, "an outage must not be remembered as \"no layout\"");
    }
}
