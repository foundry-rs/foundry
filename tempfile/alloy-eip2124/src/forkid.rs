//! EIP-2124 implementation based on <https://eips.ethereum.org/EIPS/eip-2124>.
//!
//! Previously version of Apache licenced [`ethereum-forkid`](https://crates.io/crates/ethereum-forkid).

use crate::Head;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};
use alloy_primitives::{hex, BlockNumber, B256};
use alloy_rlp::{Error as RlpError, *};
use core::{
    cmp::Ordering,
    fmt,
    ops::{Add, AddAssign},
};
use crc::*;

const CRC_32_IEEE: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);
const TIMESTAMP_BEFORE_ETHEREUM_MAINNET: u64 = 1_300_000_000;

/// `CRC32` hash of all previous forks starting from genesis block.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(all(test, feature = "std"), feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, RlpEncodableWrapper, RlpDecodableWrapper, RlpMaxEncodedLen,
)]
pub struct ForkHash(pub [u8; 4]);

impl fmt::Debug for ForkHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ForkHash").field(&hex::encode(&self.0[..])).finish()
    }
}

impl From<B256> for ForkHash {
    fn from(genesis: B256) -> Self {
        Self(CRC_32_IEEE.checksum(&genesis[..]).to_be_bytes())
    }
}

impl<T> AddAssign<T> for ForkHash
where
    T: Into<u64>,
{
    fn add_assign(&mut self, v: T) {
        let blob = v.into().to_be_bytes();
        let digest = CRC_32_IEEE.digest_with_initial(u32::from_be_bytes(self.0));
        let value = digest.finalize();
        let mut digest = CRC_32_IEEE.digest_with_initial(value);
        digest.update(&blob);
        self.0 = digest.finalize().to_be_bytes();
    }
}

impl<T> Add<T> for ForkHash
where
    T: Into<u64>,
{
    type Output = Self;
    fn add(mut self, block: T) -> Self {
        self += block;
        self
    }
}

/// How to filter forks.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkFilterKey {
    /// By block number activation.
    Block(BlockNumber),
    /// By timestamp activation.
    Time(u64),
}

impl PartialOrd for ForkFilterKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ForkFilterKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Block(a), Self::Block(b)) | (Self::Time(a), Self::Time(b)) => a.cmp(b),
            (Self::Block(_), Self::Time(_)) => Ordering::Less,
            _ => Ordering::Greater,
        }
    }
}

impl From<ForkFilterKey> for u64 {
    fn from(value: ForkFilterKey) -> Self {
        match value {
            ForkFilterKey::Block(block) => block,
            ForkFilterKey::Time(time) => time,
        }
    }
}

/// A fork identifier as defined by EIP-2124.
/// Serves as the chain compatibility identifier.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(all(test, feature = "std"), feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable, RlpMaxEncodedLen)]
pub struct ForkId {
    /// CRC32 checksum of the all fork blocks and timestamps from genesis.
    pub hash: ForkHash,
    /// Next upcoming fork block number or timestamp, 0 if not yet known.
    pub next: u64,
}

/// Represents a forward-compatible ENR entry for including the forkid in a node record via
/// EIP-868. Forward compatibility is achieved via EIP-8.
///
/// See:
/// <https://github.com/ethereum/devp2p/blob/master/enr-entries/eth.md#entry-format>
///
/// for how geth implements `ForkId` values and forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq, RlpEncodable)]
pub struct EnrForkIdEntry {
    /// The inner forkid
    pub fork_id: ForkId,
}

impl Decodable for EnrForkIdEntry {
    // NOTE(onbjerg): Manual implementation to satisfy EIP-8.
    //
    // See https://eips.ethereum.org/EIPS/eip-8
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let b = &mut &**buf;
        let rlp_head = Header::decode(b)?;
        if !rlp_head.list {
            return Err(RlpError::UnexpectedString);
        }
        let started_len = b.len();

        let this = Self { fork_id: Decodable::decode(b)? };

        // NOTE(onbjerg): Because of EIP-8, we only check that we did not consume *more* than the
        // payload length, i.e. it is ok if payload length is greater than what we consumed, as we
        // just discard the remaining list items
        let consumed = started_len - b.len();
        if consumed > rlp_head.payload_length {
            return Err(RlpError::ListLengthMismatch {
                expected: rlp_head.payload_length,
                got: consumed,
            });
        }

