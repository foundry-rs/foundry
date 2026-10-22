//! Independent native Commonware verification of Tempo consensus core artifacts.

use alloy_primitives::{B256, Bytes, b256, hex, keccak256};
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
use commonware_utils::N3f1;
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use serde::Deserialize;
use std::io::{Read, Write};
use tempo_dkg_onchain_artifacts::OnchainDkgOutcome;
use tempo_primitives::TempoHeader;

type SigningScheme = Scheme<PublicKey, MinSig>;

// Digest is only Commonware's fixed 32-byte wire container. Header hashing remains Keccak-256.
type Certificate = Finalization<SigningScheme, Digest>;

type Identity = <MinSig as Variant>::Public;

// Trust the fixed default-mnemonic development identity, never a per-header supplied key.
const INITIAL_IDENTITY: [u8; 96] = hex!(
    "a9fa69f935f2c004b632669e79cc7fb800aea27984f73e6df7d9d5c390b79c0f1f58aaa756e32ff352c891d52d4f0e5c03b9f2f3ce84bac04d066b667cd3780daadf507fbeef7d557ffd1f550a5190dec1a9a82adc3ca92e58b76c348d8bb2dc"
);

const INITIAL_PROPOSER: B256 =
    b256!("c7a2385306345b175d35a119733f90900d85eafa2fb05f305a1ecaa78b337005");

#[derive(Deserialize)]
struct Input {
    identity: Bytes,
    rotation: Vec<CertifiedHeader>,
    rounds: Vec<CertifiedHeader>,
}

#[derive(Deserialize)]
struct CertifiedHeader {
    header: TempoHeader,
    epoch: u64,
    view: u64,
    digest: B256,
    certificate: Bytes,
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
    assert_eq!(input.identity.as_ref(), INITIAL_IDENTITY);
    let identity = Identity::decode(input.identity.as_ref())?;
    let mut rng = ChaCha20Rng::from_seed([0; 32]);
    verify_rotation(&input.rotation, identity, &mut rng);
    verify_rounds(&input.rounds, &identity, &mut rng);
    writeln!(
        std::io::stdout().lock(),
        "verified 6 rotation headers, 2 DKG transitions and 9 round cases with native Commonware 2026.9.0; malformed and tampered certificates rejected",
    )?;
    Ok(())
}

fn verify_rotation(headers: &[CertifiedHeader], mut identity: Identity, rng: &mut ChaCha20Rng) {
    assert_eq!(headers.len(), 6);
    let mut proposer = INITIAL_PROPOSER;
    let mut pending = None;
    let mut previous = None;
    for (index, header) in headers.iter().enumerate() {
        let height = index as u64 + 1;
        assert_eq!(header.header.inner.number, height);
        assert_eq!((header.epoch, header.view), (height / 3, height % 3 + 1));
        if height.is_multiple_of(3) {
            previous = Some(identity);
            (identity, proposer) = pending.take().expect("the prior boundary announced this epoch");
            assert_ne!(Some(identity), previous);
        }
        let certificate = verify_header(header, &identity, rng);
        assert_eq!(B256::from(header.header.consensus_context.unwrap().proposer), proposer);
        if let Some(previous) = previous {
            let verifier = SigningScheme::certificate_verifier(b"TEMPO", previous);
            assert!(!certificate.verify(rng, &verifier, &Sequential));
        }
        if height % 3 == 2 {
            // Only a boundary already authenticated by the current key can introduce the next key.
            let bytes = header.header.inner.extra_data.as_ref();
            let outcome = OnchainDkgOutcome::decode(bytes).unwrap();
            assert_eq!(outcome.encode().as_ref(), bytes);
            assert_eq!(outcome.epoch, header.epoch + 1);
            assert_eq!(outcome.output.quorum::<N3f1>(), 1);
            assert_eq!(outcome.next_players, *outcome.output.players());
            assert!(!outcome.is_next_full_dkg);
            let next_identity = *outcome.network_identity();
            assert_ne!(next_identity, identity);
            let next_proposer = outcome.next_players.iter().next().unwrap().encode();
            pending = Some((next_identity, B256::from_slice(&next_proposer)));
        } else {
            assert!(header.header.inner.extra_data.is_empty());
        }
    }
}

