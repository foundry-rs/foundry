//! general eth api tests

use crate::abi::Greeter;
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256, Uint, address, b256, bytes, utils::Unit};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockId, TransactionRequest,
    state::{AccountOverride, EvmOverrides, StateOverride},
};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, eth::backend::db::SerializableState, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use foundry_test_utils::rpc::next_http_archive_rpc_url;
use revm::{
    context_interface::block::BlobExcessGasAndPrice,
    primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
};
use serde_json::{Value, json};
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread")]
async fn executes_rpc_notification_without_response() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let account = address!("0000000000000000000000000000000000000001");
    let balance = U256::from(42);

    let response = reqwest::Client::new()
        .post(handle.http_endpoint())
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "anvil_setBalance",
            "params": [account, balance],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(response.bytes().await.unwrap().is_empty());
    assert_eq!(handle.http_provider().get_balance(account).await.unwrap(), balance);
}

async fn state_without_block_history() -> (Value, Address, U256, u64) {
    let account = address!("0000000000000000000000000000000000010363");
    let balance = U256::from(10363);
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_balance(account, balance).await.unwrap();
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let mut state = serde_json::to_value(api.serialized_state(false).await.unwrap()).unwrap();
    let state = state.as_object_mut().unwrap();
    state.remove("blocks");
    state.remove("transactions");
    state.remove("historical_states");

    let block = state.get_mut("block").unwrap().as_object_mut().unwrap();
    let beneficiary = block.remove("beneficiary").unwrap();
    block.insert("coinbase".to_string(), beneficiary);

    (Value::Object(state.clone()), account, balance, api.backend.fees().base_fee())
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;

    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let num = api.block_number().unwrap();

    let state = api.serialized_state(false).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let num2 = api.block_number().unwrap();

    // Ref: https://github.com/foundry-rs/foundry/issues/9017
    // Check responses of eth_blockNumber and eth_getBlockByNumber don't deviate after loading state
    let num_from_tag = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .number;
    assert_eq!(num, num2);

    assert_eq!(num, U256::from(num_from_tag));
}

// <https://github.com/foundry-rs/foundry/issues/10331>
#[tokio::test(flavor = "multi_thread")]
async fn test_load_state_continues_saved_timeline() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    // Move the chain's timeline one year ahead of wall-clock time, then mine on it.
    let one_year_ahead =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
            + 31_536_000;
    api.evm_set_next_block_timestamp(one_year_ahead).unwrap();
    api.mine_one().await.unwrap();

    let saved_head_timestamp = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .timestamp;
    assert_eq!(saved_head_timestamp, one_year_ahead);

    let state = api.serialized_state(false).await.unwrap();
    let (api, _handle) = spawn(NodeConfig::test().with_init_state(Some(state))).await;

    api.mine_one().await.unwrap();
    let new_head_timestamp = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .timestamp;

    // The block mined after loading the state must continue the saved timeline instead of
    // falling back to the wall-clock anchor of the fresh node.
    assert!(
        new_head_timestamp >= saved_head_timestamp,
        "block after load_state went back in time: {new_head_timestamp} < {saved_head_timestamp}"
    );
}

// Loading a state whose head has the same number as the fork block must keep the fork time
// anchor: the canonical head stays the fork block, so the saved timeline does not apply.
#[tokio::test(flavor = "multi_thread")]
async fn test_load_state_equal_height_fork_keeps_fork_anchor() {
    // The state source: one block mined a year ahead of wall-clock time.
    let (api_state, _handle_state) = spawn(NodeConfig::test()).await;
    let one_year_ahead =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
            + 31_536_000;
    api_state.evm_set_next_block_timestamp(one_year_ahead).unwrap();
    api_state.mine_one().await.unwrap();
    let state = api_state.serialized_state(false).await.unwrap();

    // The fork source: one block mined on the wall-clock timeline, same height as the state.
    let (api_remote, handle_remote) = spawn(NodeConfig::test()).await;
    api_remote.mine_one().await.unwrap();
    let remote_head_timestamp = api_remote
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .timestamp;

    // Fork the remote head (height 1) and load the state (best height 1 as well): the
    // canonical head keeps being the fork block, so its time anchor must be preserved.
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(handle_remote.http_endpoint()))
            .with_init_state(Some(state)),
    )
    .await;

    api.mine_one().await.unwrap();
    let new_head_timestamp = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .timestamp;

    assert!(
        new_head_timestamp < one_year_ahead,
        "block after load_state jumped to the loaded state's timeline instead of keeping the \
         fork anchor: {new_head_timestamp} >= {one_year_ahead}"
    );
    assert!(
        new_head_timestamp >= remote_head_timestamp,
        "block after load_state went back in time: {new_head_timestamp} < \
         {remote_head_timestamp}"
    );
}