        let rem = rlp_head.payload_length - consumed;
        b.advance(rem);
        *buf = *b;

        Ok(this)
    }
}

impl From<ForkId> for EnrForkIdEntry {
    fn from(fork_id: ForkId) -> Self {
        Self { fork_id }
    }
}

impl From<EnrForkIdEntry> for ForkId {
    fn from(entry: EnrForkIdEntry) -> Self {
        entry.fork_id
    }
}

/// Reason for rejecting provided `ForkId`.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq, Hash)]
pub enum ValidationError {
    /// Remote node is outdated and needs a software update.
    #[error(
        "remote node is outdated and needs a software update: local={local:?}, remote={remote:?}"
    )]
    RemoteStale {
        /// locally configured forkId
        local: ForkId,
        /// `ForkId` received from remote
        remote: ForkId,
    },
    /// Local node is on an incompatible chain or needs a software update.
    #[error("local node is on an incompatible chain or needs a software update: local={local:?}, remote={remote:?}")]
    LocalIncompatibleOrStale {
        /// locally configured forkId
        local: ForkId,
        /// `ForkId` received from remote
        remote: ForkId,
    },
}

/// Filter that describes the state of blockchain and can be used to check incoming `ForkId`s for
/// compatibility.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkFilter {
    /// The forks in the filter are keyed by `(timestamp, block)`. This ensures that block-based
    /// forks (`time == 0`) are processed before time-based forks as required by
    /// [EIP-6122][eip-6122].
    ///
    /// Time-based forks have their block number set to 0, allowing easy comparisons with a [Head];
    /// a fork is active if both it's time and block number are less than or equal to [Head].
    ///
    /// [eip-6122]: https://eips.ethereum.org/EIPS/eip-6122
    forks: BTreeMap<ForkFilterKey, ForkHash>,

    /// The current head, used to select forks that are active locally.
    head: Head,

    cache: Cache,
}