fn verify_rounds(headers: &[CertifiedHeader], identity: &Identity, rng: &mut ChaCha20Rng) {
    let rounds = [
        (0, 127),
        (0, 128),
        (0, 16_383),
        (0, 16_384),
        (127, 128),
        (128, 129),
        (16_383, 16_384),
        (16_384, 16_385),
        (0, 2),
    ];
    assert_eq!(headers.len(), rounds.len());
    for (index, (header, round)) in headers.iter().zip(rounds).enumerate() {
        assert_eq!((header.epoch, header.view), round);
        assert_eq!(header.header.inner.number, round.0 * 32_768 + round.1 - 1);
        if index == rounds.len() - 1 {
            assert!(header.header.consensus_context.is_none());
        } else {
            assert_eq!(
                B256::from(header.header.consensus_context.unwrap().proposer),
                INITIAL_PROPOSER,
            );
        }
        assert!(header.header.inner.extra_data.is_empty());
        verify_header(header, identity, rng);
    }
}

fn verify_header(
    header: &CertifiedHeader,
    identity: &Identity,
    rng: &mut ChaCha20Rng,
) -> Certificate {
    let certificate = Certificate::decode(header.certificate.as_ref()).unwrap();
    assert_eq!(certificate.encode().as_ref(), header.certificate.as_ref());
    assert_eq!(certificate.proposal.round.epoch().get(), header.epoch);
    assert_eq!(certificate.proposal.round.view().get(), header.view);
    assert_eq!(certificate.proposal.parent.get(), header.view - 1);
    assert_eq!(certificate.proposal.payload.0, header.digest.0);
    assert_eq!(keccak256(alloy_rlp::encode(&header.header)), header.digest);
    assert_ne!(keccak256(alloy_rlp::encode(&header.header.inner)), header.digest);
    if let Some(context) = header.header.consensus_context {
        assert_eq!(context.epoch, header.epoch);
        assert_eq!(context.view, header.view);
        assert_eq!(context.parent_view, certificate.proposal.parent.get());
    }
    let verifier = SigningScheme::certificate_verifier(b"TEMPO", *identity);
    assert!(certificate.verify(rng, &verifier, &Sequential));
    verify_rejections(&certificate, &header.certificate, identity, rng);
    certificate
}

fn verify_rejections(
    certificate: &Certificate,
    bytes: &[u8],
    identity: &Identity,
    rng: &mut ChaCha20Rng,
) {
    let verifier = SigningScheme::certificate_verifier(b"TEMPO", *identity);
    let wrong_key = ops::compute_public::<MinSig>(&Private::random(&mut *rng));
    let wrong_verifier = SigningScheme::certificate_verifier(b"TEMPO", wrong_key);
    assert!(!certificate.verify(rng, &wrong_verifier, &Sequential));
    let wrong_namespace = SigningScheme::certificate_verifier(b"ANVIL", *identity);
    assert!(!certificate.verify(rng, &wrong_namespace, &Sequential));

    let mut tampered = certificate.clone();
    tampered.proposal.payload.0[0] ^= 1;
    assert!(!tampered.verify(rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    tampered.proposal.parent = View::new(certificate.proposal.parent.get() ^ 1);
    assert!(!tampered.verify(rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    tampered.proposal.round = Round::new(
        Epoch::new(certificate.proposal.round.epoch().get() ^ 1),
        certificate.proposal.round.view(),
    );
    assert!(!tampered.verify(rng, &verifier, &Sequential));
    let mut tampered = certificate.clone();
    tampered.proposal.round = Round::new(
        certificate.proposal.round.epoch(),
        View::new(certificate.proposal.round.view().get() ^ 1),
    );
    assert!(!tampered.verify(rng, &verifier, &Sequential));

    // Commonware decodes the two signatures lazily; malformed bytes can fail at verification.
    for offset in [bytes.len() - 96, bytes.len() - 48] {
        let mut tampered = bytes.to_vec();
        tampered[offset..offset + 48].fill(0);
        assert!(
            !Certificate::decode(tampered.as_slice()).is_ok_and(|certificate| certificate.verify(
                rng,
                &verifier,
                &Sequential
            ))
        );
    }
    assert!(Certificate::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(Certificate::decode(trailing.as_slice()).is_err());
}