// Loading a legacy account-only state at the fork head must still reset timestamp controls to
// the canonical fork timeline, even though the state has no block environment.
#[tokio::test(flavor = "multi_thread")]
async fn test_load_blockless_state_reanchors_time_to_fork_head() {
    let (api_remote, handle_remote) = spawn(NodeConfig::test()).await;
    api_remote.mine_one().await.unwrap();
    let remote_head = api_remote
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();

    let (api, _handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(handle_remote.http_endpoint()))).await;
    let one_year_ahead = remote_head.timestamp + 31_536_000;
    api.evm_increase_time(U256::from(60)).await.unwrap();
    api.evm_set_next_block_timestamp(one_year_ahead).unwrap();

    let state = SerializableState::default();
    let state = Bytes::from(serde_json::to_vec(&state).unwrap());
    assert!(api.anvil_load_state(state).await.unwrap());

    api.mine_one().await.unwrap();
    let new_head = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();
    assert_eq!(new_head.parent_hash, remote_head.hash);
    assert!(
        new_head.timestamp < one_year_ahead,
        "block after loading a blockless state reused pending timestamp controls: {} >= {}",
        new_head.timestamp,
        one_year_ahead
    );
}

// When `anvil_loadState` rolls an already-advanced fork back to its fork head (state file at
// or below the fork block), the discarded local timeline must not leak into the next block:
// block time re-anchors to the fork head, exactly like `anvil_reset`.
#[tokio::test(flavor = "multi_thread")]
async fn test_load_state_fork_rollback_reanchors_time_to_fork_head() {
    // The state source: a plain node dumped at genesis (height 0).
    let (api_state, _handle_state) = spawn(NodeConfig::test()).await;
    let state = api_state.anvil_dump_state(None).await.unwrap();

    // The fork source: one block mined on the wall-clock timeline.
    let (api_remote, handle_remote) = spawn(NodeConfig::test()).await;
    api_remote.mine_one().await.unwrap();
    let remote_head = api_remote
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();

    // Fork the remote head (height 1), then mine a local block one year ahead of wall-clock.
    let (api, _handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(handle_remote.http_endpoint()))).await;
    let one_year_ahead =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
            + 31_536_000;
    api.evm_set_next_block_timestamp(one_year_ahead).unwrap();
    api.mine_one().await.unwrap();

    // Loading the height-0 state rolls the canonical head back to the exact fork head.
    assert!(api.anvil_load_state(state).await.unwrap());
    let head = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();
    assert_eq!(head.hash, remote_head.hash, "canonical head must return to the exact fork head");

    // The next block continues from the fork head's timeline, not from the discarded
    // future-dated local block.
    api.mine_one().await.unwrap();
    let new_head = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();
    assert_eq!(new_head.parent_hash, remote_head.hash);
    assert!(
        new_head.timestamp >= remote_head.timestamp,
        "block after fork rollback went back in time: {} < {}",
        new_head.timestamp,
        remote_head.timestamp
    );
    assert!(
        new_head.timestamp < one_year_ahead,
        "block after the fork rollback reused the discarded local timeline: {} >= {}",
        new_head.timestamp,
        one_year_ahead
    );
}

// A state file can carry competing blocks at the same height: dump an older state, keep mining,
// load that older dump back, then mine a replacement block. Loading such a file must select the
// replacement as the canonical head and continue its timeline.
#[tokio::test(flavor = "multi_thread")]
async fn test_load_state_stale_blocks_preserve_canonical_head() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let one_year_ahead = now + 31_536_000;
    let two_years_ahead = now + 63_072_000;

    // Head at height 1, one year ahead: this is the canonical timeline to preserve.
    api.evm_set_next_block_timestamp(one_year_ahead).unwrap();
    api.mine_one().await.unwrap();
    let older_dump = api.anvil_dump_state(None).await.unwrap();

    // Advance to height 2, two years ahead, then roll best back to height 1 by loading the
    // older dump: the height-2 block stays in storage above the restored best block.
    api.evm_set_next_block_timestamp(two_years_ahead).unwrap();
    api.mine_one().await.unwrap();
    assert!(api.anvil_load_state(older_dump).await.unwrap());

    // Mine a replacement at height 2. The stale height-2 block remains in storage, while the
    // replacement becomes canonical.
    api.evm_set_next_block_timestamp(one_year_ahead + 1).unwrap();
    api.mine_one().await.unwrap();
    let canonical_head = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();
    let state = api.serialized_state(false).await.unwrap();
    assert_eq!(state.blocks.iter().filter(|block| block.header.number == 2).count(), 2);
    assert_eq!(state.best_block_number, Some(2));

    // A fresh node selects the replacement as its head and mines on top of it.
    let (api, _handle) = spawn(NodeConfig::test().with_init_state(Some(state))).await;
    assert_eq!(api.backend.best_hash(), canonical_head.hash);
    api.mine_one().await.unwrap();
    let new_head = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .clone();
    assert_eq!(new_head.parent_hash, canonical_head.hash);
    assert!(
        new_head.timestamp >= one_year_ahead && new_head.timestamp < two_years_ahead,
        "block after loading a dump with stale blocks must continue the canonical timeline: \
         got {}, expected within [{}, {})",
        new_head.timestamp,
        one_year_ahead,
        two_years_ahead
    );
}

