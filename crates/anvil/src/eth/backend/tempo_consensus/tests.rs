use super::*;
use alloy_primitives::{b256, hex};
use commonware_codec::{DecodeExt, ReadExt, varint::UInt};
use commonware_cryptography::bls12381::primitives::{
    group::{G1, G2},
    ops,
};

#[test]
fn zero_depth_unwind_preserves_pending_rotation() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(3).unwrap());
    let mut header = TempoHeader::default();
    header.inner.number = 1;
    consensus.prepare_header(&mut header, TempoHardfork::T4).unwrap();
    let first = consensus.finalize(&header, TempoHardfork::T4).unwrap();
    let pending = consensus.rotate().unwrap();
    let mut unchanged = consensus.clone();

    consensus.unwind(1).unwrap();
    assert_eq!(consensus.latest(), Some((1, &first)));
    assert_eq!(consensus.rotate().unwrap(), unchanged.rotate().unwrap());
    assert_ne!(consensus.identity(), pending);
}

#[test]
fn default_mnemonic_preserves_development_identity() {
    let seed =
        TempoConsensus::default_seed("test test test test test test test test test test test junk");
    assert_eq!(seed, b256!("4b9aa82cf6c29eddbd043574e458c69f0ec0a8ab81692bf3fe89f98c4b976cf7"));
    let identity = TempoConsensus::new(seed, NonZeroU64::new(21_600).unwrap()).identity();
    assert_eq!(identity, TempoConsensus::new(seed, NonZeroU64::new(21_600).unwrap()).identity());
    assert_eq!((identity.participants, identity.quorum), (1, 1));
    assert_eq!(
        identity.proposer,
        b256!("c7a2385306345b175d35a119733f90900d85eafa2fb05f305a1ecaa78b337005")
    );
    // Public-key vector captured from the original deterministic development signer.
    assert_eq!(
        identity.identity.as_ref(),
        &hex!(
            "a9fa69f935f2c004b632669e79cc7fb800aea27984f73e6df7d9d5c390b79c0f1f58aaa756e32ff352c891d52d4f0e5c03b9f2f3ce84bac04d066b667cd3780daadf507fbeef7d557ffd1f550a5190dec1a9a82adc3ca92e58b76c348d8bb2dc"
        )
    );
    assert_eq!((identity.from_epoch, identity.epoch_length), (0, 21_600));
    assert_ne!(
        identity.identity,
        TempoConsensus::new(B256::ZERO, NonZeroU64::new(21_600).unwrap()).identity().identity
    );
}

#[test]
fn finalized_boundary_authenticates_rotation_and_preserves_announced_key() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(3).unwrap());
    let initial = consensus.identity();
    finalize(&mut consensus, 1);
    let first = consensus.rotate().unwrap();
    let (boundary, certificate) = finalize(&mut consensus, 2);
    assert_eq!(consensus.identity(), initial);
    assert_certificate(&initial, &boundary, &certificate);

    let outcome = OnchainDkgOutcome::decode(boundary.inner.extra_data.as_ref()).unwrap();
    assert_eq!(outcome.epoch, 1);
    assert_eq!(outcome.network_identity().encode().as_ref(), first.identity.as_ref());
    assert_eq!(outcome.output.public().required(), 1);
    assert_eq!(outcome.output.quorum::<N3f1>(), 1);
    assert_eq!(outcome.next_players, outcome.output.players().clone());
    assert_eq!(outcome.next_players.len(), 1);
    assert!(!outcome.is_next_full_dkg);

    let superseded = consensus.rotate().unwrap();
    let second = consensus.rotate().unwrap();
    assert_eq!(second.from_epoch, 2);
    assert_ne!(superseded.identity, second.identity);
    let (header, certificate) = finalize(&mut consensus, 3);
    assert_eq!(consensus.identity(), first);
    assert_certificate(&first, &header, &certificate);
    assert!(!certificate_verifies(&initial, &certificate));
    assert!(!certificate_verifies(&second, &certificate));

    finalize(&mut consensus, 4);
    let (boundary, certificate) = finalize(&mut consensus, 5);
    assert_certificate(&first, &boundary, &certificate);
    let outcome = OnchainDkgOutcome::decode(boundary.inner.extra_data.as_ref()).unwrap();
    assert_eq!(outcome.epoch, 2);
    assert_eq!(outcome.network_identity().encode().as_ref(), second.identity.as_ref());
    let (header, certificate) = finalize(&mut consensus, 6);
    assert_eq!(consensus.identity(), second);
    assert_certificate(&second, &header, &certificate);
    assert!(!certificate_verifies(&first, &certificate));
    assert!(!certificate_verifies(&superseded, &certificate));
}

