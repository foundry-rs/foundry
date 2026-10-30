//! Deterministic single-member Tempo development consensus.
//!
//! This is a local signer, not a running Simplex network. It derives an ed25519 proposer and a
//! real 1-of-1 BLS sharing from `ChaCha20(keccak256(seed || counter_be_u64))`. The default seed is
//! `keccak256("anvil/tempo-consensus/" || mnemonic)`. Keys are predictable development identities,
//! not production trust anchors. Each rotation derives a fresh sharing, rather than resharing an
//! existing key.
//!
//! For epoch length `L`, height `h` uses epoch `h / L`. Signed heights in epoch zero use view `h`,
//! while later epochs use view `h % L + 1`; the parent view is one less. Genesis and each preceding
//! epoch boundary serve as view-zero parents. This no-skips development convention does not
//! inherit a remote chain's rounds. A boundary at `h % L == L - 1` announces the next committee
//! in a native [`OnchainDkgOutcome`]. The boundary is signed by the old committee; the next epoch
//! activates the announced key. The outcome's `is_next_full_dkg = false` describes the next
//! ceremony, not whether this local key changed.
//!
//! [`TempoHeader`] owns the full RLP encoding and hash, including all execution fields and the
//! optional context. Genesis is unsigned and contextless. Context is omitted before T4, following
//! [TIP-1031's activation gate][tip1031]. The supported wire mapping is pinned to Tempo `2c25f7cd`
//! and Commonware `2026.9.0`, and rejects unknown hardforks pending review. This is not a claim of
//! upstream format stability; protocol-owner confirmation remains tracked by OSS-1186. Older
//! execution hardforks still use the pinned certificate/DKG codec, not historical Commonware
//! codecs.
//!
//! Cloning snapshots the complete controller, including pending rotations. Unwind instead restores
//! the retained height's checkpoint and discards later certificates and rotations. Checkpoints
//! retain only their active and future committees. Callers can bound retained history with
//! [`TempoConsensus::prune`]; unwind cannot cross the pruning boundary. Reset restores the unsigned
//! initial anchor, including its original rewind floor. Re-signing replacement history can produce
//! valid certificates for conflicting headers at the same height: this development rewind behavior
//! is not consensus safety. Callers own canonical commit, execution/body validation, archive
//! pruning, and persistence; this module has no mining, RPC, CLI, or fork-routing integration.
//!
//! [tip1031]: https://github.com/tempoxyz/tempo/blob/b34e701ecc69d91d72ffb3e77114958bb4f8bc38/crates/commonware-node/src/consensus/application/actor.rs

use alloy_consensus::Sealable;
use alloy_primitives::{B256, Bytes, keccak256};
use commonware_codec::Encode;
use commonware_cryptography::{
    Signer,
    bls12381::{
        dkg::feldman_desmedt::{self, Output},
        primitives::{group::Share, sharing::Mode, variant::MinSig},
    },
    ed25519::{PrivateKey, PublicKey},
};
use commonware_math::algebra::Random;
use commonware_utils::{N3f1, TryFromIterator, ordered};
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, num::NonZeroU64, sync::Arc};
use tempo_dkg_onchain_artifacts::OnchainDkgOutcome;
use tempo_hardfork::TempoHardfork;
use tempo_primitives::{TempoConsensusContext, TempoHeader};

mod certificate;

/// A deterministic development committee with one participant and quorum one.
#[derive(Clone)]
pub struct TempoConsensus {
    seed: B256,
    epoch_length: NonZeroU64,
    initial: Checkpoint,
    state: Checkpoint,
    checkpoints: BTreeMap<u64, Checkpoint>,
    finalizations: BTreeMap<u64, TempoFinalization>,
    retained_height: u64,
}

impl TempoConsensus {
    /// Creates the development committee at genesis.
    pub fn new(seed: B256, epoch_length: NonZeroU64) -> Self {
        Self::at_height(seed, epoch_length, 0)
    }

    /// Starts at an unsigned checkpoint, without certifying or importing its history.
    ///
    /// The local identity is independent of any upstream committee. Only strict successors of
    /// this height can be signed. Reset restores this anchor; unwind cannot cross it.
    pub fn at_height(seed: B256, epoch_length: NonZeroU64, height: u64) -> Self {
        let epoch = height / epoch_length;
        let initial = Checkpoint {
            height,
            counter: 0,
            committees: BTreeMap::from([(epoch, Arc::new(Committee::derive(seed, 0)))]),
        };
        Self {
            seed,
            epoch_length,
            initial: initial.clone(),
            state: initial,
            checkpoints: BTreeMap::new(),
            finalizations: BTreeMap::new(),
            retained_height: height,
        }
    }

