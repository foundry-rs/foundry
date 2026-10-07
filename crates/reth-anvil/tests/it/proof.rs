//! tests for `eth_getProof`
//!
//! Anvil's tests compare the proofs with fixed vectors. Here the proofs are verified against the
//! state root instead: the writes reach the state with a block, whose system calls change the
//! system contracts' storage, so the tries differ from anvil's.

use alloy_primitives::{Address, B256, U256, address, fixed_bytes, keccak256};
use alloy_rpc_types::BlockNumberOrTag;
use alloy_trie::{Nibbles, TrieAccount, proof::verify_proof};
use reth_anvil::{EthApi, NodeConfig, spawn};
use std::collections::BTreeMap;

/// Checks the account proof of `address` against the latest state root.
async fn verify_account_proof(api: &EthApi, address: Address) {
    let root =
        api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap().header.state_root;
    let proof = api.get_proof(address, Vec::new(), None).await.unwrap();
    let account = TrieAccount {
        nonce: proof.nonce,
        balance: proof.balance,
        storage_root: proof.storage_hash,
        code_hash: proof.code_hash,
    };
    let key = Nibbles::unpack(keccak256(address));
    verify_proof(root, key, Some(alloy_rlp::encode(account)), proof.account_proof.iter()).unwrap();
}

/// Checks the storage proof of `slot` of `address` against the account's storage root.
async fn verify_storage_proof(api: &EthApi, address: Address, slot: B256) {
    let proof = api.get_proof(address, vec![slot], None).await.unwrap();
    let storage = &proof.storage_proof[0];
    let expected = (!storage.value.is_zero()).then(|| alloy_rlp::encode(storage.value));
    let key = Nibbles::unpack(keccak256(slot));
    verify_proof(proof.storage_hash, key, expected, storage.proof.iter()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_account_proof() {
    let (api, _handle) = spawn(NodeConfig::empty_state()).await;

    let accounts = [
        (address!("0x2031f89b3ea8014eb51a78c316e42af3e0d7695f"), 45000000000000000000_u128),
        (address!("0x33f0fc440b8477fcfbe9d0bf8649e7dea9baedb2"), 1),
        (address!("0x62b0dd4aab2b1a0a04e279e2b828791a10755528"), 1100000000000000000),
        (address!("0x1ed9b1dd266b607ee278726d324b855a093394a6"), 120000000000000000),
    ];
    for (address, balance) in accounts {
        api.anvil_set_balance(address, U256::from(balance)).await.unwrap();
    }
    // The writes reach the state with the next block; anvil's state holds them at once.
    api.mine_one().await.unwrap();

    for (address, _) in accounts {
        verify_account_proof(&api, address).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_storage_proof() {
    let target = address!("0x1ed9b1dd266b607ee278726d324b855a093394a6");

    let (api, _handle) = spawn(NodeConfig::empty_state()).await;
    // Revm clears the storage of an account without balance, nonce, or code; anvil keeps it.
    api.anvil_set_balance(target, U256::ONE).await.unwrap();
    let storage: BTreeMap<U256, B256> =
        serde_json::from_str(include_str!("../../test-data/storage_sample.json")).unwrap();

    for (key, value) in storage {
        api.anvil_set_storage_at(target, key, value).await.unwrap();
    }
    api.mine_one().await.unwrap();

    for slot in [
        fixed_bytes!("0000000000000000000000000000000000000000000000000000000000000022"),
        fixed_bytes!("0000000000000000000000000000000000000000000000000000000000000023"),
        fixed_bytes!("0000000000000000000000000000000000000000000000000000000000000024"),
        fixed_bytes!("0000000000000000000000000000000000000000000000000000000000000100"),
    ] {
        verify_storage_proof(&api, target, slot).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_random_account_proofs() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    for acc in std::iter::repeat_with(Address::random).take(10) {
        let _ = api
            .get_proof(acc, Vec::new(), None)
            .await
            .unwrap_or_else(|_| panic!("Failed to get proof for {acc:?}"));
    }
}
