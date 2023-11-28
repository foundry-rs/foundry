//! tests for `eth_getProof`

use crate::proof::eip1186::verify_proof;
use alloy_rpc_types::EIP1186AccountProofResponse;
use anvil::{spawn, NodeConfig};
use anvil_core::eth::{
    proof::BasicAccount,
    trie::ExtensionLayout,
};
use ethers::{
    abi::ethereum_types::BigEndianHash,
    types::{Address, H256, U256},
    utils::{keccak256, rlp},
};
use foundry_evm::revm::primitives::KECCAK_EMPTY;
use foundry_common::types::{ToEthers, ToAlloy};

mod eip1186;

#[tokio::test(flavor = "multi_thread")]
async fn can_get_proof() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let acc: Address = "0xaaaf5374fce5edbc8e2a8697c15331677e6ebaaa".parse().unwrap();

    let key = U256::zero();
    let value = U256::one();

    api.anvil_set_storage_at(acc.to_alloy(), key.to_alloy(), H256::from_uint(&value).to_alloy()).await.unwrap();

    let proof: EIP1186AccountProofResponse = api.get_proof(acc.to_alloy(), vec![H256::from_uint(&key).to_alloy()], None).await.unwrap();

    let account = BasicAccount {
        nonce: 0.into(),
        balance: 0.into(),
        storage_root: proof.storage_hash.to_ethers(),
        code_hash: KECCAK_EMPTY.to_ethers(),
    };

    let rlp_account = rlp::encode(&account);

    let root: H256 = api.state_root().await.unwrap().to_ethers();
    let acc_proof: Vec<Vec<u8>> = proof
        .account_proof
        .into_iter()
        .map(|node| rlp::decode::<Vec<u8>>(&node).unwrap())
        .collect();

    verify_proof::<ExtensionLayout>(
        &root.0,
        &acc_proof,
        &keccak256(acc.as_bytes())[..],
        Some(rlp_account.as_ref()),
    )
    .unwrap();

    assert_eq!(proof.storage_proof.len(), 1);
    let expected_value = rlp::encode(&value);
    let proof = proof.storage_proof[0].clone();
    let storage_proof: Vec<Vec<u8>> =
        proof.proof.into_iter().map(|node| rlp::decode::<Vec<u8>>(&node).unwrap()).collect();
    let key = H256::from(keccak256(proof.key.0.0));
    verify_proof::<ExtensionLayout>(
        &account.storage_root.0,
        &storage_proof,
        key.as_bytes(),
        Some(expected_value.as_ref()),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_random_account_proofs() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    for acc in std::iter::repeat_with(Address::random).take(10) {
        let _ = api
            .get_proof(acc.to_alloy(), Vec::new(), None)
            .await
            .unwrap_or_else(|_| panic!("Failed to get proof for {acc:?}"));
    }
}
