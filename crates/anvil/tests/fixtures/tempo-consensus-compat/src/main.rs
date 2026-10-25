//! Independent native Commonware decoding and verification of Anvil's Tempo RPC output.

use alloy_consensus::{Sealable, proofs::calculate_transaction_root};
use alloy_primitives::{B256, Bytes, hex};
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
use std::io::{Read, Write};

#[derive(Deserialize)]
struct Input {
    identity: Identity,
    finalization: CertifiedBlock,
}

#[derive(Deserialize)]
struct Identity {
    identity: Bytes,
}

#[derive(Deserialize)]
struct CertifiedBlock {
    epoch: u64,
    view: u64,
    digest: B256,
    certificate: String,
    block: tempo_primitives::Block,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = if let Ok(path) = std::env::var("ANVIL_TEMPO_CONSENSUS_ARTIFACT") {
        std::fs::read_to_string(path)?
    } else {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        input
    };
    let input: Input = serde_json::from_str(&input)?;
    let identity = <MinSig as Variant>::Public::decode(input.identity.identity.as_ref())?;
    let verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", identity);
    let bytes = hex::decode(&input.finalization.certificate)?;
    let certificate = Finalization::<Scheme<PublicKey, MinSig>, Digest>::decode(bytes.as_slice())?;
    assert_eq!(certificate.proposal.round.epoch().get(), input.finalization.epoch);
    assert_eq!(certificate.proposal.round.view().get(), input.finalization.view);
    assert_eq!(certificate.proposal.payload.0, input.finalization.digest.0);
    assert_eq!(input.finalization.block.header.hash_slow(), input.finalization.digest);
    assert_eq!(
        calculate_transaction_root(&input.finalization.block.body.transactions),
        input.finalization.block.header.inner.transactions_root,
    );
    assert_eq!(
        input.finalization.block.body.calculate_ommers_root(),
        input.finalization.block.header.inner.ommers_hash,
    );
    assert_eq!(
        input.finalization.block.body.calculate_withdrawals_root(),
        input.finalization.block.header.inner.withdrawals_root,
    );
    assert_eq!(certificate.encode().as_ref(), bytes.as_slice());
    if let Some(context) = input.finalization.block.header.consensus_context {
        assert_eq!(context.epoch, input.finalization.epoch);
        assert_eq!(context.view, input.finalization.view);
        assert_eq!(context.parent_view, certificate.proposal.parent.get());
    }
    let mut rng = ChaCha20Rng::from_seed([0; 32]);
    assert!(certificate.verify(&mut rng, &verifier, &Sequential));
    let wrong_key = ops::compute_public::<MinSig>(&Private::random(&mut rng));
    let wrong_verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", wrong_key);
    assert!(!certificate.verify(&mut rng, &wrong_verifier, &Sequential));

    let mut tampered = certificate.clone();
    tampered.proposal.payload.0[0] ^= 1;
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    tampered.proposal.parent = View::new(certificate.proposal.parent.get() ^ 1);
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    tampered.proposal.round =
        Round::new(Epoch::new(input.finalization.epoch ^ 1), certificate.proposal.round.view());
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    tampered.proposal.round =
        Round::new(certificate.proposal.round.epoch(), View::new(input.finalization.view + 1));
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential));

    let mut tampered = certificate.clone();
    let mut signature = certificate.certificate.get().unwrap().clone();
    signature.vote_signature = signature.seed_signature;
    tampered.certificate = signature.into();
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    let mut signature = certificate.certificate.get().unwrap().clone();
    signature.seed_signature = signature.vote_signature;
    tampered.certificate = signature.into();
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential));

    writeln!(
        std::io::stdout().lock(),
        "verified height={} epoch={} view={} digest={} with native Commonware 2026.9.0; wrong identity, digest, parent view, epoch, view, vote signature and seed signature rejected",
        input.finalization.block.header.inner.number,
        input.finalization.epoch,
        input.finalization.view,
        input.finalization.digest,
    )?;
    Ok(())
}
