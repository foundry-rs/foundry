//! Independent native Commonware decoding and verification of Anvil's Tempo RPC output.

use alloy_consensus::{
    Sealable, Typed2718, proofs::calculate_transaction_root, transaction::TxHashRef,
};
use alloy_primitives::{B256, Bytes, hex};
use alloy_provider::Provider;
use anvil::{NodeConfig, NodeHandle, spawn};
use commonware_codec::{DecodeExt, Encode};
use commonware_consensus::{
    simplex::{scheme::bls12381_threshold::vrf::Scheme, types::Finalization},
    types::{Epoch, Round, View},
};
use commonware_cryptography::{
    bls12381::primitives::{
        group::Private,
        ops,
        variant::{MinSig, Variant},
    },
    ed25519::PublicKey,
    sha256::Digest,
};
use commonware_math::algebra::Random;
use commonware_parallel::Sequential;
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use serde::Deserialize;
use std::num::NonZeroU64;
use tempo_hardfork::TempoHardfork;

#[derive(Debug, Deserialize)]
struct Identity {
    identity: Bytes,
}

#[derive(Debug, Deserialize)]
struct CertifiedBlock {
    epoch: u64,
    view: u64,
    digest: B256,
    certificate: String,
    block: tempo_primitives::Block,
}

#[tokio::test(flavor = "multi_thread")]
async fn native_empty_block() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    api.mine_one().await.unwrap();
    let finalization = verify_block(&handle, 1).await;
    assert_eq!((finalization.epoch, finalization.view), (0, 1));
    assert!(finalization.block.body.transactions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn native_nonempty_block() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let accounts = handle.dev_accounts().collect::<Vec<_>>();
    let ethereum_hash = provider
        .client()
        .request::<_, B256>(
            "eth_sendTransaction",
            (serde_json::json!({
                "from": accounts[0],
                "to": accounts[2],
                "type": "0x2",
                "gas": "0xf4240",
            }),),
        )
        .await
        .unwrap();
    let tempo_hash = provider
        .client()
        .request::<_, B256>(
            "eth_sendTransaction",
            (serde_json::json!({
                "from": accounts[1],
                "type": "0x76",
                "gas": "0x1e8480",
                "calls": [{ "to": accounts[2], "value": "0x0", "input": "0x1234" }],
            }),),
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let finalization = verify_block(&handle, 1).await;
    let mut transactions = finalization
        .block
        .body
        .transactions
        .iter()
        .map(|tx| (tx.ty(), *tx.tx_hash()))
        .collect::<Vec<_>>();
    transactions.sort_unstable();
    assert_eq!(transactions, [(2, ethereum_hash), (0x76, tempo_hash)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn native_epoch_boundaries() {
    for hardfork in [TempoHardfork::T3, TempoHardfork::T4] {
        let (api, handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_hardfork(Some(hardfork.into()))
                .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
        )
        .await;
        for (height, round) in [(1, (0, 1)), (2, (0, 2)), (3, (1, 1))] {
            api.mine_one().await.unwrap();
            let finalization = verify_block(&handle, height).await;
            assert_eq!((finalization.epoch, finalization.view), round);
            assert_eq!(finalization.block.header.consensus_context.is_some(), hardfork.is_t4());
            assert_eq!(finalization.block.header.inner.extra_data.is_empty(), height != 2);
        }
    }
}

async fn verify_block(handle: &NodeHandle, height: u64) -> CertifiedBlock {
    let provider = handle.http_provider();
    let identity =
        provider.client().request::<_, Identity>("anvil_tempoConsensusIdentity", ()).await.unwrap();
    let finalization = provider
        .client()
        .request::<_, CertifiedBlock>(
            "consensus_getFinalization",
            (serde_json::json!({ "height": height }),),
        )
        .await
        .unwrap();
    assert_eq!(finalization.block.header.inner.number, height);
    let identity = <MinSig as Variant>::Public::decode(identity.identity.as_ref()).unwrap();
    let verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", identity);
    let bytes = hex::decode(&finalization.certificate).unwrap();
    let certificate =
        Finalization::<Scheme<PublicKey, MinSig>, Digest>::decode(bytes.as_slice()).unwrap();
    assert_eq!(certificate.proposal.round.epoch().get(), finalization.epoch);
    assert_eq!(certificate.proposal.round.view().get(), finalization.view);
    assert_eq!(certificate.proposal.payload.0, finalization.digest.0);
    assert_eq!(finalization.block.header.hash_slow(), finalization.digest);
    assert_eq!(
        calculate_transaction_root(&finalization.block.body.transactions),
        finalization.block.header.inner.transactions_root,
    );
    assert_eq!(
        finalization.block.body.calculate_ommers_root(),
        finalization.block.header.inner.ommers_hash,
    );
    assert_eq!(
        finalization.block.body.calculate_withdrawals_root(),
        finalization.block.header.inner.withdrawals_root,
    );
    assert_eq!(certificate.encode().as_ref(), bytes.as_slice());
    if let Some(context) = finalization.block.header.consensus_context {
        assert_eq!(context.epoch, finalization.epoch);
        assert_eq!(context.view, finalization.view);
        assert_eq!(context.parent_view, certificate.proposal.parent.get());
    }
    let mut rng = ChaCha20Rng::from_seed([0; 32]);
    assert!(certificate.verify(&mut rng, &verifier, &Sequential), "valid certificate");
    let wrong_key = ops::compute_public::<MinSig>(&Private::random(&mut rng));
    let wrong_verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", wrong_key);
    assert!(!certificate.verify(&mut rng, &wrong_verifier, &Sequential), "wrong identity");

    let mut tampered = certificate.clone();
    tampered.proposal.payload.0[0] ^= 1;
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed digest");
    let mut tampered = certificate.clone();
    tampered.proposal.parent = View::new(certificate.proposal.parent.get() ^ 1);
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed parent view");
    let mut tampered = certificate.clone();
    tampered.proposal.round =
        Round::new(Epoch::new(finalization.epoch ^ 1), certificate.proposal.round.view());
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed epoch");
    let mut tampered = certificate.clone();
    tampered.proposal.round =
        Round::new(certificate.proposal.round.epoch(), View::new(finalization.view + 1));
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed view");

    let mut tampered = certificate.clone();
    let mut signature = certificate.certificate.get().unwrap().clone();
    signature.vote_signature = signature.seed_signature;
    tampered.certificate = signature.into();
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed vote signature");
    let mut tampered = certificate.clone();
    let mut signature = certificate.certificate.get().unwrap().clone();
    signature.seed_signature = signature.vote_signature;
    tampered.certificate = signature.into();
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed seed signature");
    finalization
}