    /// Derives the default seed from the account mnemonic with a consensus-specific domain.
    pub fn default_seed(mnemonic: &str) -> B256 {
        let mut input = b"anvil/tempo-consensus/".to_vec();
        input.extend_from_slice(mnemonic.as_bytes());
        keccak256(input)
    }

    /// Returns the public development identity.
    pub fn identity(&self) -> TempoConsensusIdentity {
        self.identity_at(self.state.height / self.epoch_length)
    }

    /// Installs metadata for the next header before its hash is calculated.
    ///
    /// Genesis stays contextless and unsigned. Only its `consensus_context` is cleared; all other
    /// fields, including caller-provided `extra_data`, are preserved.
    ///
    /// For other heights, this owns `consensus_context` and `extra_data`: non-boundaries have no
    /// dealer logs, while boundaries announce the native next-epoch outcome. Execution fields are
    /// preserved. Errors leave the header unchanged.
    pub fn prepare_header(
        &self,
        header: &mut TempoHeader,
        hardfork: TempoHardfork,
    ) -> Result<(), TempoConsensusError> {
        ensure_supported_hardfork(hardfork)?;
        if header.inner.number == 0 && self.state.height == 0 {
            header.consensus_context = None;
            return Ok(());
        }
        self.check_next_height(header.inner.number)?;
        let metadata = self.header_metadata(header.inner.number, hardfork.is_t4())?;
        header.consensus_context = metadata.consensus_context;
        header.inner.extra_data = metadata.extra_data;
        Ok(())
    }

    /// Certifies a prepared native header and checkpoints the resulting local state.
    ///
    /// Revalidates the metadata against the current key schedule, so rotating after preparation
    /// cannot sign a stale boundary announcement. The digest is computed from the complete native
    /// header. This does not validate execution, body commitments, or canonical chain membership;
    /// callers must commit the header and consensus state atomically with those checks.
    pub fn finalize(
        &mut self,
        header: &TempoHeader,
        hardfork: TempoHardfork,
    ) -> Result<TempoFinalization, TempoConsensusError> {
        self.check_next_height(header.inner.number)?;
        ensure_supported_hardfork(hardfork)?;
        let metadata = self.header_metadata(header.inner.number, hardfork.is_t4())?;
        if metadata.consensus_context != header.consensus_context {
            return Err(TempoConsensusError::InvalidContext);
        }
        if metadata.extra_data != header.inner.extra_data {
            return Err(TempoConsensusError::InvalidExtraData);
        }
        let height = header.inner.number;
        let (epoch, view, parent_view) = self.round(height);
        let digest = header.hash_slow();
        let (&from_epoch, committee) = self.committee_at(epoch);
        let certificate = certificate::encode(
            epoch,
            view,
            parent_view,
            digest,
            &committee.output,
            &committee.share,
        );
        let finalization = TempoFinalization { epoch, view, digest, certificate };
        self.state.height = height;
        // Older checkpoints retain the committees needed to replay their own history.
        self.state.committees.retain(|&epoch, _| epoch >= from_epoch);
        self.finalizations.insert(height, finalization.clone());
        self.checkpoints.insert(height, self.state.clone());
        Ok(finalization)
    }

    /// Returns the certificate for a retained signed height, excluding the unsigned anchor.
    pub fn finalization(&self, height: u64) -> Option<&TempoFinalization> {
        self.finalizations.get(&height)
    }

    /// Returns the latest signed height and its certificate.
    pub fn latest(&self) -> Option<(u64, &TempoFinalization)> {
        self.finalizations.last_key_value().map(|(&height, certificate)| (height, certificate))
    }

    fn identity_at(&self, epoch: u64) -> TempoConsensusIdentity {
        let (&from_epoch, committee) = self.committee_at(epoch);
        TempoConsensusIdentity {
            identity: committee.output.public().public().encode().into(),
            proposer: B256::from(<[u8; 32]>::from(&committee.proposer)),
            from_epoch,
            epoch_length: self.epoch_length.get(),
            participants: 1,
            quorum: committee.output.quorum::<N3f1>(),
        }
    }

    fn committee_at(&self, epoch: u64) -> (&u64, &Arc<Committee>) {
        self.state.committees.range(..=epoch).next_back().expect("the anchor has a committee")
    }

