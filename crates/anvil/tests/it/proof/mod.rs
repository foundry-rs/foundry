//! tests for `eth_getProof`

use std::str::FromStr;

use alloy_primitives::{address, fixed_bytes, hex, keccak256, Address, Bytes, U256};
use anvil::{eth::EthApi, spawn, NodeConfig};
use anvil_core::eth::{proof::BasicAccount, trie::ExtensionLayout};
use foundry_evm::revm::primitives::KECCAK_EMPTY;

async fn verify_proof(api: &EthApi, address: Address, proof: impl IntoIterator<Item = &str>) {
    let expected_proof =
        proof.into_iter().map(Bytes::from_str).collect::<Result<Vec<_>, _>>().unwrap();
    let proof = api.get_proof(address, Vec::new(), None).await.unwrap();

    assert_eq!(proof.account_proof, expected_proof);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_account_proof() {
    let (api, _handle) = spawn(NodeConfig::empty_state()).await;

    api.anvil_set_balance(
        address!("2031f89b3ea8014eb51a78c316e42af3e0d7695f"),
        U256::from(45000000000000000000_u128),
    )
    .await
    .unwrap();
    api.anvil_set_balance(address!("33f0fc440b8477fcfbe9d0bf8649e7dea9baedb2"), U256::from(1))
        .await
        .unwrap();
    api.anvil_set_balance(
        address!("62b0dd4aab2b1a0a04e279e2b828791a10755528"),
        U256::from(1100000000000000000_u128),
    )
    .await
    .unwrap();
    api.anvil_set_balance(
        address!("1ed9b1dd266b607ee278726d324b855a093394a6"),
        U256::from(120000000000000000_u128),
    )
    .await
    .unwrap();

    verify_proof(&api, address!("2031f89b3ea8014eb51a78c316e42af3e0d7695f"), [
        "0xe48200a7a040f916999be583c572cc4dd369ec53b0a99f7de95f13880cf203d98f935ed1b3",
        "0xf87180a04fb9bab4bb88c062f32452b7c94c8f64d07b5851d44a39f1e32ba4b1829fdbfb8080808080a0b61eeb2eb82808b73c4ad14140a2836689f4ab8445d69dd40554eaf1fce34bc080808080808080a0dea230ff2026e65de419288183a340125b04b8405cc61627b3b4137e2260a1e880",
        "0xf8719f31355ec1c8f7e26bb3ccbcb0b75d870d15846c0b98e5cc452db46c37faea40b84ff84d80890270801d946c940000a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    ]).await;

    verify_proof(&api, address!("33f0fc440b8477fcfbe9d0bf8649e7dea9baedb2"), [
        "0xe48200a7a040f916999be583c572cc4dd369ec53b0a99f7de95f13880cf203d98f935ed1b3",
        "0xf87180a04fb9bab4bb88c062f32452b7c94c8f64d07b5851d44a39f1e32ba4b1829fdbfb8080808080a0b61eeb2eb82808b73c4ad14140a2836689f4ab8445d69dd40554eaf1fce34bc080808080808080a0dea230ff2026e65de419288183a340125b04b8405cc61627b3b4137e2260a1e880",
        "0xe48200d3a0ef957210bca5b9b402d614eb8408c88cfbf4913eb6ab83ca233c8b8f0e626b54",
        "0xf851808080a02743a5addaf4cf9b8c0c073e1eaa555deaaf8c41cb2b41958e88624fa45c2d908080808080a0bfbf6937911dfb88113fecdaa6bde822e4e99dae62489fcf61a91cb2f36793d680808080808080",
        "0xf8679e207781e762f3577784bab7491fcc43e291ce5a356b9bc517ac52eed3a37ab846f8448001a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
    ]).await;

    verify_proof(&api, address!("62b0dd4aab2b1a0a04e279e2b828791a10755528"), [
        "0xe48200a7a040f916999be583c572cc4dd369ec53b0a99f7de95f13880cf203d98f935ed1b3",
        "0xf87180a04fb9bab4bb88c062f32452b7c94c8f64d07b5851d44a39f1e32ba4b1829fdbfb8080808080a0b61eeb2eb82808b73c4ad14140a2836689f4ab8445d69dd40554eaf1fce34bc080808080808080a0dea230ff2026e65de419288183a340125b04b8405cc61627b3b4137e2260a1e880",
        "0xf8709f3936599f93b769acf90c7178fd2ddcac1b5b4bc9949ee5a04b7e0823c2446eb84ef84c80880f43fc2c04ee0000a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
    ]).await;

    verify_proof(&api, address!("1ed9b1dd266b607ee278726d324b855a093394a6"), [
        "0xe48200a7a040f916999be583c572cc4dd369ec53b0a99f7de95f13880cf203d98f935ed1b3",
        "0xf87180a04fb9bab4bb88c062f32452b7c94c8f64d07b5851d44a39f1e32ba4b1829fdbfb8080808080a0b61eeb2eb82808b73c4ad14140a2836689f4ab8445d69dd40554eaf1fce34bc080808080808080a0dea230ff2026e65de419288183a340125b04b8405cc61627b3b4137e2260a1e880",
        "0xe48200d3a0ef957210bca5b9b402d614eb8408c88cfbf4913eb6ab83ca233c8b8f0e626b54",
        "0xf851808080a02743a5addaf4cf9b8c0c073e1eaa555deaaf8c41cb2b41958e88624fa45c2d908080808080a0bfbf6937911dfb88113fecdaa6bde822e4e99dae62489fcf61a91cb2f36793d680808080808080",
        "0xf86f9e207a32b8ab5eb4b043c65b1f00c93f517bc8883c5cd31baf8e8a279475e3b84ef84c808801aa535d3d0c0000a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    ]).await;
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