impl ForkFilter {
    /// Create the filter from provided head, genesis block hash, past forks and expected future
    /// forks.
    pub fn new<F>(head: Head, genesis_hash: B256, genesis_timestamp: u64, forks: F) -> Self
    where
        F: IntoIterator<Item = ForkFilterKey>,
    {
        let genesis_fork_hash = ForkHash::from(genesis_hash);
        let mut forks = forks.into_iter().collect::<BTreeSet<_>>();
        forks.remove(&ForkFilterKey::Time(0));
        forks.remove(&ForkFilterKey::Block(0));

        let forks = forks
            .into_iter()
            // filter out forks that are pre-genesis by timestamp
            .filter(|key| match key {
                ForkFilterKey::Block(_) => true,
                ForkFilterKey::Time(time) => *time > genesis_timestamp,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .fold(
                (BTreeMap::from([(ForkFilterKey::Block(0), genesis_fork_hash)]), genesis_fork_hash),
                |(mut acc, base_hash), key| {
                    let fork_hash = base_hash + u64::from(key);
                    acc.insert(key, fork_hash);
                    (acc, fork_hash)
                },
            )
            .0;

        // Compute cache based on filtered forks and the current head.
        let cache = Cache::compute_cache(&forks, head);

        // Create and return a new `ForkFilter`.
        Self { forks, head, cache }
    }

    fn set_head_priv(&mut self, head: Head) -> Option<ForkTransition> {
        let head_in_past = match self.cache.epoch_start {
            ForkFilterKey::Block(epoch_start_block) => head.number < epoch_start_block,
            ForkFilterKey::Time(epoch_start_time) => head.timestamp < epoch_start_time,
        };
        let head_in_future = match self.cache.epoch_end {
            Some(ForkFilterKey::Block(epoch_end_block)) => head.number >= epoch_end_block,
            Some(ForkFilterKey::Time(epoch_end_time)) => head.timestamp >= epoch_end_time,
            None => false,
        };

        self.head = head;

        // Recompute the cache if the head is in the past or future epoch.
        (head_in_past || head_in_future).then(|| {
            let past = self.current();
            self.cache = Cache::compute_cache(&self.forks, head);
            ForkTransition { current: self.current(), past }
        })
    }

    /// Set the current head.
    ///
    /// If the update updates the current [`ForkId`] it returns a [`ForkTransition`]
    pub fn set_head(&mut self, head: Head) -> Option<ForkTransition> {
        self.set_head_priv(head)
    }

    /// Return current fork id
    #[must_use]
    pub const fn current(&self) -> ForkId {
        self.cache.fork_id
    }

    /// Manually set the current fork id.
    ///
    /// Caution: this disregards all configured fork filters and is reset on the next head update.
    /// This is useful for testing or to connect to networks over p2p where only the latest forkid
    /// is known.
    pub fn set_current_fork_id(&mut self, fork_id: ForkId) {
        self.cache.fork_id = fork_id;
    }

    /// Check whether the provided `ForkId` is compatible based on the validation rules in
    /// `EIP-2124`.
    ///
    /// Implements the rules following: <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2124.md#stale-software-examples>
    ///
    /// # Errors
    ///
    /// Returns a `ValidationError` if the `ForkId` is not compatible.
    pub fn validate(&self, fork_id: ForkId) -> Result<(), ValidationError> {
        // 1) If local and remote FORK_HASH matches...
        if self.current().hash == fork_id.hash {
            if fork_id.next == 0 {
                // 1b) No remotely announced fork, connect.
                return Ok(());
            }

            let is_incompatible = if self.head.number < TIMESTAMP_BEFORE_ETHEREUM_MAINNET {
                // When the block number is less than an old timestamp before Ethereum mainnet,
                // we check if this fork is time-based or block number-based by estimating that,
                // if fork_id.next is bigger than the old timestamp, we are dealing with a
                // timestamp, otherwise with a block.
                (fork_id.next > TIMESTAMP_BEFORE_ETHEREUM_MAINNET
                    && self.head.timestamp >= fork_id.next)
                    || (fork_id.next <= TIMESTAMP_BEFORE_ETHEREUM_MAINNET
                        && self.head.number >= fork_id.next)
            } else {
                // Extra safety check to future-proof for when Ethereum has over a billion blocks.
                let head_block_or_time = match self.cache.epoch_start {
                    ForkFilterKey::Block(_) => self.head.number,
                    ForkFilterKey::Time(_) => self.head.timestamp,
                };
                head_block_or_time >= fork_id.next
            };

            return if is_incompatible {
                // 1a) A remotely announced but remotely not passed block is already passed locally,
                // disconnect, since the chains are incompatible.
                Err(ValidationError::LocalIncompatibleOrStale {
                    local: self.current(),
                    remote: fork_id,
                })
            } else {
                // 1b) Remotely announced fork not yet passed locally, connect.
                Ok(())
            };
        }

        // 2) If the remote FORK_HASH is a subset of the local past forks...
        let mut it = self.cache.past.iter();
        while let Some((_, hash)) = it.next() {
            if *hash == fork_id.hash {
                // ...and the remote FORK_NEXT matches with the locally following fork block number
                // or timestamp, connect.
                if let Some((actual_key, _)) = it.next() {
                    return if u64::from(*actual_key) == fork_id.next {
                        Ok(())
                    } else {
                        Err(ValidationError::RemoteStale { local: self.current(), remote: fork_id })
                    };
                }

                break;
            }
        }

        // 3) If the remote FORK_HASH is a superset of the local past forks and can be completed
        // with locally known future forks, connect.
        for future_fork_hash in &self.cache.future {
            if *future_fork_hash == fork_id.hash {
                return Ok(());
            }
        }

        // 4) Reject in all other cases.
        Err(ValidationError::LocalIncompatibleOrStale { local: self.current(), remote: fork_id })
    }
}

/// Represents a transition from one fork to another
///
/// See also [`ForkFilter::set_head`]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ForkTransition {
    /// The new, active `ForkId`
    pub current: ForkId,
    /// The previously active `ForkId` before the transition
    pub past: ForkId,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cache {
    // An epoch is a period between forks.
    // When we progress from one fork to the next one we move to the next epoch.
    epoch_start: ForkFilterKey,
    epoch_end: Option<ForkFilterKey>,
    past: Vec<(ForkFilterKey, ForkHash)>,
    future: Vec<ForkHash>,
    fork_id: ForkId,
}

impl Cache {
    /// Compute cache.
    fn compute_cache(forks: &BTreeMap<ForkFilterKey, ForkHash>, head: Head) -> Self {
        // Prepare vectors to store past and future forks.
        let mut past = Vec::with_capacity(forks.len());
        let mut future = Vec::with_capacity(forks.len());

        // Initialize variables to track the epoch range.
        let mut epoch_start = ForkFilterKey::Block(0);
        let mut epoch_end = None;

        // Iterate through forks and categorize them into past and future.
        for (key, hash) in forks {
            // Check if the fork is active based on its type (Block or Time).
            let active = match key {
                ForkFilterKey::Block(block) => *block <= head.number,
                ForkFilterKey::Time(time) => *time <= head.timestamp,
            };

            // Categorize forks into past or future based on activity.
            if active {
                epoch_start = *key;
                past.push((*key, *hash));
            } else {
                if epoch_end.is_none() {
                    epoch_end = Some(*key);
                }
                future.push(*hash);
            }
        }

        // Create ForkId using the last past fork's hash and the next epoch start.
        let fork_id = ForkId {
            hash: past.last().expect("there is always at least one - genesis - fork hash").1,
            next: epoch_end.unwrap_or(ForkFilterKey::Block(0)).into(),
        };

        // Return the computed cache.
        Self { epoch_start, epoch_end, past, future, fork_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::b256;
    use alloy_rlp::encode_fixed_size;

    /// The Ethereum mainnet genesis hash.
    const MAINNET_GENESIS_HASH: B256 =
        b256!("d4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3");

    // EIP test vectors.
    #[test]
    fn forkhash() {
        let mut fork_hash = ForkHash::from(MAINNET_GENESIS_HASH);
        assert_eq!(fork_hash.0, hex!("fc64ec04"));

        fork_hash += 1_150_000u64;
        assert_eq!(fork_hash.0, hex!("97c2c34c"));

        fork_hash += 1_920_000u64;
        assert_eq!(fork_hash.0, hex!("91d1f948"));
    }

    #[test]
    fn compatibility_check() {
        let mut filter = ForkFilter::new(
            Head { number: 0, ..Default::default() },
            MAINNET_GENESIS_HASH,
            0,
            vec![
                ForkFilterKey::Block(1_150_000),
                ForkFilterKey::Block(1_920_000),
                ForkFilterKey::Block(2_463_000),
                ForkFilterKey::Block(2_675_000),
                ForkFilterKey::Block(4_370_000),
                ForkFilterKey::Block(7_280_000),
            ],
        );

        // Local is mainnet Petersburg, remote announces the same. No future fork is announced.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        assert_eq!(filter.validate(ForkId { hash: ForkHash(hex!("668db0af")), next: 0 }), Ok(()));

        // Local is mainnet Petersburg, remote announces the same. Remote also announces a next fork
        // at block 0xffffffff, but that is uncertain.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        assert_eq!(
            filter.validate(ForkId { hash: ForkHash(hex!("668db0af")), next: BlockNumber::MAX }),
            Ok(())
        );

        // Local is mainnet currently in Byzantium only (so it's aware of Petersburg),remote
        // announces also Byzantium, but it's not yet aware of Petersburg (e.g. non updated
        // node before the fork). In this case we don't know if Petersburg passed yet or
        // not.
        filter.set_head(Head { number: 7_279_999, ..Default::default() });
        assert_eq!(filter.validate(ForkId { hash: ForkHash(hex!("a00bc324")), next: 0 }), Ok(()));

        // Local is mainnet currently in Byzantium only (so it's aware of Petersburg), remote
        // announces also Byzantium, and it's also aware of Petersburg (e.g. updated node
        // before the fork). We don't know if Petersburg passed yet (will pass) or not.
        filter.set_head(Head { number: 7_279_999, ..Default::default() });
        assert_eq!(
            filter.validate(ForkId { hash: ForkHash(hex!("a00bc324")), next: 7_280_000 }),
            Ok(())
        );

        // Local is mainnet currently in Byzantium only (so it's aware of Petersburg), remote
        // announces also Byzantium, and it's also aware of some random fork (e.g.
        // misconfigured Petersburg). As neither forks passed at neither nodes, they may
        // mismatch, but we still connect for now.
        filter.set_head(Head { number: 7_279_999, ..Default::default() });
        assert_eq!(
            filter.validate(ForkId { hash: ForkHash(hex!("a00bc324")), next: BlockNumber::MAX }),
            Ok(())
        );

        // Local is mainnet Petersburg, remote announces Byzantium + knowledge about Petersburg.
        // Remote is simply out of sync, accept.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        assert_eq!(
            filter.validate(ForkId { hash: ForkHash(hex!("a00bc324")), next: 7_280_000 }),
            Ok(())
        );

        // Local is mainnet Petersburg, remote announces Spurious + knowledge about Byzantium.
        // Remote is definitely out of sync. It may or may not need the Petersburg update,
        // we don't know yet.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        assert_eq!(
            filter.validate(ForkId { hash: ForkHash(hex!("3edd5b10")), next: 4_370_000 }),
            Ok(())
        );

        // Local is mainnet Byzantium, remote announces Petersburg. Local is out of sync, accept.
        filter.set_head(Head { number: 7_279_999, ..Default::default() });
        assert_eq!(filter.validate(ForkId { hash: ForkHash(hex!("668db0af")), next: 0 }), Ok(()));

        // Local is mainnet Spurious, remote announces Byzantium, but is not aware of Petersburg.
        // Local out of sync. Local also knows about a future fork, but that is uncertain
        // yet.
        filter.set_head(Head { number: 4_369_999, ..Default::default() });
        assert_eq!(filter.validate(ForkId { hash: ForkHash(hex!("a00bc324")), next: 0 }), Ok(()));

        // Local is mainnet Petersburg. remote announces Byzantium but is not aware of further
        // forks. Remote needs software update.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        let remote = ForkId { hash: ForkHash(hex!("a00bc324")), next: 0 };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::RemoteStale { local: filter.current(), remote })
        );

        // Local is mainnet Petersburg, and isn't aware of more forks. Remote announces Petersburg +
        // 0xffffffff. Local needs software update, reject.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        let remote = ForkId { hash: ForkHash(hex!("5cddc0e1")), next: 0 };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // Local is mainnet Byzantium, and is aware of Petersburg. Remote announces Petersburg +
        // 0xffffffff. Local needs software update, reject.
        filter.set_head(Head { number: 7_279_999, ..Default::default() });
        let remote = ForkId { hash: ForkHash(hex!("5cddc0e1")), next: 0 };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // Local is mainnet Petersburg, remote is Rinkeby Petersburg.
        filter.set_head(Head { number: 7_987_396, ..Default::default() });
        let remote = ForkId { hash: ForkHash(hex!("afec6b27")), next: 0 };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // Local is mainnet Petersburg, far in the future. Remote announces Gopherium (non existing
        // fork) at some future block 88888888, for itself, but past block for local. Local
        // is incompatible.
        //
        // This case detects non-upgraded nodes with majority hash power (typical Ropsten mess).
        filter.set_head(Head { number: 88_888_888, ..Default::default() });
        let remote = ForkId { hash: ForkHash(hex!("668db0af")), next: 88_888_888 };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // Local is mainnet Byzantium. Remote is also in Byzantium, but announces Gopherium (non
        // existing fork) at block 7279999, before Petersburg. Local is incompatible.
        filter.set_head(Head { number: 7_279_999, ..Default::default() });
        let remote = ForkId { hash: ForkHash(hex!("a00bc324")), next: 7_279_999 };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // Block far in the future (block number bigger than TIMESTAMP_BEFORE_ETHEREUM_MAINNET), not
        // compatible.
        filter
            .set_head(Head { number: TIMESTAMP_BEFORE_ETHEREUM_MAINNET + 1, ..Default::default() });
        let remote = ForkId {
            hash: ForkHash(hex!("668db0af")),
            next: TIMESTAMP_BEFORE_ETHEREUM_MAINNET + 1,
        };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // Block far in the future (block number bigger than TIMESTAMP_BEFORE_ETHEREUM_MAINNET),
        // compatible.
        filter
            .set_head(Head { number: TIMESTAMP_BEFORE_ETHEREUM_MAINNET + 1, ..Default::default() });
        let remote = ForkId {
            hash: ForkHash(hex!("668db0af")),
            next: TIMESTAMP_BEFORE_ETHEREUM_MAINNET + 2,
        };
        assert_eq!(filter.validate(remote), Ok(()));

        // block number smaller than TIMESTAMP_BEFORE_ETHEREUM_MAINNET and
        // fork_id.next > TIMESTAMP_BEFORE_ETHEREUM_MAINNET && self.head.timestamp >= fork_id.next,
        // not compatible.
        filter.set_head(Head {
            number: TIMESTAMP_BEFORE_ETHEREUM_MAINNET - 1,
            timestamp: TIMESTAMP_BEFORE_ETHEREUM_MAINNET + 2,
            ..Default::default()
        });
        let remote = ForkId {
            hash: ForkHash(hex!("668db0af")),
            next: TIMESTAMP_BEFORE_ETHEREUM_MAINNET + 1,
        };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // block number smaller than TIMESTAMP_BEFORE_ETHEREUM_MAINNET and
        // fork_id.next <= TIMESTAMP_BEFORE_ETHEREUM_MAINNET && self.head.number >= fork_id.next,
        // not compatible.
        filter
            .set_head(Head { number: TIMESTAMP_BEFORE_ETHEREUM_MAINNET - 1, ..Default::default() });
        let remote = ForkId {
            hash: ForkHash(hex!("668db0af")),
            next: TIMESTAMP_BEFORE_ETHEREUM_MAINNET - 2,
        };
        assert_eq!(
            filter.validate(remote),
            Err(ValidationError::LocalIncompatibleOrStale { local: filter.current(), remote })
        );

        // block number smaller than TIMESTAMP_BEFORE_ETHEREUM_MAINNET and
        // !((fork_id.next > TIMESTAMP_BEFORE_ETHEREUM_MAINNET && self.head.timestamp >=
        // fork_id.next) || (fork_id.next <= TIMESTAMP_BEFORE_ETHEREUM_MAINNET && self.head.number
        // >= fork_id.next)), compatible.
        filter
            .set_head(Head { number: TIMESTAMP_BEFORE_ETHEREUM_MAINNET - 2, ..Default::default() });
        let remote = ForkId {
            hash: ForkHash(hex!("668db0af")),
            next: TIMESTAMP_BEFORE_ETHEREUM_MAINNET - 1,
        };
        assert_eq!(filter.validate(remote), Ok(()));
    }

