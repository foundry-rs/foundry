use alloy_network::AnyNetwork;
use alloy_primitives::{address, Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_sol_types::{sol, SolCall};
use futures::future::join_all;
use std::collections::HashMap;
use tracing::trace;

use crate::eth::backend::fork::ClientFork;

/// The canonical Multicall3 deployment address (same on all EVM chains).
pub const MULTICALL3_ADDRESS: Address =
    address!("cA11bde05977b3631167028862bE2a173976CA11");

sol! {
    struct Call3 {
        address target;
        bool allowFailure;
        bytes callData;
    }
    function aggregate3(Call3[] calldata calls) external payable returns (bytes[] memory);
}

/// Lightweight check: is this request targeting Multicall3 with aggregate3 selector?
/// Avoids full ABI decode overhead.
pub fn is_multicall3_aggregate(request: &TransactionRequest) -> bool {
    const AGGREGATE3_SELECTOR: [u8; 4] = [0x82, 0xad, 0x56, 0xcb];

    let Some(to) = request.to.as_ref().and_then(|kind| kind.to()) else { return false };
    if *to != MULTICALL3_ADDRESS {
        return false;
    }
    let Some(input) = request.input.input() else { return false };
    input.len() >= 4 && input[..4] == AGGREGATE3_SELECTOR
}

/// A decoded sub-call extracted from a Multicall3 `aggregate3` payload.
/// Only used in tests — production code uses [`is_multicall3_aggregate`] for lightweight detection.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct DecodedSubCall {
    pub target: Address,
    pub call_data: Bytes,
    pub allow_failure: bool,
}

/// Attempts to decode a `TransactionRequest` as a Multicall3 `aggregate3` call.
///
/// Only used in tests — production code uses [`is_multicall3_aggregate`] for lightweight detection.
#[cfg(test)]
pub fn decode_multicall3(request: &TransactionRequest) -> Option<Vec<DecodedSubCall>> {
    // Check that the call targets the Multicall3 contract.
    let to = request.to.as_ref().and_then(|kind| kind.to())?;
    if *to != MULTICALL3_ADDRESS {
        return None;
    }

    // Obtain the call data.
    let data = request.input.input()?;
    if data.len() < 4 {
        return None;
    }

    // ABI-decode as aggregate3.
    let decoded = aggregate3Call::abi_decode(data).ok()?;

    if decoded.calls.is_empty() {
        return None;
    }

    let sub_calls = decoded
        .calls
        .into_iter()
        .map(|c| DecodedSubCall {
            target: c.target,
            call_data: Bytes::copy_from_slice(&c.callData),
            allow_failure: c.allowFailure,
        })
        .collect();

    Some(sub_calls)
}

