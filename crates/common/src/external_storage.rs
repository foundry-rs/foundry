//! Storage layouts for contracts that are not part of the local project.
//!
//! When a forked test touches a contract that isn't in the local artifacts, its storage slots can
//! still be decoded by compiling the verified source a block explorer has for it.
//!
//! Finding that source is the caller's job — `foundry-evm-traces` already knows how to ask a block
//! explorer for it. This module owns what happens next: compiling it for a storage layout, which
//! costs a full `solc` invocation, and making sure that cost is paid once.
//! `forge test` runs test contracts in parallel, so lookups are:
//!
//! - deduplicated process-wide by `LOOKUPS`, so concurrent tests touching the same contract compile
//!   it once instead of racing each other;
//! - persisted to disk once resolved, so later runs skip straight to the layout.
//!
//! Unverified responses are memoized only for this run so later runs can discover newly verified
//! contracts.

use crate::fs;
use alloy_chains::Chain;
use alloy_primitives::{Address, map::AddressMap};
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    artifacts::{
        CompilerOutput, SolcInput, SolcLanguage, Source, Sources, StorageLayout,
        output_selection::OutputSelection,
    },
    solc::Solc,
};
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use wait_timeout::ChildExt;

/// First solc release that emits `storageLayout`. Matches the floor `cast storage` enforces.
const MIN_STORAGE_LAYOUT_SOLC: semver::Version = semver::Version::new(0, 6, 5);
/// First solc release with `--base-path` support.
const BASE_PATH_SOLC: semver::Version = semver::Version::new(0, 6, 9);
/// First solc release with `--no-import-callback` support.
const NO_IMPORT_CALLBACK_SOLC: semver::Version = semver::Version::new(0, 8, 22);

/// A resolved storage layout, or `None` if the contract has no layout we can use.
///
/// A `None` means the lookup concluded that there is nothing to decode with: the contract is
/// unverified, is Vyper, failed to compile, or compiled to an empty layout. It is memoized for
/// the rest of the run.
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

/// Bounds expensive compiler installation and execution across parallel tests.
static COMPILER: Mutex<()> = Mutex::new(());

/// Makes cache publication paths unique within one process; the PID separates processes.
static CACHE_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Resolves the storage layouts of `addresses` on `chain`, compiling verified sources as needed.
///
/// `fetch_sources` is only called for the addresses still unknown after the in-process and
/// on-disk caches have been consulted, so a warm run makes no network requests at all. For each
/// address it is handed, it returns that exact contract's verified source, or `None` if the block
/// explorer conclusively reports that there is none.
///
/// An address `fetch_sources` leaves out is one it reached no conclusion about. Those are left
/// unresolved rather than remembered as having no layout, so a later call tries again instead of
/// letting one explorer outage disable decoding for the rest of the run.
///
/// Addresses without a usable layout are absent from the returned map.
pub fn fetch_external_storage_layouts(
    chain: Chain,
    addresses: impl IntoIterator<Item = Address>,
    timeout: Duration,
    fetch_sources: impl FnOnce(&[Address], Duration) -> AddressMap<Option<Metadata>>,
) -> AddressMap<(String, Arc<StorageLayout>)> {
    let cache_dir = Config::foundry_etherscan_chain_cache_dir(chain);
    resolve(chain.id(), cache_dir.as_deref(), addresses, timeout, fetch_sources)
}