    fn check_next_height(&self, height: u64) -> Result<(), TempoConsensusError> {
        let expected =
            self.state.height.checked_add(1).ok_or(TempoConsensusError::HeightOverflow)?;
        if height != expected {
            return Err(TempoConsensusError::UnexpectedHeight { expected, actual: height });
        }
        Ok(())
    }

    fn header_metadata(
        &self,
        height: u64,
        include_context: bool,
    ) -> Result<PreparedHeaderMetadata, TempoConsensusError> {
        let (epoch, view, parent_view) = self.round(height);
        let extra_data = if height % self.epoch_length == self.epoch_length.get() - 1 {
            let next_epoch = epoch.checked_add(1).ok_or(TempoConsensusError::EpochOverflow)?;
            let committee = self.committee_at(next_epoch).1;
            OnchainDkgOutcome {
                epoch: next_epoch,
                output: committee.output.clone(),
                next_players: committee.output.players().clone(),
                is_next_full_dkg: false,
            }
            .encode()
            .into()
        } else {
            Bytes::new()
        };
        let consensus_context = include_context.then(|| {
            let proposer = B256::from(<[u8; 32]>::from(&self.committee_at(epoch).1.proposer))
                .try_into()
                .expect("the committee has a valid ed25519 public key");
            TempoConsensusContext { epoch, view, parent_view, proposer }
        });
        Ok(PreparedHeaderMetadata { consensus_context, extra_data })
    }

    /// Schedules a fresh committee after the epoch containing the next block.
    ///
    /// Repeated calls replace the pending key for the same epoch, consuming one counter each.
    /// A key already announced by a finalized boundary cannot be replaced by this operation.
    pub fn rotate(&mut self) -> Result<TempoConsensusIdentity, TempoConsensusError> {
        let next_height =
            self.state.height.checked_add(1).ok_or(TempoConsensusError::HeightOverflow)?;
        let from_epoch = (next_height / self.epoch_length)
            .checked_add(1)
            .ok_or(TempoConsensusError::EpochOverflow)?;
        let counter =
            self.state.counter.checked_add(1).ok_or(TempoConsensusError::CounterOverflow)?;
        self.state.committees.insert(from_epoch, Arc::new(Committee::derive(self.seed, counter)));
        self.state.counter = counter;
        Ok(self.identity_at(from_epoch))
    }

    /// Removes unwound certificates and restores the retained height's key schedule and counter.
    ///
    /// Pending rotations after that checkpoint are discarded. Rewinding zero blocks is a no-op;
    /// use a cloned controller to restore an exact snapshot, including pending rotations.
    /// Heights below the most recent pruning boundary cannot be restored.
    pub fn unwind(&mut self, height: u64) -> Result<(), TempoConsensusError> {
        if height < self.retained_height || height > self.state.height {
            return Err(TempoConsensusError::InvalidRewind {
                earliest: self.retained_height,
                head: self.state.height,
                requested: height,
            });
        }
        if height == self.state.height {
            return Ok(());
        }
        self.state = if height == self.initial.height {
            self.initial.clone()
        } else {
            self.checkpoints.get(&height).expect("retained signed heights have checkpoints").clone()
        };
        self.checkpoints.retain(|&number, _| number <= height);
        self.finalizations.retain(|&number, _| number <= height);
        Ok(())
    }

    /// Clears certificates and rotations, restoring the original unsigned anchor and identity.
    pub fn reset(&mut self) {
        self.state = self.initial.clone();
        self.checkpoints.clear();
        self.finalizations.clear();
        self.retained_height = self.initial.height;
    }

    /// Returns the epoch, view, and parent view for a non-genesis height.
    fn round(&self, height: u64) -> (u64, u64, u64) {
        let epoch = height / self.epoch_length;
        let view = if epoch == 0 { height } else { height % self.epoch_length + 1 };
        (epoch, view, view - 1)
    }

    /// Removes certificates and checkpoints strictly below `below`, advancing the rewind floor.
    ///
    /// The current head and pending rotations are unchanged. Any checkpoint and certificate at
    /// `below` remain available, so pruning at the head retains its finalized state. Requests at
    /// or below the existing floor are no-ops; requests above the head fail without changing state.
    /// [`Self::reset`] still restores the original unsigned anchor and clears the pruning boundary.
    pub fn prune(&mut self, below: u64) -> Result<(), TempoConsensusError> {
        if below > self.state.height {
            return Err(TempoConsensusError::InvalidPrune {
                head: self.state.height,
                requested: below,
            });
        }
        if below > self.retained_height {
            self.checkpoints = self.checkpoints.split_off(&below);
            self.finalizations = self.finalizations.split_off(&below);
            self.retained_height = below;
        }
        Ok(())
    }