/// Concurrently prefetches storage slots that a Multicall3 `aggregate3` call will touch.
///
/// This is a best-effort optimisation: every failure is silently ignored so that the
/// subsequent EVM execution still works (it will simply fetch on demand).
///
/// **Phases:**
/// 1. Send ONE `eth_createAccessList` for the entire original Multicall3 request (preserving
///    `from`, `to`, `data`, and using the correct `block_id`).
/// 2. Collect all unique `(address, storage_slot)` pairs from the returned access list.
/// 3. Fetch every slot value via `eth_getStorageAt` concurrently.
/// 4. Batch-insert the results into the fork's `BlockchainDb` cache so that the EVM reads
///    hit cache instead of issuing individual RPC round-trips.
pub async fn prefetch_multicall(
    fork: &ClientFork<AnyNetwork>,
    request: &WithOtherFields<TransactionRequest>,
    block_id: BlockId,
) {
    let provider = fork.provider();

    // Phase 1: ONE eth_createAccessList for the entire multicall request.
    // Using the original request preserves `from`, `to`, `data`, and any other fields,
    // so `msg.sender`-dependent storage access is captured correctly.
    let access_list_result = match provider
        .create_access_list(request)
        .block_id(block_id)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            trace!(target: "multicall_prefetch", error = %e, "eth_createAccessList failed");
            return;
        }
    };

    // Phase 2: Collect all unique (address, slot) pairs.
    let mut slots_by_address: HashMap<Address, Vec<U256>> = HashMap::new();

    for item in access_list_result.access_list.0 {
        let entry = slots_by_address.entry(item.address).or_default();
        for key in item.storage_keys {
            let slot = U256::from_be_bytes(key.0);
            if !entry.contains(&slot) {
                entry.push(slot);
            }
        }
    }

    let total_slots: usize = slots_by_address.values().map(|v| v.len()).sum();
    trace!(
        addresses = slots_by_address.len(),
        total_slots,
        "multicall prefetch: fetching storage slots"
    );

    if total_slots == 0 {
        return;
    }

    // Phase 3: Fetch all storage values concurrently via eth_getStorageAt.
    let fetch_futures: Vec<_> = slots_by_address
        .iter()
        .flat_map(|(addr, slots)| {
            let provider = provider.clone();
            let addr = *addr;
            slots.iter().map(move |slot| {
                let provider = provider.clone();
                let slot = *slot;
                async move {
                    let value = provider
                        .get_storage_at(addr, slot)
                        .block_id(block_id)
                        .await
                        .ok()?;
                    Some((addr, slot, value))
                }
            })
        })
        .collect();

    let fetch_results = join_all(fetch_futures).await;

    // Phase 4: Insert fetched values into the fork's BlockchainDb cache.
    //
    // We acquire the database lock and use `maybe_inner()` (from `MaybeForkedDatabase`) to
    // reach the underlying `BlockchainDb`, then write directly into its storage map.
    let db = fork.database.read().await;
    let blockchain_db = match db.maybe_inner() {
        Ok(bdb) => bdb,
        Err(e) => {
            trace!(error = %e, "multicall prefetch: could not access BlockchainDb");
            return;
        }
    };

    let mut storage = blockchain_db.storage().write();
    let mut inserted = 0usize;

    for result in fetch_results.into_iter().flatten() {
        let (addr, slot, value) = result;
        let account_storage = storage.entry(addr).or_default();
        account_storage.insert(slot, value);
        inserted += 1;
    }

    trace!(inserted, "multicall prefetch: cache populated");
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, bytes, TxKind};

    /// Build a minimal `TransactionRequest` with an explicit `to` and `input`.
    fn make_request(to: Address, input: Bytes) -> TransactionRequest {
        TransactionRequest { to: Some(TxKind::Call(to)), input: input.into(), ..Default::default() }
    }

    #[test]
    fn decode_valid_aggregate3() {
        let calls = vec![
            Call3 {
                target: address!("1111111111111111111111111111111111111111"),
                allowFailure: false,
                callData: bytes!("deadbeef").into(),
            },
            Call3 {
                target: address!("2222222222222222222222222222222222222222"),
                allowFailure: true,
                callData: bytes!("cafebabe").into(),
            },
        ];

        let encoded = aggregate3Call { calls }.abi_encode();
        let request = make_request(MULTICALL3_ADDRESS, Bytes::from(encoded));

        let result = decode_multicall3(&request).expect("should decode");
        assert_eq!(result.len(), 2);

        assert_eq!(result[0].target, address!("1111111111111111111111111111111111111111"));
        assert_eq!(result[0].allow_failure, false);
        assert_eq!(result[0].call_data, bytes!("deadbeef"));

        assert_eq!(result[1].target, address!("2222222222222222222222222222222222222222"));
        assert_eq!(result[1].allow_failure, true);
        assert_eq!(result[1].call_data, bytes!("cafebabe"));
    }

    #[test]
    fn decode_non_multicall_returns_none() {
        let calls = vec![Call3 {
            target: address!("1111111111111111111111111111111111111111"),
            allowFailure: false,
            callData: bytes!("deadbeef").into(),
        }];

        let encoded = aggregate3Call { calls }.abi_encode();
        // Use a different target address — not the Multicall3 address.
        let wrong_to = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let request = make_request(wrong_to, Bytes::from(encoded));

        assert!(decode_multicall3(&request).is_none());
    }

    #[test]
    fn decode_wrong_selector_returns_none() {
        // Right address, but payload starts with a different selector.
        let garbage = Bytes::from(vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03]);
        let request = make_request(MULTICALL3_ADDRESS, garbage);

        assert!(decode_multicall3(&request).is_none());
    }
}