/// [`fetch_external_storage_layouts`] with the cache location supplied, so tests can point it
/// somewhere other than the user's home directory.
fn resolve(
    chain_id: u64,
    cache_dir: Option<&Path>,
    addresses: impl IntoIterator<Item = Address>,
    timeout: Duration,
    fetch_sources: impl FnOnce(&[Address], Duration) -> AddressMap<Option<Metadata>>,
) -> AddressMap<(String, Arc<StorageLayout>)> {
    let deadline = Instant::now().checked_add(timeout).unwrap_or_else(Instant::now);
    let mut resolved = AddressMap::default();

    // Claim a lookup slot per address. Sorting keeps the acquisition order identical in every
    // thread, so holding several slots at once cannot deadlock.
    let mut addresses = addresses.into_iter().collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    let slots = {
        let Some(mut lookups) = lock_until(&LOOKUPS, deadline) else {
            warn!(target: "external-storage", "external storage lookup timed out");
            return resolved;
        };
        addresses
            .into_iter()
            .map(|address| (address, lookups.entry((chain_id, address)).or_default().clone()))
            .collect::<Vec<_>>()
    };

    let mut pending = Vec::new();
    for (address, slot) in &slots {
        // Waits for a concurrent lookup of the same address to finish, if any.
        let Some(guard) = lock_until(slot, deadline) else {
            warn!(target: "external-storage", %address, "external storage lookup timed out");
            return resolved;
        };
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
        let Some(cached) = read_cached_layout(cache_dir, *address) else {
            return true;
        };
        resolved.insert(*address, cached.clone());
        **guard = Some(Some(cached));
        false
    });

    if pending.is_empty() {
        return resolved;
    }

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        warn!(target: "external-storage", "external storage lookup timed out");
        return resolved;
    }
    let sources =
        fetch_sources(&pending.iter().map(|(address, _)| *address).collect::<Vec<_>>(), remaining);

    for (address, mut guard) in pending {
        let Some(source) = sources.get(&address) else {
            // The lookup reached no conclusion. Leave the slot unresolved so the next call
            // retries, rather than recording "no layout" on the strength of an outage.
            continue;
        };

        let Some(metadata) = source else {
            // Retry unverified contracts on the next run, when sources may be available.
            *guard = Some(None);
            continue;
        };

        let layout = compile_storage_layout(address, metadata, deadline);
        if layout.is_none() && Instant::now() >= deadline {
            // Exhausting this call's budget says nothing about whether the contract has a
            // usable layout. Leave it unresolved so a later call can retry.
            continue;
        }
        if let Some((name, layout)) = &layout {
            resolved.insert(address, (name.clone(), layout.clone()));
            write_cached_layout(cache_dir, address, name, layout);
        }
        // Compilation failures are memoized only for this run. Unlike an explicit unverified
        // response, they may be transient and must not become a persistent negative entry.
        *guard = Some(layout);
    }

    resolved
}