// <https://github.com/foundry-rs/foundry/issues/12645>
#[tokio::test(flavor = "multi_thread")]
async fn finalized_block_hash_consistent_after_load_state() {
    use alloy_eips::BlockNumberOrTag;

    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;

    api.mine_one().await.unwrap();

    // Get the original genesis block hash
    let original_genesis = api.block_by_number(BlockNumberOrTag::Number(0)).await.unwrap().unwrap();
    let original_genesis_hash = original_genesis.header.hash;

    let state = api.serialized_state(false).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    // Load state with a different genesis timestamp.
    // The new instance will create its own genesis block with a different timestamp,
    // but then load_state should overwrite it. The bug is that genesis_hash field isn't updated.
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_genesis_timestamp(Some(original_genesis.header.timestamp + 1000))
            .with_init_state_path(state_file),
    )
    .await;

    // Query finalized block - should return genesis (block 0) since best_number is small
    let finalized_block = api.block_by_number(BlockNumberOrTag::Finalized).await.unwrap().unwrap();
    let finalized_hash = finalized_block.header.hash;
    let finalized_number = finalized_block.header.number;

    // Query block by the finalized block's number directly
    let block_by_number =
        api.block_by_number(BlockNumberOrTag::Number(finalized_number)).await.unwrap().unwrap();
    let block_by_number_hash = block_by_number.header.hash;

    // Verify the loaded genesis matches the original
    assert_eq!(
        block_by_number_hash, original_genesis_hash,
        "Loaded genesis should match original genesis hash"
    );

    // Both finalized and block 0 should return the same hash
    assert_eq!(
        finalized_hash, block_by_number_hash,
        "Finalized block hash should match block queried by number"
    );

    // Also verify Earliest block tag returns consistent hash
    let earliest_block = api.block_by_number(BlockNumberOrTag::Earliest).await.unwrap().unwrap();
    assert_eq!(
        earliest_block.header.hash, original_genesis_hash,
        "Earliest block hash should match original genesis hash"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state_legacy() {
    let state_file = "test-data/state-dump-legacy.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}

// <https://github.com/foundry-rs/foundry/issues/10363>
#[tokio::test(flavor = "multi_thread")]
async fn can_load_state_without_block_history() {
    let (state, account, balance, next_base_fee) = state_without_block_history().await;
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let (api, handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;
    let provider = handle.http_provider();

    assert_eq!(provider.get_balance(account).await.unwrap(), balance);
    assert_eq!(api.block_number().unwrap(), U256::from(2));

    let checkpoint =
        api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(checkpoint.header.number, 2);
    assert_eq!(checkpoint.header.hash, api.backend.best_hash());
    assert_eq!(api.backend.fees().base_fee(), next_base_fee);

    api.mine_one().await.unwrap();
    let latest = api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(latest.header.number, 3);
    assert_eq!(latest.header.parent_hash, checkpoint.header.hash);
    assert_eq!(latest.header.base_fee_per_gas, Some(next_base_fee));

    let dumped = api.serialized_state(false).await.unwrap();
    assert!(dumped.blocks.iter().any(|block| block.header.number == 2));

    let (reloaded, _handle) = spawn(NodeConfig::test().with_init_state(Some(dumped))).await;
    let reloaded_latest =
        reloaded.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(reloaded_latest.header.number, 3);
    assert_eq!(reloaded_latest.header.hash, latest.header.hash);
}

// <https://github.com/foundry-rs/foundry/issues/10363>
#[tokio::test(flavor = "multi_thread")]
async fn can_load_state_without_block_history_at_runtime() {
    let (mut state, _, _, _) = state_without_block_history().await;
    let loaded_beneficiary = address!("0000000000000000000000000000000000010363");
    state["block"]["coinbase"] = json!(loaded_beneficiary);

    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.backend.set_coinbase(address!("0000000000000000000000000000000000000001"));
    api.mine_one().await.unwrap();
    let parent =
        api.block_by_number(alloy_eips::BlockNumberOrTag::Number(1)).await.unwrap().unwrap();
    api.mine_one().await.unwrap();
    let previous =
        api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();

    api.anvil_load_state(Bytes::from(serde_json::to_vec(&state).unwrap())).await.unwrap();

    let checkpoint =
        api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(checkpoint.header.number, 2);
    assert_eq!(checkpoint.header.parent_hash, parent.header.hash);
    assert_eq!(checkpoint.header.beneficiary, loaded_beneficiary);
    assert_ne!(checkpoint.header.hash, previous.header.hash);
    assert_eq!(checkpoint.header.hash, api.backend.best_hash());

    api.mine_one().await.unwrap();
    let latest = api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(latest.header.number, 3);
    assert_eq!(latest.header.parent_hash, checkpoint.header.hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn state_without_block_history_restores_osaka_blob_excess_gas() {
    let (mut state, _, _, _) = state_without_block_history().await;
    let blob_params = alloy_eips::eip7840::BlobParams::osaka();
    let target_blob_gas = blob_params.target_blob_gas_per_block();
    state["block"]["basefee"] = json!(1);
    state["block"]["blob_excess_gas_and_price"]["excess_blob_gas"] = json!(target_blob_gas);
    state["block"]["blob_excess_gas_and_price"]["blob_gasprice"] =
        json!(blob_params.calc_blob_fee(target_blob_gas));

    let state = serde_json::from_value(state).unwrap();
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Osaka.into()))
            .with_init_state(Some(state)),
    )
    .await;

    assert_eq!(api.backend.fees().excess_blob_gas_and_price().unwrap().excess_blob_gas, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_nonempty_block_history_without_best_block() {
    let (source, _handle) = spawn(NodeConfig::test()).await;
    source.backend.set_coinbase(address!("0000000000000000000000000000000000000002"));
    source.mine_one().await.unwrap();
    source.mine_one().await.unwrap();
    source.mine_one().await.unwrap();
    let mut state = source.serialized_state(false).await.unwrap();
    state.blocks.retain(|block| block.header.number != 3);
    assert!(!state.blocks.is_empty());
    let incoming_hash =
        state.blocks.iter().find(|block| block.header.number == 2).unwrap().header.hash_slow();

    let (api, _handle) = spawn(NodeConfig::test()).await;
    let original_coinbase = address!("0000000000000000000000000000000000000001");
    api.backend.set_coinbase(original_coinbase);
    api.mine_one().await.unwrap();
    let original =
        api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert!(api.block_by_hash(incoming_hash).await.unwrap().is_none());

    let err =
        api.anvil_load_state(Bytes::from(serde_json::to_vec(&state).unwrap())).await.unwrap_err();
    assert!(err.to_string().contains("Best hash not found for best number 3"));
    assert_eq!(api.backend.best_number(), original.header.number);
    assert_eq!(api.backend.best_hash(), original.header.hash);
    assert_eq!(api.backend.coinbase(), original_coinbase);
    assert!(api.block_by_hash(incoming_hash).await.unwrap().is_none());

    api.mine_one().await.unwrap();
    let latest = api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(latest.header.number, original.header.number + 1);
    assert_eq!(latest.header.parent_hash, original.header.hash);
    assert_eq!(latest.header.beneficiary, original_coinbase);
}

#[tokio::test(flavor = "multi_thread")]
async fn loaded_state_fees_use_selected_head() {
    let (source, _handle) = spawn(NodeConfig::test()).await;
    source.mine_one().await.unwrap();
    let mut state = source.serialized_state(false).await.unwrap();
    let selected_hash = source.backend.best_hash();
    let selected_next_base_fee = source.backend.fees().base_fee();

    source.mine_one().await.unwrap();
    let newer_state = source.serialized_state(false).await.unwrap();
    let newer_block =
        newer_state.blocks.into_iter().find(|block| block.header.number == 2).unwrap();
    assert_ne!(source.backend.fees().base_fee(), selected_next_base_fee);
    state.blocks.push(newer_block);

    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.anvil_load_state(Bytes::from(serde_json::to_vec(&state).unwrap())).await.unwrap();
    assert_eq!(api.backend.best_hash(), selected_hash);
    assert_eq!(api.backend.fees().base_fee(), selected_next_base_fee);

    api.mine_one().await.unwrap();
    let latest = api.block_by_number(alloy_eips::BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(latest.header.parent_hash, selected_hash);
    assert_eq!(latest.header.base_fee_per_gas, Some(selected_next_base_fee));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state_legacy_stress() {
    let state_file = "test-data/state-dump-legacy-stress.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state() {
    let state_file = "test-data/state-dump.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_make_sure_historical_state_is_not_cleared_on_dump() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();

    let address = greeter.address();

    let _tx = greeter
        .setGreeting("World!".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    api.mine_one().await.unwrap();

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(3));

    // Makes sure historical states of the new instance are not cleared.
    let code = provider.get_code_at(*address).block_id(BlockId::number(2)).await.unwrap();

    assert_ne!(code, Bytes::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_preserve_historical_states_between_dump_and_load() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();

    let address = greeter.address();

    let deploy_blk_num = provider.get_block_number().await.unwrap();

    let tx = greeter
        .setGreeting("World!".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let change_greeting_blk_num = tx.block_number.unwrap();

    api.mine_one().await.unwrap();

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let (api, handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(3));

    let provider = handle.http_provider();

    let greeter = Greeter::new(*address, provider);

    let greeting_at_init =
        greeter.greet().block(BlockId::number(deploy_blk_num)).call().await.unwrap();

    assert_eq!(greeting_at_init, "Hello");

    let greeting_after_change =
        greeter.greet().block(BlockId::number(change_greeting_blk_num)).call().await.unwrap();

    assert_eq!(greeting_after_change, "World!");
}

#[tokio::test(flavor = "multi_thread")]
async fn state_dump_is_deterministic() {
    let timestamp = 1_700_000_000u64;
    let (api, handle) = spawn(NodeConfig::test().with_genesis_timestamp(timestamp.into())).await;
    let provider = handle.http_provider();
    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();
    greeter.setGreeting("World!".to_string()).send().await.unwrap().watch().await.unwrap();
    api.mine_one().await.unwrap();

    let dump = api.anvil_dump_state(Some(true)).await.unwrap();
    assert_eq!(dump, api.anvil_dump_state(Some(true)).await.unwrap());

    let (loaded_api, _handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(timestamp.into())).await;
    assert!(loaded_api.anvil_load_state(dump.clone()).await.unwrap());
    assert_eq!(dump, loaded_api.anvil_dump_state(Some(true)).await.unwrap());
}

// <https://github.com/foundry-rs/foundry/issues/9053>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    let bob = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let alice = address!("0x9276449EaC5b4f7Bc17cFC6700f7BeeB86F9bCd0");

    let provider = handle.http_provider();

    let init_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let init_balance_alice = provider.get_balance(alice).await.unwrap();

    let value = Unit::ETHER.wei().saturating_mul(U256::from(1)); // 1 ether
    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let serialized_state = api.serialized_state(false).await.unwrap();

    let state_dump_block = api.block_number().unwrap();

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070686u64)) // Forked chain has moved forward
            .with_init_state(Some(serialized_state)),
    )
    .await;

    // Ensure the initial block number is the fork_block_number and not the state_dump_block
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(21070686u64));
    assert_ne!(block_number, state_dump_block);

    let provider = handle.http_provider();

    let restart_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let restart_balance_alice = provider.get_balance(alice).await.unwrap();

    assert_eq!(init_nonce_bob + 1, restart_nonce_bob);

    assert_eq!(init_balance_alice + value, restart_balance_alice);

    // Send another tx to check if the state is preserved

    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let balance_alice = provider.get_balance(alice).await.unwrap();

    let tx = TransactionRequest::default()
        .with_to(alice)
        .with_value(value)
        .with_from(bob)
        .with_nonce(nonce_bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let latest_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let latest_balance_alice = provider.get_balance(alice).await.unwrap();

    assert_eq!(nonce_bob + 1, latest_nonce_bob);

    assert_eq!(balance_alice + value, latest_balance_alice);
}

// <https://github.com/foundry-rs/foundry/issues/10501>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state_keeps_number_opcode_in_sync() {
    let target = address!("0000000000000000000000000000000000010501");
    let bob = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    // Runtime code: NUMBER, PUSH0, SSTORE, STOP.
    api.anvil_set_code(target, bytes!("435f5500")).await.unwrap();
    api.mine_one().await.unwrap();

    let serialized_state = api.serialized_state(false).await.unwrap();

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070686u64))
            .with_init_state(Some(serialized_state)),
    )
    .await;
    let provider = handle.http_provider();

    let current_block = api.block_number().unwrap().to::<u64>();
    assert_eq!(current_block, 21070686u64);

    let tx = TransactionRequest::default().with_to(target).with_from(bob);
    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let mined_block = receipt.block_number.unwrap();

    let stored = provider.get_storage_at(target, U256::ZERO).await.unwrap();
    assert_eq!(stored, U256::from(mined_block));
    assert_eq!(stored, U256::from(current_block + 1));
}

