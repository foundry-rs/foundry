//! tests for `eth_getProof`

use crate::proof::eip1186::verify_proof;
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_rlp::Decodable;
use alloy_rpc_types::EIP1186AccountProofResponse;
use anvil::{spawn, NodeConfig};
use anvil_core::eth::{proof::BasicAccount, trie::ExtensionLayout};
use foundry_evm::revm::primitives::KECCAK_EMPTY;

mod eip1186;

#[tokio::test(flavor = "multi_thread")]
async fn can_get_proof() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let acc: Address = "0xaaaf5374fce5edbc8e2a8697c15331677e6ebaaa".parse().unwrap();

    let key = U256::ZERO;
    let value = U256::from(1);

    api.anvil_set_storage_at(acc, key, B256::from(value)).await.unwrap();

    let proof: EIP1186AccountProofResponse =
        api.get_proof(acc, vec![B256::from(key)], None).await.unwrap();

    let account = BasicAccount {
        nonce: U256::from(0),
        balance: U256::from(0),
        storage_root: proof.storage_hash,
        code_hash: KECCAK_EMPTY,
    };

    let rlp_account = alloy_rlp::encode(&account);

    let root: B256 = api.state_root().await.unwrap();
    let acc_proof: Vec<Vec<u8>> = proof
        .account_proof
        .into_iter()
        .map(|node| Vec::<u8>::decode(&mut &node[..]).unwrap())
        .collect();

    verify_proof::<ExtensionLayout>(
        &root.0,
        &acc_proof,
        &keccak256(acc.as_slice())[..],
        Some(rlp_account.as_ref()),
    )
    .unwrap();

    assert_eq!(proof.storage_proof.len(), 1);
    let expected_value = alloy_rlp::encode(value);
    let proof = proof.storage_proof[0].clone();
    let storage_proof: Vec<Vec<u8>> =
        proof.proof.into_iter().map(|node| Vec::<u8>::decode(&mut &node[..]).unwrap()).collect();
    let key = B256::from(keccak256(proof.key.0 .0));
    verify_proof::<ExtensionLayout>(
        &account.storage_root.0,
        &storage_proof,
        key.as_slice(),
        Some(expected_value.as_ref()),
    )
    .unwrap();
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