/// Compiles a verified source with `storageLayout` output enabled and extracts the layout.
fn compile_storage_layout(
    address: Address,
    metadata: &Metadata,
    deadline: Instant,
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

    let sources = metadata
        .sources()
        .into_iter()
        .map(|(path, source)| (PathBuf::from(path), Source::new(source.content)))
        .collect::<Sources>();
    if sources.is_empty() {
        trace!(target: "external-storage", %address, "verified metadata has no sources");
        return None;
    }
    let Some(_compiler) = lock_until(&COMPILER, deadline) else {
        warn!(target: "external-storage", %address, "external storage compilation timed out");
        return None;
    };

    // Compile standard JSON with every source supplied inline in an empty sandbox. Newer solc
    // versions disable the filesystem import callback explicitly; the empty working/base directory
    // prevents older versions from resolving omitted sources.
    let version = metadata.compiler_version().ok()?;
    let mut settings = match metadata.settings() {
        Ok(settings) => settings,
        Err(err) => {
            warn!(target: "external-storage", %address, %err, "failed to read compiler settings");
            return None;
        }
    };
    settings.output_selection =
        OutputSelection::common_output_selection(["storageLayout".to_string()]);
    let input = SolcInput::new(SolcLanguage::Solidity, sources, settings).sanitized(&version);
    let svm_version = semver::Version::new(version.major, version.minor, version.patch);
    let solc = match Solc::find_svm_installed_version(&svm_version) {
        Ok(Some(solc)) => solc,
        Ok(None) => {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let installed =
                crate::block_on(tokio::time::timeout(remaining, Solc::install(&svm_version)));
            match installed {
                Ok(Ok(solc)) => solc,
                Ok(Err(err)) => {
                    warn!(target: "external-storage", %address, %err, "failed to install compiler");
                    return None;
                }
                Err(_) => {
                    warn!(target: "external-storage", %address, "compiler installation timed out");
                    return None;
                }
            }
        }
        Err(err) => {
            warn!(target: "external-storage", %address, %err, "failed to find compiler");
            return None;
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let output = match run_solc(&solc, &version, &input, remaining) {
        Ok(output) => output,
        Err(err) => {
            warn!(target: "external-storage", %address, %err, "failed to compile contract");
            return None;
        }
    };

    let name = metadata.contract_name.clone();
    let mut matches = output
        .contracts
        .values()
        .filter_map(|contracts| contracts.get(&name))
        .filter(|contract| !contract.storage_layout.storage.is_empty());
    let layout = matches.next().map(|contract| contract.storage_layout.clone());
    if matches.next().is_some() {
        warn!(target: "external-storage", %address, %name, "multiple artifacts match contract name");
        return None;
    }

    let Some(layout) = layout else {
        warn!(target: "external-storage", %address, %name, "no storage layout in compiled artifacts");
        return None;
    };

    Some((name, Arc::new(layout)))
}

/// Runs solc in an empty directory and terminates it if the remaining lookup budget expires.
fn run_solc(
    solc: &Solc,
    version: &semver::Version,
    input: &SolcInput,
    timeout: Duration,
) -> Result<CompilerOutput, String> {
    if timeout.is_zero() {
        return Err("compilation timed out".to_string());
    }

    let sandbox = tempfile::tempdir().map_err(|err| err.to_string())?;
    let mut stdin = tempfile::tempfile().map_err(|err| err.to_string())?;
    serde_json::to_writer(&mut stdin, input).map_err(|err| err.to_string())?;
    stdin.seek(SeekFrom::Start(0)).map_err(|err| err.to_string())?;
    let mut stdout = tempfile::tempfile().map_err(|err| err.to_string())?;
    let mut stderr = tempfile::tempfile().map_err(|err| err.to_string())?;

    let mut command = Command::new(&solc.solc);
    command.arg("--standard-json").current_dir(sandbox.path());
    if version >= &BASE_PATH_SOLC {
        command.arg("--base-path").arg(sandbox.path());
    }
    if version >= &NO_IMPORT_CALLBACK_SOLC {
        command.arg("--no-import-callback");
    }
    command
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout.try_clone().map_err(|err| err.to_string())?))
        .stderr(Stdio::from(stderr.try_clone().map_err(|err| err.to_string())?));

    let mut child = command.spawn().map_err(|err| err.to_string())?;
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("compilation timed out".to_string());
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err.to_string());
        }
    };
    if !status.success() {
        stderr.seek(SeekFrom::Start(0)).map_err(|err| err.to_string())?;
        let mut message = String::new();
        stderr.read_to_string(&mut message).map_err(|err| err.to_string())?;
        return Err(if message.trim().is_empty() {
            format!("solc exited with {status}")
        } else {
            message
        });
    }

    stdout.seek(SeekFrom::Start(0)).map_err(|err| err.to_string())?;
    serde_json::from_reader(stdout).map_err(|err| err.to_string())
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
const CACHE_VERSION: u32 = 2;

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
    let cached = fs::read_json_file::<CachedStorageLayout>(&path).ok()?;
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

    // A uniquely created file prevents threads and processes from sharing a writer. Persisting it
    // in the destination directory publishes a complete JSON document with one atomic rename.
    let tmp_path = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        CACHE_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp_path) {
        Ok(_) => {}
        Err(err) => {
            warn!(target: "external-storage", %address, %err, "failed to create cache temporary file");
            return;
        }
    }
    let write = fs::write_json_file(&tmp_path, &cached).and_then(|()| {
        std::fs::rename(&tmp_path, &path)
            .map_err(|err| crate::errors::FsPathError::write(err, path.as_path()))
    });
    if let Err(err) = write {
        warn!(target: "external-storage", %address, %err, "failed to cache storage layout");
        let _ = std::fs::remove_file(&tmp_path);
    }
}

