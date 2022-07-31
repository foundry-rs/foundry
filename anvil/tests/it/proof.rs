//! tests for `eth_getProof`

use anvil::{spawn, NodeConfig};
use ethers::types::Address;
use forge::revm::KECCAK_EMPTY;

#[tokio::test(flavor = "multi_thread")]
async fn can_get_proof() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let acc: Address = "0xaaaf5374fce5edbc8e2a8697c15331677e6ebaaa".parse().unwrap();
    let proof = api.get_proof(acc, Vec::new(), None).await.unwrap();

    assert_eq!(proof.code_hash, KECCAK_EMPTY);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_random_account_proofs() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    for acc in std::iter::repeat_with(Address::random).take(10) {
        let _ = api
            .get_proof(acc, Vec::new(), None)
            .await
            .unwrap_or_else(|_| panic!("Failed to get proof for {:?}", acc));
    }
}
