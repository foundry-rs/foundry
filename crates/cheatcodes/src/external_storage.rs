//! Storage layouts for contracts that aren't part of the local project.
//!
//! [`foundry_common::external_storage`] turns a verified source into a storage layout and caches
//! the result; [`ExternalIdentifier`] finds that source on Sourcify or a block explorer. This
//! module is the seam between them, and owns the state that has to outlive a single lookup.

use alloy_primitives::{
    Address,
    map::{AddressMap, HashMap},
};
use foundry_common::external_storage::fetch_external_storage_layouts;
use foundry_compilers::artifacts::StorageLayout;
use foundry_config::Chain;
use foundry_evm_traces::identifier::{ExternalIdentifier, ExternalIdentifierConfig};
use std::sync::{Arc, LazyLock, Mutex};

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
    fetch_external_storage_layouts(chain, addresses, |unresolved| {
        let Some(identifier) = identifier(sources, chain) else { return Default::default() };
        let mut identifier = identifier.lock().unwrap_or_else(|err| err.into_inner());
        foundry_common::block_on(identifier.get_implementations(unresolved))
    })
}

/// The identifier for `chain`, building it on first use.
///
/// Warns once per chain when there is nothing to look contracts up with, since the alternative is
/// silently decoding nothing for a run the user explicitly asked to decode.
fn identifier(
    sources: &ExternalIdentifierConfig,
    chain: Chain,
) -> Option<Arc<Mutex<ExternalIdentifier>>> {
    let mut identifiers = IDENTIFIERS.lock().unwrap_or_else(|err| err.into_inner());
    identifiers
        .entry(chain.id())
        .or_insert_with(|| match sources.identifier(Some(chain)) {
            Some(identifier) => Some(Arc::new(Mutex::new(identifier))),
            None => {
                let _ = sh_warn!(
                    "cannot decode external storage on chain {chain}: no block explorer is \
                     configured for it and Sourcify is unavailable"
                );
                None
            }
        })
        .clone()
}
