//! [EIP-7685]: General purpose execution layer requests
//!
//! [EIP-7685]: https://eips.ethereum.org/EIPS/eip-7685

use alloc::vec::Vec;
use alloy_primitives::{b256, Bytes, B256};
use derive_more::{Deref, DerefMut, From, IntoIterator};

/// The empty requests hash.
///
/// This is equivalent to `sha256("")`
pub const EMPTY_REQUESTS_HASH: B256 =
    b256!("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

/// A container of EIP-7685 requests.
///
/// The container only holds the `requests` as defined by their respective EIPs. The first byte of
/// each element is the `request_type` and the remaining bytes are the `request_data`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash, Deref, DerefMut, From, IntoIterator)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Requests(Vec<Bytes>);

impl Requests {
    /// Construct a new [`Requests`] container with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Construct a new [`Requests`] container.
    ///
    /// This function assumes that the request type byte is already included as the
    /// first byte in the provided `Bytes` blob.
    pub const fn new(requests: Vec<Bytes>) -> Self {
        Self(requests)
    }

    /// Add a new request into the container.
    pub fn push_request(&mut self, request: Bytes) {
        // Omit empty requests.
        if request.len() == 1 {
            return;
        }
        self.0.push(request);
    }

    /// Adds a new request with the given request type into the container.
    pub fn push_request_with_type(
        &mut self,
        request_type: u8,
        request: impl IntoIterator<Item = u8>,
    ) {
        let mut request = request.into_iter().peekable();
        // Omit empty requests.
        if request.peek().is_none() {
            return;
        }
        self.0.push(core::iter::once(request_type).chain(request).collect());
    }

    /// Consumes [`Requests`] and returns the inner raw opaque requests.
    ///
    /// # Note
    ///
    /// These requests include the `request_type` as the first byte in each
    /// `Bytes` element, followed by the `requests_data`.
    pub fn take(self) -> Vec<Bytes> {
        self.0
    }

    /// Get an iterator over the requests.
    pub fn iter(&self) -> core::slice::Iter<'_, Bytes> {
        self.0.iter()
    }

    /// Calculate the requests hash as defined in EIP-7685 for the requests.
    ///
    /// The requests hash is defined as
    ///
    /// ```text
    /// sha256(sha256(requests_0) ++ sha256(requests_1) ++ ...)
    /// ```
    ///
    /// Each request in the container is expected to already have the `request_type` prepended
    /// to its corresponding `requests_data`. This function directly calculates the hash based
    /// on the combined `request_type` and `requests_data`.
    ///
    /// Empty requests are omitted from the hash calculation.
    /// Requests are sorted by their `request_type` before hashing, see also [Ordering](https://eips.ethereum.org/EIPS/eip-7685#ordering)
    #[cfg(feature = "sha2")]
    pub fn requests_hash(&self) -> B256 {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();

        let mut requests: Vec<_> = self.0.iter().filter(|req| !req.is_empty()).collect();
        requests.sort_unstable_by_key(|req| {
            // SAFETY: only includes non-empty requests
            req[0]
        });

        for req in requests {
            let mut req_hash = Sha256::new();
            req_hash.update(req);
            hash.update(req_hash.finalize());
        }
        B256::new(hash.finalize().into())
    }

    /// Extend this container with requests from another container.
    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.take());
    }
}

/// A list of requests or a precomputed requests hash.
///
/// For testing purposes, the `Hash` variant stores a precomputed requests hash. This can be useful
/// when the exact contents of the requests are unnecessary, and only a consistent hash value is
/// needed to simulate the presence of requests without holding actual data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, derive_more::From)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RequestsOrHash {
    /// Stores a list of requests, allowing for dynamic requests hash calculation.
    Requests(Requests),
    /// Stores a precomputed requests hash, used primarily for testing or mocking because the
    /// header only contains the hash.
    Hash(B256),
}

impl RequestsOrHash {
    /// Returns the requests hash for the enum instance.
    ///
    /// - If the instance contains a list of requests, this function calculates the hash using
    ///   `requests_hash` of the [`Requests`] struct.
    /// - If it contains a precomputed hash, it returns that hash directly.
    #[cfg(feature = "sha2")]
    pub fn requests_hash(&self) -> B256 {
        match self {
            Self::Requests(requests) => requests.requests_hash(),
            Self::Hash(precomputed_hash) => *precomputed_hash,
        }
    }

    /// Returns an instance with the [`EMPTY_REQUESTS_HASH`].
    pub const fn empty() -> Self {
        Self::Hash(EMPTY_REQUESTS_HASH)
    }

    /// Returns the requests, if any.
    pub const fn requests(&self) -> Option<&Requests> {
        match self {
            Self::Requests(requests) => Some(requests),
            Self::Hash(_) => None,
        }
    }
}

impl Default for RequestsOrHash {
    fn default() -> Self {
        Self::Requests(Requests::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extend() {
        // Test extending a Requests container with another Requests container
        let mut reqs1 = Requests::new(vec![Bytes::from(vec![0x01, 0x02])]);
        let reqs2 =
            Requests::new(vec![Bytes::from(vec![0x03, 0x04]), Bytes::from(vec![0x05, 0x06])]);

        // Extend reqs1 with reqs2
        reqs1.extend(reqs2);

        // Ensure the requests are correctly combined
        assert_eq!(reqs1.0.len(), 3);
        assert_eq!(
            reqs1.0,
            vec![
                Bytes::from(vec![0x01, 0x02]),
                Bytes::from(vec![0x03, 0x04]),
                Bytes::from(vec![0x05, 0x06])
            ]
        );
    }

    #[test]
    #[cfg(feature = "sha2")]
    fn test_consistent_requests_hash() {
        // We test that the empty requests hash is consistent with the EIP-7685 definition.
        assert_eq!(Requests::default().requests_hash(), EMPTY_REQUESTS_HASH);

        // Test to hash a non-empty vector of requests.
        assert_eq!(
            Requests(vec![
                Bytes::from(vec![0x00, 0x0a, 0x0b, 0x0c]),
                Bytes::from(vec![0x01, 0x0d, 0x0e, 0x0f])
            ])
            .requests_hash(),
            b256!("be3a57667b9bb9e0275019c0faf0f415fdc8385a408fd03e13a5c50615e3530c"),
        );
    }
}