// <https://github.com/foundry-rs/foundry/issues/9539>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state_with_greater_state_block() {
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    api.mine_one().await.unwrap();

    let block_number = api.block_number().unwrap();

    let serialized_state = api.serialized_state(false).await.unwrap();

    assert_eq!(serialized_state.best_block_number, Some(block_number.to::<u64>()));

    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)) // Forked chain has moved forward
            .with_init_state(Some(serialized_state)),
    )
    .await;

    let new_block_number = api.block_number().unwrap();

    assert_eq!(new_block_number, block_number);
}

// <https://github.com/foundry-rs/foundry/issues/10488>
#[tokio::test(flavor = "multi_thread")]
async fn computes_next_base_fee_after_loading_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let bob = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let alice = address!("0x9276449EaC5b4f7Bc17cFC6700f7BeeB86F9bCd0");

    let provider = handle.http_provider();

    let base_fee_empty_chain = api.backend.fees().base_fee();

    let value = Unit::ETHER.wei().saturating_mul(U256::from(1)); // 1 ether
    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let _receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let base_fee_after_one_tx = api.backend.fees().base_fee();
    // the test is meaningless if this does not hold
    assert_ne!(base_fee_empty_chain, base_fee_after_one_tx);

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;
    let base_fee_after_reload = api.backend.fees().base_fee();
    assert_eq!(base_fee_after_reload, base_fee_after_one_tx);
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_deserialization_v1_2() {
    let old_format = r#"{
        "block": {
            "number": "0x5",
            "coinbase": "0x1234567890123456789012345678901234567890",
            "timestamp": "0x688c83b5",
            "gas_limit": "0x1c9c380",
            "basefee": "0x3b9aca00",  
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
            "blob_excess_gas_and_price": {
                "excess_blob_gas": 173990704,
                "blob_gasprice": 43056053164891617135028
            }
        },
        "accounts": {},
        "best_block_number": "0x5",
        "blocks": [],
        "transactions": []
    }"#;

    let state: SerializableState = serde_json::from_str(old_format).unwrap();
    assert!(state.block.is_some());
    let block_env = state.block.unwrap();
    assert_eq!(block_env.number, U256::from(5));
    // Verify coinbase was converted to beneficiary
    assert_eq!(block_env.beneficiary, address!("0x1234567890123456789012345678901234567890"));
    let blob = block_env.blob_excess_gas_and_price.unwrap();
    assert_eq!(blob.excess_blob_gas, 173990704);
    assert_eq!(blob.blob_gasprice, 43056053164891617135028);

    // New format with beneficiary and numeric values
    let new_format = r#"{
        "block": {
            "number": 6,
            "beneficiary": "0x1234567890123456789012345678901234567891",
            "timestamp": 1751619509,
            "gas_limit": 30000000,
            "basefee": 1000000000,
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
            "blob_excess_gas_and_price": {
                "excess_blob_gas": 0,
                "blob_gasprice": 1
            }
        },
        "accounts": {},
        "best_block_number": 6,
        "blocks": [],
        "transactions": []
    }"#;

    let state: SerializableState = serde_json::from_str(new_format).unwrap();
    assert!(state.block.is_some());
    let block_env = state.block.unwrap();
    assert_eq!(block_env.number, U256::from(6));
    assert_eq!(block_env.beneficiary, address!("0x1234567890123456789012345678901234567891"));
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_mixed_formats_deserialization_v1_2() {
    let mixed_format = json!({
        "block": {
            "number": "0x3",
            "coinbase": "0x1111111111111111111111111111111111111111",
            "timestamp": 1751619509,
            "gas_limit": "0x1c9c380",
            "basefee": 1000000000,
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
            "blob_excess_gas_and_price": {
                "excess_blob_gas": 0,
                "blob_gasprice": 1
            }
        },
        "accounts": {},
        "best_block_number": 3,
        "blocks": [],
        "transactions": []
    });

    let state: SerializableState = serde_json::from_str(&mixed_format.to_string()).unwrap();
    let block_env = state.block.unwrap();

    assert_eq!(block_env.number, U256::from(3));
    assert_eq!(block_env.beneficiary, address!("0x1111111111111111111111111111111111111111"));
    assert_eq!(block_env.timestamp, U256::from(1751619509));
    assert_eq!(block_env.gas_limit, 0x1c9c380);
    assert_eq!(block_env.basefee, 1_000_000_000);
    assert_eq!(block_env.difficulty, U256::ZERO);
    assert_eq!(
        block_env.prevrandao.unwrap(),
        b256!("ecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5")
    );

    let blob = block_env.blob_excess_gas_and_price.unwrap();
    assert_eq!(blob.excess_blob_gas, 0);
    assert_eq!(blob.blob_gasprice, 1);

    assert_eq!(state.best_block_number, Some(3));
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_optional_fields_deserialization_v1_2() {
    let partial_old_format = json!({
        "block": {
            "number": "0x1",
            "coinbase": "0x0000000000000000000000000000000000000000",
            "timestamp": "0x688c83b5",
            "gas_limit": "0x1c9c380",
            "basefee": "0x3b9aca00",
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5"
            // Missing blob_excess_gas_and_price - should be None
        },
        "accounts": {},
        "best_block_number": "0x1"
        // Missing blocks and transactions arrays - should default to empty
    });

    let state: SerializableState = serde_json::from_str(&partial_old_format.to_string()).unwrap();

    let block_env = state.block.unwrap();
    assert_eq!(block_env.number, U256::from(1));
    assert_eq!(block_env.beneficiary, address!("0x0000000000000000000000000000000000000000"));
    assert_eq!(block_env.timestamp, U256::from(0x688c83b5));
    assert_eq!(block_env.gas_limit, 0x1c9c380);
    assert_eq!(block_env.basefee, 0x3b9aca00);
    assert_eq!(block_env.difficulty, U256::ZERO);
    assert_eq!(
        block_env.prevrandao.unwrap(),
        b256!("ecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5")
    );
    assert_eq!(
        block_env.blob_excess_gas_and_price,
        Some(BlobExcessGasAndPrice::new(0, BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE))
    );

    assert_eq!(state.best_block_number, Some(1));
    assert!(state.blocks.is_empty());
    assert!(state.transactions.is_empty());
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_state_dump_deserialization_v1_2() {
    let tmp = tempfile::tempdir().unwrap();
    let old_state_file = tmp.path().join("old_state.json");

    // A simple state dump with a single block containing one transaction of a Counter contract
    // deployment.
    let old_state_json = json!({
      "block": {
        "number": "0x1",
        "coinbase": "0x0000000000000000000000000000000000000001",
        "timestamp": "0x688c83b5",
        "gas_limit": "0x1c9c380",
        "basefee": "0x3b9aca00",
        "difficulty": "0x0",
        "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
        "blob_excess_gas_and_price": {
          "excess_blob_gas": 0,
          "blob_gasprice": 1
        }
      },
      "accounts": {
        "0x0000000000000000000000000000000000000000": {
          "nonce": 0,
          "balance": "0x26481",
          "code": "0x",
          "storage": {}
        },
        "0x14dc79964da2c08b23698b3d3cc7ca32193d9955": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x23618e81e3f5cdf7f54c3d65f7fbc0abf5b21e8f": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x4e59b44847b379578588920ca78fbf26c0b4956c": {
          "nonce": 0,
          "balance": "0x0",
          "code": "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3",
          "storage": {}
        },
        "0x5fbdb2315678afecb367f032d93f642f64180aa3": {
          "nonce": 1,
          "balance": "0x0",
          "code": "0x608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
          "storage": {}
        },
        "0x70997970c51812dc3a010c7d01b50e0d17dc79c8": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x90f79bf6eb2c4f870365e785982e1f101e93b906": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x976ea74026e726554db657fa54763abd0c3a0aa9": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x9965507d1a55bcc2695c58ba16fb37d819b0a4dc": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0xa0ee7a142d267c1f36714e4a8f75612f20a79720": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266": {
          "nonce": 1,
          "balance": "0x21e19e03b1e9e55d17f",
          "code": "0x",
          "storage": {}
        }
      },
      "best_block_number": "0x1",
      "blocks": [
        {
          "header": {
            "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "miner": "0x0000000000000000000000000000000000000000",
            "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "receiptsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "difficulty": "0x0",
            "number": "0x0",
            "gasLimit": "0x1c9c380",
            "gasUsed": "0x0",
            "timestamp": "0x688c83b0",
            "extraData": "0x",
            "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "nonce": "0x0000000000000000",
            "baseFeePerGas": "0x3b9aca00",
            "withdrawalsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "blobGasUsed": "0x0",
            "excessBlobGas": "0x0",
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"
          },
          "transactions": [],
          "ommers": []
        },
        {
          "header": {
            "parentHash": "0x25097583380d90c4ac42b454ed7d2f59450ed3a16fdcf7f7bd93295aa126a901",
            "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "miner": "0x0000000000000000000000000000000000000000",
            "stateRoot": "0x6e005b459ac9acefa5f47fd2d7ff8ca81a91794fdc5f7fbc3e2faeeaefe5d516",
            "transactionsRoot": "0x59f0457ec18e2181c186f49d9ac911b33b5f4f55db5c494022147346bcfc9837",
            "receiptsRoot": "0x88ac48b910f796aab7407814203b3a15a04a812f387e92efeccc92a2ecf809da",
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "difficulty": "0x0",
            "number": "0x1",
            "gasLimit": "0x1c9c380",
            "gasUsed": "0x26481",
            "timestamp": "0x688c83b5",
            "extraData": "0x",
            "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "nonce": "0x0000000000000000",
            "baseFeePerGas": "0x3b9aca00",
            "withdrawalsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "blobGasUsed": "0x0",
            "excessBlobGas": "0x0",
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"
          },
          "transactions": [
            {
              "transaction": {
                "type": "0x2",
                "chainId": "0x7a69",
                "nonce": "0x0",
                "gas": "0x31c41",
                "maxFeePerGas": "0x77359401",
                "maxPriorityFeePerGas": "0x1",
                "to": null,
                "value": "0x0",
                "accessList": [],
                "input": "0x6080604052348015600e575f5ffd5b506101e18061001c5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
                "r": "0xa7398e28ca9a56b423cab87aeb3612378bac9c5684aaf778a78943f2637fd731",
                "s": "0x583511da658f564253c8c0f9ee1820ef370f23556be504b304ac1292f869d9a0",
                "yParity": "0x0",
                "v": "0x0",
                "hash": "0x9e4846328caa09cbe8086d11b7e115adf70390e79ff203d8e5f37785c2a890be"
              },
              "impersonated_sender": null
            }
          ],
          "ommers": []
        }
      ],
      "transactions": [
        {
          "info": {
            "transaction_hash": "0x9e4846328caa09cbe8086d11b7e115adf70390e79ff203d8e5f37785c2a890be",
            "transaction_index": 0,
            "from": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "to": null,
            "contract_address": "0x5fbdb2315678afecb367f032d93f642f64180aa3",
            "traces": [
              {
                "parent": null,
                "children": [],
                "idx": 0,
                "trace": {
                  "depth": 0,
                  "success": true,
                  "caller": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                  "address": "0x5fbdb2315678afecb367f032d93f642f64180aa3",
                  "maybe_precompile": false,
                  "selfdestruct_address": null,
                  "selfdestruct_refund_target": null,
                  "selfdestruct_transferred_value": null,
                  "kind": "CREATE",
                  "value": "0x0",
                  "data": "0x6080604052348015600e575f5ffd5b506101e18061001c5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
                  "output": "0x608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
                  "gas_used": 96345,
                  "gas_limit": 143385,
                  "gas_refund_counter": 0,
                  "status": "Return",
                  "steps": [],
                  "decoded": null
                },
                "logs": [],
                "ordering": []
              }
            ],
            "exit": "Return",
            "out": "0x608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
            "nonce": 0,
            "gas_used": 156801
          },
          "receipt": {
            "type": "0x2",
            "status": "0x1",
            "cumulativeGasUsed": "0x26481",
            "logs": [],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
          },
          "block_hash": "0x313ea0d32d662434a55a20d7c58544e6baaea421b6eccf4b68392dec2a76d771",
          "block_number": 1
        }
      ],
      "historical_states": null
    });

    // Write the old state to file.
    foundry_common::fs::write_json_file(&old_state_file, &old_state_json).unwrap();

    // Test deserializing the old state dump directly.
    let deserialized_state: SerializableState = serde_json::from_value(old_state_json).unwrap();

    // Verify the old state was loaded correctly with `coinbase` to `beneficiary` conversion.
    let block_env = deserialized_state.block.unwrap();
    assert_eq!(block_env.number, U256::from(1));
    assert_eq!(block_env.beneficiary, address!("0000000000000000000000000000000000000001"));
    assert_eq!(block_env.gas_limit, 0x1c9c380);
    assert_eq!(block_env.basefee, 0x3b9aca00);

    // Verify best_block_number hex string parsing.
    assert_eq!(deserialized_state.best_block_number, Some(1));

    // Verify account data was preserved.
    assert_eq!(deserialized_state.accounts.len(), 13);

    // Test specific accounts from the old dump.
    let deployer_addr = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap();
    let deployer_account = deserialized_state.accounts.get(&deployer_addr).unwrap();
    assert_eq!(deployer_account.nonce, 1);
    assert_eq!(deployer_account.balance, U256::from_str("0x21e19e03b1e9e55d17f").unwrap());

    // Test contract account.
    let contract_addr = "0x5fbdb2315678afecb367f032d93f642f64180aa3".parse().unwrap();
    let contract_account = deserialized_state.accounts.get(&contract_addr).unwrap();
    assert_eq!(contract_account.nonce, 1);
    assert_eq!(contract_account.balance, U256::ZERO);
    assert!(!contract_account.code.is_empty());

    // Verify blocks and transactions are preserved.
    assert_eq!(deserialized_state.blocks.len(), 2);
    assert_eq!(deserialized_state.transactions.len(), 1);

    // Test that Anvil can load this old state dump.
    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(&old_state_file)).await;

    // Verify the state was loaded correctly.
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(1));

    // Verify account balances are preserved.
    let provider = _handle.http_provider();
    let deployer_balance = provider.get_balance(deployer_addr).await.unwrap();
    assert_eq!(deployer_balance, U256::from_str("0x21e19e03b1e9e55d17f").unwrap());
    let contract_balance = provider.get_balance(contract_addr).await.unwrap();
    assert_eq!(contract_balance, U256::ZERO);

    // Verify contract code is preserved.
    let contract_code = provider.get_code_at(contract_addr).await.unwrap();
    assert!(!contract_code.is_empty());
}