    #[test]
    fn forkid_serialization() {
        assert_eq!(
            &*encode_fixed_size(&ForkId { hash: ForkHash(hex!("00000000")), next: 0 }),
            hex!("c6840000000080")
        );
        assert_eq!(
            &*encode_fixed_size(&ForkId { hash: ForkHash(hex!("deadbeef")), next: 0xBADD_CAFE }),
            hex!("ca84deadbeef84baddcafe")
        );
        assert_eq!(
            &*encode_fixed_size(&ForkId { hash: ForkHash(hex!("ffffffff")), next: u64::MAX }),
            hex!("ce84ffffffff88ffffffffffffffff")
        );

        assert_eq!(
            ForkId::decode(&mut (&hex!("c6840000000080") as &[u8])).unwrap(),
            ForkId { hash: ForkHash(hex!("00000000")), next: 0 }
        );
        assert_eq!(
            ForkId::decode(&mut (&hex!("ca84deadbeef84baddcafe") as &[u8])).unwrap(),
            ForkId { hash: ForkHash(hex!("deadbeef")), next: 0xBADD_CAFE }
        );
        assert_eq!(
            ForkId::decode(&mut (&hex!("ce84ffffffff88ffffffffffffffff") as &[u8])).unwrap(),
            ForkId { hash: ForkHash(hex!("ffffffff")), next: u64::MAX }
        );
    }