/// Acquires a lookup lock without exceeding the caller's deadline.
fn lock_until<T>(mutex: &Mutex<T>, deadline: Instant) -> Option<MutexGuard<'_, T>> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            // A slot only holds a memoized result, so recovering after a panic is safe.
            Err(TryLockError::Poisoned(err)) => return Some(err.into_inner()),
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(1)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_block_explorers::contract::SourceCodeMetadata;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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
        let resolved = resolve(
            chain_id,
            Some(cache_dir.path()),
            [cached, fresh],
            Duration::from_secs(1),
            |addresses, _| {
                asked_for.extend_from_slice(addresses);
                AddressMap::default()
            },
        );

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
        let mut unverified = |addresses: &[Address], _: Duration| {
            lookups += 1;
            addresses.iter().map(|address| (*address, None)).collect::<AddressMap<_>>()
        };

        assert!(
            resolve(
                chain_id,
                Some(cache_dir.path()),
                [address],
                Duration::from_secs(1),
                &mut unverified,
            )
            .is_empty()
        );
        assert!(
            resolve(
                chain_id,
                Some(cache_dir.path()),
                [address],
                Duration::from_secs(1),
                &mut unverified,
            )
            .is_empty()
        );
        assert_eq!(lookups, 1, "an unverified contract should only be looked up once");
        assert!(!cache_path(cache_dir.path(), address).exists());
    }

    #[test]
    fn retries_an_address_the_lookup_reached_no_conclusion_about() {
        let cache_dir = tempfile::tempdir().unwrap();
        let chain_id = next_chain_id();
        let address = Address::with_last_byte(1);

        // An explorer outage: the lookup answers for nothing it was asked about.
        let mut lookups = 0;
        let mut unavailable = |_: &[Address], _: Duration| {
            lookups += 1;
            AddressMap::default()
        };

        assert!(
            resolve(
                chain_id,
                Some(cache_dir.path()),
                [address],
                Duration::from_secs(1),
                &mut unavailable,
            )
            .is_empty()
        );
        assert!(
            resolve(
                chain_id,
                Some(cache_dir.path()),
                [address],
                Duration::from_secs(1),
                &mut unavailable,
            )
            .is_empty()
        );
        assert_eq!(lookups, 2, "an outage must not be remembered as \"no layout\"");
    }

    #[test]
    fn waiting_for_an_inflight_lookup_respects_the_timeout() {
        let chain_id = next_chain_id();
        let address = Address::with_last_byte(1);
        let slot = {
            let mut lookups = LOOKUPS.lock().unwrap();
            lookups.entry((chain_id, address)).or_default().clone()
        };
        let _inflight = slot.lock().unwrap();
        let started = Instant::now();
        let mut fetched = false;

        let result = resolve(chain_id, None, [address], Duration::from_millis(20), |_, _| {
            fetched = true;
            AddressMap::default()
        });

        assert!(result.is_empty());
        assert!(!fetched);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn solc_is_killed_when_compilation_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solc");
        std::fs::write(&path, "#!/bin/sh\nexec sleep 10\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let solc = Solc {
            solc: path,
            version: NO_IMPORT_CALLBACK_SOLC,
            base_path: None,
            allow_paths: Default::default(),
            include_paths: Default::default(),
            extra_args: Vec::new(),
        };
        let input = SolcInput::new(SolcLanguage::Solidity, Default::default(), Default::default());
        let started = Instant::now();

        assert!(
            run_solc(&solc, &NO_IMPORT_CALLBACK_SOLC, &input, Duration::from_millis(20)).is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn solc_runs_in_an_empty_sandbox_with_imports_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solc");
        std::fs::write(
            &path,
            "#!/bin/sh\ncase \" $* \" in *\" --no-import-callback \"*) ;; *) exit 1;; esac\n[ -z \"$(ls -A)\" ] || exit 1\nprintf '{\"contracts\":{}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let solc = Solc {
            solc: path,
            version: NO_IMPORT_CALLBACK_SOLC,
            base_path: None,
            allow_paths: Default::default(),
            include_paths: Default::default(),
            extra_args: Vec::new(),
        };
        let input = SolcInput::new(SolcLanguage::Solidity, Default::default(), Default::default());

        assert!(run_solc(&solc, &NO_IMPORT_CALLBACK_SOLC, &input, Duration::from_secs(1)).is_ok());
    }

    fn source_less_metadata() -> Metadata {
        Metadata {
            source_code: SourceCodeMetadata::Sources(Default::default()),
            abi: "[]".to_string(),
            contract_name: "MissingSources".to_string(),
            compiler_version: "v0.8.30".to_string(),
            optimization_used: 0,
            runs: 0,
            constructor_arguments: Default::default(),
            evm_version: String::new(),
            library: String::new(),
            license_type: String::new(),
            proxy: 0,
            implementation: None,
            swarm_source: String::new(),
        }
    }

    #[test]
    fn retries_after_the_compilation_budget_expires() {
        let chain_id = next_chain_id();
        let address = Address::with_last_byte(1);
        let metadata = Metadata {
            source_code: SourceCodeMetadata::SourceCode(
                "pragma solidity ^0.8.30; contract Counter { uint256 public count; }".to_string(),
            ),
            contract_name: "Counter".to_string(),
            ..source_less_metadata()
        };
        // Keep compilation from progressing after fetching consumes the shared budget.
        let _compiler = COMPILER.lock().unwrap();
        let result =
            resolve(chain_id, None, [address], Duration::from_millis(20), |_, remaining| {
                std::thread::sleep(remaining);
                [(address, Some(metadata))].into_iter().collect()
            });
        assert!(result.is_empty());

        let mut retried = false;
        resolve(chain_id, None, [address], Duration::from_secs(1), |addresses, _| {
            assert_eq!(addresses, [address]);
            retried = true;
            AddressMap::default()
        });
        assert!(retried, "a compilation timeout must not be memoized as no layout");
    }

    #[test]
    fn source_less_metadata_is_rejected_without_compiling() {
        let metadata = source_less_metadata();
        assert!(compile_storage_layout(Address::ZERO, &metadata, Instant::now()).is_none());
    }
}