    /// Returns the unsigned initial height, independently of the pruning boundary.
    pub(super) const fn anchor_height(&self) -> u64 {
        self.initial.height
    }
}

/// Public development committee information, without secret key material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoConsensusIdentity {
    /// Compressed 96-byte BLS12-381 G2 public key.
    pub identity: Bytes,
    /// The committee's ed25519 proposer key.
    pub proposer: B256,
    /// The first epoch using this identity.
    pub from_epoch: u64,
    /// Number of block heights per epoch.
    pub epoch_length: u64,
    /// Number of committee members.
    pub participants: u32,
    /// Number of shares required to sign a certificate.
    pub quorum: u32,
}

/// Certificate data bound to a complete native Tempo header.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TempoFinalization {
    /// The proposal's epoch.
    pub epoch: u64,
    /// The proposal's view.
    pub view: u64,
    /// Keccak-256 of the complete RLP-encoded Tempo header.
    pub digest: B256,
    /// Native Simplex finalization bytes with no additional framing.
    #[serde(with = "alloy_primitives::hex::serde::no_prefix")]
    pub certificate: Bytes,
}

/// Local identity state at the anchor or a finalized height.
#[derive(Clone)]
struct Checkpoint {
    height: u64,
    counter: u64,
    committees: BTreeMap<u64, Arc<Committee>>,
}

struct PreparedHeaderMetadata {
    consensus_context: Option<TempoConsensusContext>,
    extra_data: Bytes,
}

/// Invalid local consensus input or exhausted height/identity space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TempoConsensusError {
    /// The current height has no representable successor.
    #[error("Tempo consensus block height overflow")]
    HeightOverflow,
    /// The next epoch cannot be represented.
    #[error("Tempo consensus epoch overflow")]
    EpochOverflow,
    /// The header is not the strict successor of the local head.
    #[error("expected Tempo consensus height {expected}, got {actual}")]
    UnexpectedHeight { expected: u64, actual: u64 },
    /// The header's context differs from the selected local round or proposer.
    #[error("invalid Tempo consensus context")]
    InvalidContext,
    /// The header's DKG data differs from the current committee schedule.
    #[error("invalid Tempo consensus extra data")]
    InvalidExtraData,
    /// All development identity derivation counters have been exhausted.
    #[error("Tempo consensus identity counter overflow")]
    CounterOverflow,
    /// The rewind target is outside this controller's retained history.
    #[error(
        "cannot rewind Tempo consensus from {head} to {requested} with earliest retained height {earliest}"
    )]
    InvalidRewind { earliest: u64, head: u64, requested: u64 },
    /// A new hardfork requires review of the pinned header, DKG, and certificate formats.
    #[error("unsupported Tempo consensus hardfork {0}")]
    UnsupportedHardfork(TempoHardfork),
    /// The requested pruning boundary is beyond the current head.
    #[error("cannot prune Tempo consensus below {requested} beyond head {head}")]
    InvalidPrune { head: u64, requested: u64 },
}

struct Committee {
    proposer: PublicKey,
    output: Output<MinSig, PublicKey>,
    share: Share,
}

impl Committee {
    fn derive(seed: B256, counter: u64) -> Self {
        // The counter is an eight-byte big-endian integer. Generate the proposer before the
        // sharing from the same stream to preserve the spike's deterministic development keys.
        let mut input = seed.to_vec();
        input.extend_from_slice(&counter.to_be_bytes());
        let mut rng = ChaCha20Rng::from_seed(keccak256(input).0);
        let proposer = PrivateKey::random(&mut rng).public_key();
        let players = ordered::Set::try_from_iter([proposer.clone()]).expect("one distinct player");
        let (output, shares) =
            feldman_desmedt::deal::<MinSig, _, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("dealing to one player cannot fail");
        let share = shares.values()[0].clone();
        Self { proposer, output, share }
    }
}

const fn ensure_supported_hardfork(hardfork: TempoHardfork) -> Result<(), TempoConsensusError> {
    // Upstream appends new hardforks; extending support requires a wire-format review.
    if hardfork as u64 <= TempoHardfork::T14 as u64 {
        Ok(())
    } else {
        Err(TempoConsensusError::UnsupportedHardfork(hardfork))
    }
}

#[cfg(test)]
mod tests;
