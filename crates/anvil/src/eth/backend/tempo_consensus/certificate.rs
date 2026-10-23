//! The private wire adapter for Tempo's Simplex finalization certificates.
//!
//! The pinned format is `UInt(epoch) || UInt(view) || UInt(parent) || digest || vote || seed`,
//! with two compressed 48-byte MinSig signatures. The vote signs the whole proposal, while the
//! seed signs only its epoch and view. Integers use Commonware's unsigned varints, not fixed-width
//! Ethereum quantities. Keep this adapter aligned with the pinned upstream definitions:
//!
//! - [Commonware 2026.9.0 `Proposal` and `Finalization`][proposal].
//! - [Commonware 2026.9.0 threshold VRF signature pair][vrf] and [namespaces][namespaces].
//! - [Tempo's pinned `TEMPO` namespace][tempo].
//!
//! [proposal]: https://github.com/commonwarexyz/monorepo/blob/d476a2361ce6840d2b9d0aa6fb30a924429046d4/consensus/src/simplex/types.rs
//! [vrf]: https://github.com/commonwarexyz/monorepo/blob/d476a2361ce6840d2b9d0aa6fb30a924429046d4/consensus/src/simplex/scheme/bls12381_threshold/vrf.rs
//! [namespaces]: https://github.com/commonwarexyz/monorepo/blob/d476a2361ce6840d2b9d0aa6fb30a924429046d4/consensus/src/simplex/scheme/mod.rs
//! [tempo]: https://github.com/tempoxyz/tempo/blob/2c25f7cd8267b205a6b95e08bbc73baf887bea88/crates/consensus/src/config.rs

use alloy_primitives::{B256, Bytes};
use commonware_codec::{
    FixedSize, Write,
    varint::{MAX_U64_VARINT_SIZE, UInt},
};
use commonware_cryptography::{
    bls12381::{
        dkg::feldman_desmedt::Output,
        primitives::{
            group::{G1, Share},
            ops::threshold,
            variant::MinSig,
        },
    },
    ed25519::PublicKey,
};
use commonware_parallel::Sequential;

// Three UInt<u64> varints, a digest, and two compressed MinSig signatures.
const MAX_CERTIFICATE_BYTES: usize = 3 * MAX_U64_VARINT_SIZE + B256::len_bytes() + 2 * G1::SIZE;

/// Signs a proposal with the local 1-of-1 committee and encodes its finalization certificate.
pub(super) fn encode(
    epoch: u64,
    view: u64,
    parent_view: u64,
    digest: B256,
    output: &Output<MinSig, PublicKey>,
    share: &Share,
) -> Bytes {
    let mut certificate = Vec::with_capacity(MAX_CERTIFICATE_BYTES);
    UInt(epoch).write(&mut certificate);
    UInt(view).write(&mut certificate);
    let seed = threshold::sign_message::<MinSig>(share, b"TEMPO_SEED", &certificate);

    UInt(parent_view).write(&mut certificate);
    certificate.extend_from_slice(digest.as_slice());
    let vote = threshold::sign_message::<MinSig>(share, b"TEMPO_FINALIZE", &certificate);
    let (vote, seed) = threshold::recover_pair(output.public(), [&vote], [&seed], &Sequential)
        .expect("the local committee has exactly one share and a threshold of one");
    vote.write(&mut certificate);
    seed.write(&mut certificate);
    certificate.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt;
    use commonware_cryptography::{
        Signer,
        bls12381::{
            dkg::feldman_desmedt,
            primitives::{ops, sharing::Mode},
        },
        ed25519::PrivateKey,
    };
    use commonware_utils::{N3f1, TryFromIterator, ordered::Set};
    use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};

    #[test]
    fn finalization_encodes_and_signs_variable_length_proposals() {
        let (output, share) = committee(7);
        // Literal protobuf varints independently specify the proposal's three integer fields.
        let cases = [
            ((7, 9, 8), 0x42, &[7, 9, 8][..], 2),
            ((127, 128, 127), 0, &[0x7f, 0x80, 0x01, 0x7f][..], 3),
            ((128, 129, 128), 0, &[0x80, 0x01, 0x81, 0x01, 0x80, 0x01][..], 4),
            ((16_383, 16_384, 16_383), 0, &[0xff, 0x7f, 0x80, 0x80, 0x01, 0xff, 0x7f][..], 5),
            (
                (16_384, 16_385, 16_384),
                0,
                &[0x80, 0x80, 0x01, 0x81, 0x80, 0x01, 0x80, 0x80, 0x01][..],
                6,
            ),
        ];
        for ((epoch, view, parent_view), digest_byte, prefix, round_len) in cases {
            let certificate =
                encode(epoch, view, parent_view, B256::repeat_byte(digest_byte), &output, &share);
            let proposal_len = prefix.len() + 32;
            assert_eq!(&certificate[..prefix.len()], prefix);
            assert_eq!(&certificate[prefix.len()..proposal_len], &[digest_byte; 32]);
            assert_eq!(certificate.len(), proposal_len + 96);

            let vote = G1::decode(&certificate[proposal_len..proposal_len + 48]).unwrap();
            let seed = G1::decode(&certificate[proposal_len + 48..]).unwrap();
            let public = output.public().public();
            assert!(
                ops::verify_message::<MinSig>(
                    public,
                    b"TEMPO_FINALIZE",
                    &certificate[..proposal_len],
                    &vote,
                )
                .is_ok()
            );
            assert!(
                ops::verify_message::<MinSig>(public, b"TEMPO_SEED", &prefix[..round_len], &seed)
                    .is_ok()
            );
        }
    }

    #[test]
    fn finalization_seed_depends_only_on_round() {
        let (output, share) = committee(7);
        let first = encode(7, 9, 8, B256::repeat_byte(0x42), &output, &share);
        let second = encode(7, 9, 6, B256::repeat_byte(0x24), &output, &share);
        assert_ne!(&first[35..83], &second[35..83]);
        assert_eq!(&first[83..], &second[83..]);
        assert_eq!(first, encode(7, 9, 8, B256::repeat_byte(0x42), &output, &share));
    }

    fn committee(seed: u8) -> (Output<MinSig, PublicKey>, Share) {
        let proposer = PrivateKey::decode(&[seed; 32][..]).unwrap().public_key();
        let (output, shares) = feldman_desmedt::deal::<MinSig, _, N3f1>(
            ChaCha20Rng::from_seed([seed; 32]),
            Mode::NonZeroCounter,
            Set::try_from_iter([proposer]).unwrap(),
        )
        .unwrap();
        (output, shares.into_iter().next().unwrap().1)
    }
}