    #[test]
    fn fork_id_rlp() {
        // <https://github.com/ethereum/go-ethereum/blob/767b00b0b514771a663f3362dd0310fc28d40c25/core/forkid/forkid_test.go#L370-L370>
        let val = hex!("c6840000000080");
        let id = ForkId::decode(&mut &val[..]).unwrap();
        assert_eq!(id, ForkId { hash: ForkHash(hex!("00000000")), next: 0 });
        assert_eq!(alloy_rlp::encode(id), &val[..]);

        let val = hex!("ca84deadbeef84baddcafe");
        let id = ForkId::decode(&mut &val[..]).unwrap();
        assert_eq!(id, ForkId { hash: ForkHash(hex!("deadbeef")), next: 0xBADDCAFE });
        assert_eq!(alloy_rlp::encode(id), &val[..]);

        let val = hex!("ce84ffffffff88ffffffffffffffff");
        let id = ForkId::decode(&mut &val[..]).unwrap();
        assert_eq!(id, ForkId { hash: ForkHash(u32::MAX.to_be_bytes()), next: u64::MAX });
        assert_eq!(alloy_rlp::encode(id), &val[..]);
    }

    #[test]
    fn compute_cache() {
        let b1 = 1_150_000;
        let b2 = 1_920_000;

        let h0 = ForkId { hash: ForkHash(hex!("fc64ec04")), next: b1 };
        let h1 = ForkId { hash: ForkHash(hex!("97c2c34c")), next: b2 };
        let h2 = ForkId { hash: ForkHash(hex!("91d1f948")), next: 0 };

        let mut fork_filter = ForkFilter::new(
            Head { number: 0, ..Default::default() },
            MAINNET_GENESIS_HASH,
            0,
            vec![ForkFilterKey::Block(b1), ForkFilterKey::Block(b2)],
        );

        assert!(fork_filter.set_head_priv(Head { number: 0, ..Default::default() }).is_none());
        assert_eq!(fork_filter.current(), h0);

        assert!(fork_filter.set_head_priv(Head { number: 1, ..Default::default() }).is_none());
        assert_eq!(fork_filter.current(), h0);

        assert_eq!(
            fork_filter.set_head_priv(Head { number: b1 + 1, ..Default::default() }).unwrap(),
            ForkTransition { current: h1, past: h0 }
        );
        assert_eq!(fork_filter.current(), h1);

        assert!(fork_filter.set_head_priv(Head { number: b1, ..Default::default() }).is_none());
        assert_eq!(fork_filter.current(), h1);

        assert_eq!(
            fork_filter.set_head_priv(Head { number: b1 - 1, ..Default::default() }).unwrap(),
            ForkTransition { current: h0, past: h1 }
        );
        assert_eq!(fork_filter.current(), h0);

        assert!(fork_filter.set_head_priv(Head { number: b1, ..Default::default() }).is_some());
        assert_eq!(fork_filter.current(), h1);

        assert!(fork_filter.set_head_priv(Head { number: b2 - 1, ..Default::default() }).is_none());
        assert_eq!(fork_filter.current(), h1);

        assert!(fork_filter.set_head_priv(Head { number: b2, ..Default::default() }).is_some());
        assert_eq!(fork_filter.current(), h2);
    }

    mod eip8 {
        use super::*;

        fn junk_enr_fork_id_entry() -> Vec<u8> {
            let mut buf = Vec::new();
            // enr request is just an expiration
            let fork_id = ForkId { hash: ForkHash(hex!("deadbeef")), next: 0xBADDCAFE };

            // add some junk
            let junk: u64 = 112233;

            // rlp header encoding
            let payload_length = fork_id.length() + junk.length();
            alloy_rlp::Header { list: true, payload_length }.encode(&mut buf);

            // fields
            fork_id.encode(&mut buf);
            junk.encode(&mut buf);

            buf
        }

        #[test]
        fn eip8_decode_enr_fork_id_entry() {
            let enr_fork_id_entry_with_junk = junk_enr_fork_id_entry();

            let mut buf = enr_fork_id_entry_with_junk.as_slice();
            let decoded = EnrForkIdEntry::decode(&mut buf).unwrap();
            assert_eq!(
                decoded.fork_id,
                ForkId { hash: ForkHash(hex!("deadbeef")), next: 0xBADDCAFE }
            );
        }
    }
}