#[test]
fn single_block_epochs_announce_before_switching_identity() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::MIN);
    let initial = consensus.identity();
    let next = consensus.rotate().unwrap();
    assert_eq!(next.from_epoch, 2);
    let (boundary, certificate) = finalize(&mut consensus, 1);
    assert_eq!((certificate.epoch, certificate.view), (1, 1));
    assert_eq!(boundary.consensus_context.unwrap().parent_view, 0);
    assert_certificate(&initial, &boundary, &certificate);
    let outcome = OnchainDkgOutcome::decode(boundary.inner.extra_data.as_ref()).unwrap();
    assert_eq!(outcome.epoch, 2);
    assert_eq!(outcome.network_identity().encode().as_ref(), next.identity.as_ref());

    let later = consensus.rotate().unwrap();
    assert_eq!(later.from_epoch, 3);
    let (boundary, certificate) = finalize(&mut consensus, 2);
    assert_eq!(consensus.identity(), next);
    assert_certificate(&next, &boundary, &certificate);
    let outcome = OnchainDkgOutcome::decode(boundary.inner.extra_data.as_ref()).unwrap();
    assert_eq!(outcome.epoch, 3);
    assert_eq!(outcome.network_identity().encode().as_ref(), later.identity.as_ref());
}

#[test]
fn nonzero_checkpoint_stays_unsigned_and_bounds_rewinds() {
    let mut consensus = TempoConsensus::at_height(B256::ZERO, NonZeroU64::new(3).unwrap(), 8);
    let initial = consensus.identity();
    assert_eq!(initial.from_epoch, 2);
    assert!(consensus.latest().is_none());
    assert!(consensus.finalization(8).is_none());
    for height in [0, 7, 8, 10] {
        let mut header = TempoHeader::default();
        header.inner.number = height;
        let unchanged = header.clone();
        let error = TempoConsensusError::UnexpectedHeight { expected: 9, actual: height };
        assert_eq!(consensus.prepare_header(&mut header, TempoHardfork::T4), Err(error));
        assert_eq!(header, unchanged);
        assert_eq!(consensus.finalize(&header, TempoHardfork::T4), Err(error));
    }
    let rotated = consensus.rotate().unwrap();
    assert_eq!(rotated.from_epoch, 4);
    let (header, certificate) = finalize(&mut consensus, 9);
    assert_certificate(&initial, &header, &certificate);
    for requested in [7, 10] {
        assert_eq!(
            consensus.unwind(requested),
            Err(TempoConsensusError::InvalidRewind { anchor: 8, head: 9, requested })
        );
        assert_eq!(consensus.latest(), Some((9, &certificate)));
        assert_eq!(consensus.identity(), initial);
    }
    consensus.reset();
    assert!(consensus.latest().is_none());
    assert!(consensus.finalization(9).is_none());
    assert_eq!(consensus.identity(), initial);
    assert_eq!(consensus.rotate().unwrap(), rotated);
    finalize(&mut consensus, 9);
    consensus.unwind(8).unwrap();
    assert!(consensus.latest().is_none());
    assert_eq!(consensus.identity(), initial);
}

