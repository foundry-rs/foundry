//! Storage layouts for contracts that aren't part of the local project.
//!
//! [`foundry_common::external_storage`] turns a verified source into a storage layout and caches
//! the result; [`ExternalIdentifier`] finds that source on a block explorer. This module is the
//! seam between them, and owns the state that has to outlive a single lookup.

use alloy_primitives::{
    Address,
    map::{AddressMap, HashMap},
};
use foundry_common::external_storage::fetch_external_storage_layouts;
use foundry_compilers::artifacts::StorageLayout;
use foundry_config::Chain;
use foundry_evm_traces::identifier::{ExternalIdentifier, ExternalIdentifierConfig};
use std::{
    sync::{Arc, LazyLock, Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};

/// Chain id to the identifier for that chain, or `None` if one couldn't be built.
type Identifiers = HashMap<u64, Option<Arc<Mutex<ExternalIdentifier>>>>;

/// The [`ExternalIdentifier`] in use for each chain, shared by every test in the process.
///
/// One identifier per chain rather than one per lookup: it carries the metadata it has already
/// fetched and the budget for how long identification may go on for, and neither means anything
/// unless it survives across calls.
static IDENTIFIERS: LazyLock<Mutex<Identifiers>> = LazyLock::new(Default::default);

/// Resolves the storage layouts of contracts outside the local project.
///
/// Returns the layouts it could resolve; anything absent stays undecoded. Costs nothing for
/// addresses already resolved in this process or by a previous run.
pub(crate) fn storage_layouts(
    sources: &ExternalIdentifierConfig,
    chain: Chain,
    addresses: Vec<Address>,
) -> AddressMap<(String, Arc<StorageLayout>)> {
    fetch_external_storage_layouts(
        chain,
        addresses,
        sources.storage_timeout(),
        |unresolved, timeout| {
            let deadline = Instant::now().checked_add(timeout).unwrap_or_else(Instant::now);
            let Some(identifier) = identifier(sources, chain, deadline) else {
                return Default::default();
            };
            let Some(mut identifier) = lock_until(&identifier, deadline) else {
                return Default::default();
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            foundry_common::block_on(identifier.get_metadata(unresolved, remaining))
        },
    )
}

/// The identifier for `chain`, building it on first use.
///
/// Warns once per chain when there is nothing to look contracts up with, since the alternative is
/// silently decoding nothing for a run the user explicitly asked to decode.
fn identifier(
    sources: &ExternalIdentifierConfig,
    chain: Chain,
    deadline: Instant,
) -> Option<Arc<Mutex<ExternalIdentifier>>> {
    let mut identifiers = lock_until(&IDENTIFIERS, deadline)?;
    identifiers
        .entry(chain.id())
        .or_insert_with(|| match sources.storage_identifier(chain) {
            Some(identifier) => Some(Arc::new(Mutex::new(identifier))),
            None => {
                let _ = sh_warn!(
                    "cannot decode external storage on chain {chain}: no matching block explorer \
                     is configured"
                );
                None
            }
        })
        .clone()
}

/// Acquires an external-lookup lock without exceeding the caller's deadline.
fn lock_until<T>(mutex: &Mutex<T>, deadline: Instant) -> Option<MutexGuard<'_, T>> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
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
