//! Holds utilites for working with cryptographic digests, and disambiguating
//! digests via marker traits.
//!
//! We want to wrap hashes in marked newtypes in order to prevent
//! type-confusion between TXIDs, sighashes, and other digests with the same
//! length.

use digest::{
    core_api::{BlockSizeUser, OutputSizeUser},
    HashMarker, Output,
};
use std::io::Write;

use crate::ser::{ByteFormat, SerError, SerResult};

// Useful re-exports
pub use digest::Digest;
pub use generic_array::GenericArray;
pub use ripemd::Ripemd160;
pub use sha2::Sha256;
pub use sha3::Sha3_256;

/// Output of a Digest function
pub type DigestOutput<D> = GenericArray<u8, <D as OutputSizeUser>::OutputSize>;

/// Convenience interface for hash function outputs, particularly marked digest outputs
pub trait MarkedDigestOutput:
    Default + Copy + AsRef<[u8]> + AsMut<[u8]> + ByteFormat<Error = SerError>
{
    /// Returns the number of bytes in the digest
    fn size(&self) -> usize;

    /// Return a clone in opposite byte order
    fn reversed(&self) -> Self {
        let mut reversed = Self::default();
        let mut digest_bytes = self.as_slice().to_vec();
        digest_bytes.reverse();
        reversed
            .as_mut()
            .copy_from_slice(&digest_bytes[..self.size()]);
        reversed
    }

    /// Deserialize to BE hex
    fn from_be_hex(be: &str) -> SerResult<Self> {
        Ok(Self::deserialize_hex(be)?.reversed())
    }

    /// Convert to BE hex
    fn to_be_hex(&self) -> String {
        self.reversed().serialize_hex()
    }

    /// Use as a mutable slice
    fn as_mut_slice(&mut self) -> &mut [u8] {
        self.as_mut()
    }

    /// Use as a slice
    fn as_slice(&self) -> &[u8] {
        self.as_ref()
    }
}

/// A marked digest
pub trait MarkedDigest<D>: Digest + Default + Write
where
    D: MarkedDigestOutput,
{
    /// Produce a marked digest from the hasher
    fn finalize_marked(self) -> D;

    /// Shortcut to produce a marked digest
    fn digest_marked(data: &[u8]) -> D;
}

#[derive(Clone, Default)]
/// A `Digest` implementation that performs Bitcoin style double-sha256
pub struct Hash256(sha2::Sha256);

impl std::io::Write for Hash256 {
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }
}

impl HashMarker for Hash256 {}

impl BlockSizeUser for Hash256 {
    type BlockSize = <Sha256 as BlockSizeUser>::BlockSize;
}

impl OutputSizeUser for Hash256 {
    type OutputSize = <Sha256 as digest::OutputSizeUser>::OutputSize;
}

impl digest::FixedOutput for Hash256 {
    fn finalize_into(self, out: &mut GenericArray<u8, Self::OutputSize>) {
        let mut hasher = sha2::Sha256::default();
        hasher.update(self.0.finalize());
        Digest::finalize_into(hasher, out)
    }
}

impl digest::FixedOutputReset for Hash256 {
    fn finalize_into_reset(&mut self, out: &mut Output<Self>) {
        let other = self.clone();
        other.finalize_into(out);
        self.0.reset();
    }
}

impl digest::Reset for Hash256 {
    fn reset(&mut self) {
        Digest::reset(&mut self.0);
    }
}

impl digest::Update for Hash256 {
    fn update(&mut self, data: &[u8]) {
        Digest::update(&mut self.0, data);
    }
}

#[derive(Clone, Default)]
/// A `Digest` implementation that performs Bitcoin style double-sha256
pub struct Hash160(sha2::Sha256);

impl std::io::Write for Hash160 {
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }
}

impl HashMarker for Hash160 {}

impl BlockSizeUser for Hash160 {
    type BlockSize = <Ripemd160 as BlockSizeUser>::BlockSize;
}

impl OutputSizeUser for Hash160 {
    type OutputSize = <Ripemd160 as digest::OutputSizeUser>::OutputSize;
}

impl digest::FixedOutput for Hash160 {
    fn finalize_into(self, out: &mut GenericArray<u8, Self::OutputSize>) {
        let mut hasher = ripemd::Ripemd160::default();
        hasher.update(self.0.finalize());
        Digest::finalize_into(hasher, out)
    }
}

impl digest::FixedOutputReset for Hash160 {
    fn finalize_into_reset(&mut self, out: &mut Output<Self>) {
        let other = self.clone();
        other.finalize_into(out);
        self.0.reset();
    }
}

impl digest::Reset for Hash160 {
    fn reset(&mut self) {
        Digest::reset(&mut self.0);
    }
}

impl digest::Update for Hash160 {
    fn update(&mut self, data: &[u8]) {
        Digest::update(&mut self.0, data);
    }
}

marked_digest!(
    /// A bitcoin-style Hash160
    Hash160Digest,
    Hash160
);

marked_digest!(
    /// A bitcoin-style Hash256
    Hash256Digest,
    Hash256
);