#[test]
fn cloned_checkpoint_restores_pending_rotation_and_counter() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(2).unwrap());
    let initial = consensus.identity();
    let first = consensus.rotate().unwrap();
    let checkpoint = consensus.clone();
    let (_, boundary) = finalize(&mut consensus, 1);
    let (_, activated) = finalize(&mut consensus, 2);
    let second = consensus.rotate().unwrap();

    consensus = checkpoint;
    assert_eq!(consensus.identity(), initial);
    assert!(consensus.latest().is_none());
    assert_eq!(finalize(&mut consensus, 1).1, boundary);
    assert_eq!(finalize(&mut consensus, 2).1, activated);
    assert_eq!(consensus.identity(), first);
    assert_eq!(consensus.rotate().unwrap(), second);
}

#[test]
fn unwind_restores_retained_key_schedule_and_discards_later_certificates() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(3).unwrap());
    let initial = consensus.identity();
    let (_, earlier) = finalize(&mut consensus, 1);
    let first = consensus.rotate().unwrap();
    let (_, retained) = finalize(&mut consensus, 2);
    let (old_header, old_certificate) = finalize(&mut consensus, 3);
    let later = consensus.rotate().unwrap();

    consensus.unwind(2).unwrap();
    assert_eq!(consensus.identity(), initial);
    assert_eq!(consensus.latest(), Some((2, &retained)));
    assert_eq!(consensus.finalization(1), Some(&earlier));
    assert!(consensus.finalization(3).is_none());
    assert_eq!(consensus.rotate().unwrap(), later);
    let mut replacement = old_header.clone();
    replacement.inner.timestamp += 1;
    consensus.prepare_header(&mut replacement, TempoHardfork::T4).unwrap();
    let new_certificate = consensus.finalize(&replacement, TempoHardfork::T4).unwrap();
    assert_ne!(new_certificate.digest, old_certificate.digest);
    assert_certificate(&first, &old_header, &old_certificate);
    assert_certificate(&first, &replacement, &new_certificate);
    assert_eq!(consensus.finalization(3), Some(&new_certificate));
}

#[test]
fn invalid_context_does_not_advance_consensus() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(3).unwrap());
    let initial = consensus.identity();
    let mut header = TempoHeader::default();
    header.inner.number = 1;
    consensus.prepare_header(&mut header, TempoHardfork::T4).unwrap();
    let context = header.consensus_context.unwrap();
    let mut alternatives = vec![None];
    for field in 0..4 {
        let mut modified = context;
        match field {
            0 => modified.epoch += 1,
            1 => modified.view += 1,
            2 => modified.parent_view += 1,
            _ => {
                modified.proposer = TempoConsensus::new(B256::repeat_byte(1), NonZeroU64::MIN)
                    .identity()
                    .proposer
                    .try_into()
                    .unwrap();
            }
        }
        alternatives.push(Some(modified));
    }
    for context in alternatives {
        let mut invalid = header.clone();
        invalid.consensus_context = context;
        assert_eq!(
            consensus.finalize(&invalid, TempoHardfork::T4),
            Err(TempoConsensusError::InvalidContext)
        );
        assert!(consensus.latest().is_none());
        assert_eq!(consensus.identity(), initial);
    }
    let certificate = consensus.finalize(&header, TempoHardfork::T4).unwrap();
    assert_certificate(&initial, &header, &certificate);
}