// Ensures the BLOCKHASH opcode sees the real hashes of loaded blocks after
// `anvil_loadState`. The EVM-level block hash cache used to stay empty after loading state,
// so BLOCKHASH silently returned EmptyDB's placeholder hash for imported blocks.
#[tokio::test(flavor = "multi_thread")]
async fn blockhash_opcode_consistent_after_load_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let block1_hash = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Number(1))
        .await
        .unwrap()
        .unwrap()
        .header
        .hash;

    let state = api.serialized_state(false).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    // Runtime code returning BLOCKHASH(1):
    // PUSH1 1, BLOCKHASH, PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN
    let code = Bytes::from_str("0x60014060005260206000f3").unwrap();
    let target = address!("00000000000000000000000000000000000b10c4");

    let mut overrides = StateOverride::default();
    overrides.insert(target, AccountOverride { code: Some(code), ..Default::default() });

    let tx = TransactionRequest::default().with_to(target);
    let res = api
        .call(WithOtherFields::new(tx), None, EvmOverrides::new(Some(overrides), None))
        .await
        .unwrap();

    assert_eq!(B256::from_slice(res.as_ref()), block1_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn blockhash_opcode_consistent_after_loading_older_state() {
    let (source_api, _source_handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(Some(1_000_000_u64))).await;
    source_api.mine_one().await.unwrap();
    source_api.mine_one().await.unwrap();

    let block1_hash = source_api
        .block_by_number(alloy_eips::BlockNumberOrTag::Number(1))
        .await
        .unwrap()
        .unwrap()
        .header
        .hash;
    let state = source_api.serialized_state(false).await.unwrap();

    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.anvil_mine(Some(U256::from(258)), None).await.unwrap();
    api.anvil_load_state(Bytes::from(serde_json::to_vec(&state).unwrap())).await.unwrap();

    assert_eq!(api.block_number().unwrap(), U256::from(2));

    // Runtime code returning BLOCKHASH(1):
    // PUSH1 1, BLOCKHASH, PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN
    let code = Bytes::from_str("0x60014060005260206000f3").unwrap();
    let target = address!("00000000000000000000000000000000000b10c4");

    let mut overrides = StateOverride::default();
    overrides.insert(target, AccountOverride { code: Some(code), ..Default::default() });

    let tx = TransactionRequest::default().with_to(target);
    let res = api
        .call(WithOtherFields::new(tx), None, EvmOverrides::new(Some(overrides), None))
        .await
        .unwrap();

    assert_eq!(B256::from_slice(res.as_ref()), block1_hash);
}