#[test]
fn stale_or_invalid_boundary_data_does_not_advance_consensus() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(2).unwrap());
    let initial = consensus.identity();
    let mut header = TempoHeader::default();
    header.inner.number = 1;
    consensus.prepare_header(&mut header, TempoHardfork::T4).unwrap();
    let rotated = consensus.rotate().unwrap();
    assert_eq!(
        consensus.finalize(&header, TempoHardfork::T4),
        Err(TempoConsensusError::InvalidExtraData)
    );

    consensus.prepare_header(&mut header, TempoHardfork::T4).unwrap();
    let outcome = OnchainDkgOutcome::decode(header.inner.extra_data.as_ref()).unwrap();
    let mut wrong_epoch = outcome.clone();
    wrong_epoch.epoch += 1;
    let mut wrong_flag = outcome;
    wrong_flag.is_next_full_dkg = true;
    let mut trailing = header.inner.extra_data.to_vec();
    trailing.push(0);
    for extra_data in
        [Bytes::new(), wrong_epoch.encode().into(), wrong_flag.encode().into(), trailing.into()]
    {
        let mut invalid = header.clone();
        invalid.inner.extra_data = extra_data;
        assert_eq!(
            consensus.finalize(&invalid, TempoHardfork::T4),
            Err(TempoConsensusError::InvalidExtraData)
        );
        assert_eq!(consensus.identity(), initial);
        assert!(consensus.latest().is_none());
        assert!(consensus.finalization(1).is_none());
    }
    let certificate = consensus.finalize(&header, TempoHardfork::T4).unwrap();
    assert_certificate(&initial, &header, &certificate);
    let (header, certificate) = finalize(&mut consensus, 2);
    assert_eq!(consensus.identity(), rotated);
    assert_certificate(&rotated, &header, &certificate);
}

#[test]
fn genesis_and_hardfork_context_preserve_execution_header_fields() {
    for &hardfork in TempoHardfork::VARIANTS {
        let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(3).unwrap());
        let mut genesis = TempoHeader::default();
        consensus.prepare_header(&mut genesis, hardfork).unwrap();
        assert!(genesis.consensus_context.is_none());
        assert_eq!(
            consensus.finalize(&genesis, hardfork),
            Err(TempoConsensusError::UnexpectedHeight { expected: 1, actual: 0 })
        );
        let mut header = TempoHeader::default();
        header.inner.number = 1;
        header.inner.timestamp = 42;
        header.inner.state_root = B256::repeat_byte(7);
        header.inner.parent_hash = B256::repeat_byte(8);
        header.inner.gas_limit = 30_000_000;
        header.general_gas_limit = 20_000_000;
        header.shared_gas_limit = 10_000_000;
        header.timestamp_millis_part = 123;
        let mut expected = header.clone();
        header.inner.extra_data = Bytes::from_static(b"discarded dealer log");
        consensus.prepare_header(&mut header, hardfork).unwrap();
        assert_eq!(header.consensus_context.is_some(), hardfork >= TempoHardfork::T4);
        expected.consensus_context = header.consensus_context;
        assert_eq!(header, expected);
        let certificate = consensus.finalize(&header, hardfork).unwrap();
        assert_certificate(&consensus.identity(), &header, &certificate);
    }
}

#[test]
fn rejected_heights_and_overflows_leave_state_unchanged() {
    let mut consensus = TempoConsensus::new(B256::ZERO, NonZeroU64::new(3).unwrap());
    let (_, first) = finalize(&mut consensus, 1);
    for height in [0, 1, 3, u64::MAX] {
        let mut header = TempoHeader::default();
        header.inner.number = height;
        let before = header.clone();
        let error = TempoConsensusError::UnexpectedHeight { expected: 2, actual: height };
        assert_eq!(consensus.prepare_header(&mut header, TempoHardfork::T4), Err(error));
        assert_eq!(header, before);
        assert_eq!(consensus.finalize(&header, TempoHardfork::T4), Err(error));
        assert_eq!(consensus.latest(), Some((1, &first)));
    }
    finalize(&mut consensus, 2);

    for (anchor, error) in [
        (u64::MAX - 1, TempoConsensusError::EpochOverflow),
        (u64::MAX, TempoConsensusError::HeightOverflow),
    ] {
        let mut terminal = TempoConsensus::at_height(B256::ZERO, NonZeroU64::MIN, anchor);
        let identity = terminal.identity();
        let mut header = TempoHeader::default();
        header.inner.number = u64::MAX;
        let before = header.clone();
        assert_eq!(terminal.rotate(), Err(error));
        assert_eq!(terminal.prepare_header(&mut header, TempoHardfork::T4), Err(error));
        assert_eq!(header, before);
        assert_eq!(terminal.finalize(&header, TempoHardfork::T4), Err(error));
        assert_eq!(terminal.identity(), identity);
        assert!(terminal.latest().is_none());
    }

    consensus.state.counter = u64::MAX;
    let identity = consensus.identity();
    assert_eq!(consensus.rotate(), Err(TempoConsensusError::CounterOverflow));
    assert_eq!(consensus.identity(), identity);
    assert_eq!(consensus.latest().unwrap().0, 2);

    let mut largest_view =
        TempoConsensus::at_height(B256::ZERO, NonZeroU64::new(u64::MAX).unwrap(), u64::MAX - 2);
    let identity = largest_view.identity();
    let (header, certificate) = finalize(&mut largest_view, u64::MAX - 1);
    assert_eq!((certificate.epoch, certificate.view), (0, u64::MAX));
    assert_certificate(&identity, &header, &certificate);
    let outcome = OnchainDkgOutcome::decode(header.inner.extra_data.as_ref()).unwrap();
    assert_eq!(outcome.epoch, 1);
}

fn finalize(consensus: &mut TempoConsensus, height: u64) -> (TempoHeader, TempoFinalization) {
    let mut header = TempoHeader::default();
    header.inner.number = height;
    consensus.prepare_header(&mut header, TempoHardfork::T4).unwrap();
    let certificate = consensus.finalize(&header, TempoHardfork::T4).unwrap();
    (header, certificate)
}

fn assert_certificate(
    identity: &TempoConsensusIdentity,
    header: &TempoHeader,
    finalization: &TempoFinalization,
) {
    let bytes = finalization.certificate.as_ref();
    let mut input = bytes;
    let epoch = u64::from(UInt::<u64>::read(&mut input).unwrap());
    let view = u64::from(UInt::<u64>::read(&mut input).unwrap());
    let parent_view = u64::from(UInt::<u64>::read(&mut input).unwrap());
    assert_eq!((epoch, view), (finalization.epoch, finalization.view));
    assert_eq!(B256::from_slice(&input[..32]), finalization.digest);
    assert_eq!(finalization.digest, keccak256(alloy_rlp::encode(header)));
    assert_ne!(finalization.digest, header.inner.hash_slow());
    if let Some(context) = header.consensus_context {
        assert_eq!((context.epoch, context.view, context.parent_view), (epoch, view, parent_view));
        assert_eq!(B256::from(context.proposer), identity.proposer);
    }
    assert!(certificate_verifies(identity, finalization));
}

fn certificate_verifies(
    identity: &TempoConsensusIdentity,
    finalization: &TempoFinalization,
) -> bool {
    let bytes = finalization.certificate.as_ref();
    let mut input = bytes;
    UInt::<u64>::read(&mut input).unwrap();
    UInt::<u64>::read(&mut input).unwrap();
    let round_len = bytes.len() - input.len();
    UInt::<u64>::read(&mut input).unwrap();
    assert_eq!(input.len(), 32 + 48 + 48);
    let proposal_len = bytes.len() - 96;
    let vote = G1::decode(&input[32..80]).unwrap();
    let seed = G1::decode(&input[80..]).unwrap();
    let public = G2::decode(identity.identity.as_ref()).unwrap();
    ops::verify_message::<MinSig>(&public, b"TEMPO_FINALIZE", &bytes[..proposal_len], &vote).is_ok()
        && ops::verify_message::<MinSig>(&public, b"TEMPO_SEED", &bytes[..round_len], &seed).is_ok()
}
