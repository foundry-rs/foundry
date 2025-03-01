use crate::eip4844::Blob;
#[cfg(feature = "kzg")]
use c_kzg::{KzgCommitment, KzgProof};

use crate::eip4844::{
    utils::WholeFe, BYTES_PER_BLOB, FIELD_ELEMENTS_PER_BLOB, FIELD_ELEMENT_BYTES_USIZE,
};
use alloc::vec::Vec;

#[cfg(feature = "kzg")]
use crate::eip4844::env_settings::EnvKzgSettings;
#[cfg(any(feature = "kzg", feature = "arbitrary"))]
use crate::eip4844::BlobTransactionSidecar;
#[cfg(feature = "kzg")]
use crate::eip4844::Bytes48;
use core::cmp;

/// A builder for creating a [`BlobTransactionSidecar`].
///
/// [`BlobTransactionSidecar`]: crate::eip4844::BlobTransactionSidecar
#[derive(Clone, Debug)]
pub struct PartialSidecar {
    /// The blobs in the sidecar.
    blobs: Vec<Blob>,
    /// The number of field elements that we have ingested, total.
    fe: usize,
}

impl Default for PartialSidecar {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialSidecar {
    /// Create a new builder, and push an empty blob to it. This is the default
    /// constructor, and allocates space for 2 blobs (256 KiB). If you want to
    /// preallocate a specific number of blobs, use
    /// [`PartialSidecar::with_capacity`].
    pub fn new() -> Self {
        Self::with_capacity(2)
    }

    /// Create a new builder, preallocating room for `capacity` blobs, and push
    /// an empty blob to it.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut blobs = Vec::with_capacity(capacity);
        blobs.push(Blob::new([0u8; BYTES_PER_BLOB]));
        Self { blobs, fe: 0 }
    }

    /// Get a reference to the blobs currently in the builder.
    pub fn blobs(&self) -> &[Blob] {
        &self.blobs
    }

    /// Get the number of unused field elements that have been allocated
    fn free_fe(&self) -> usize {
        self.blobs.len() * FIELD_ELEMENTS_PER_BLOB as usize - self.fe
    }

    /// Calculate the length of used field elements IN BYTES in the builder.
    ///
    /// This is always strictly greater than the number of bytes that have been
    /// ingested.
    pub const fn len(&self) -> usize {
        self.fe * 32
    }

    /// Check if the builder is empty.
    pub const fn is_empty(&self) -> bool {
        self.fe == 0
    }

    /// Push an empty blob to the builder.
    fn push_empty_blob(&mut self) {
        self.blobs.push(Blob::new([0u8; BYTES_PER_BLOB]));
    }

    /// Allocate enough space for the required number of new field elements.
    pub fn alloc_fes(&mut self, required_fe: usize) {
        while self.free_fe() < required_fe {
            self.push_empty_blob()
        }
    }

    /// Get the number of used field elements in the current blob.
    const fn fe_in_current_blob(&self) -> usize {
        self.fe % FIELD_ELEMENTS_PER_BLOB as usize
    }

    /// Get the index of the first unused field element in the current blob.
    const fn first_unused_fe_index_in_current_blob(&self) -> usize {
        self.fe_in_current_blob()
    }

    /// Get a mutable reference to the current blob.
    fn current_blob_mut(&mut self) -> &mut Blob {
        let last_unused_blob_index = self.fe / FIELD_ELEMENTS_PER_BLOB as usize;
        self.blobs.get_mut(last_unused_blob_index).expect("never empty")
    }

    /// Get a mutable reference to the field element at the given index, in
    /// the current blob.
    fn fe_at_mut(&mut self, index: usize) -> &mut [u8] {
        &mut self.current_blob_mut()[index * 32..(index + 1) * 32]
    }

    /// Get a mutable reference to the next unused field element.
    fn next_unused_fe_mut(&mut self) -> &mut [u8] {
        self.fe_at_mut(self.first_unused_fe_index_in_current_blob())
    }

    /// Ingest a field element into the current blobs.
    pub fn ingest_valid_fe(&mut self, data: WholeFe<'_>) {
        self.alloc_fes(1);
        self.next_unused_fe_mut().copy_from_slice(data.as_ref());
        self.fe += 1;
    }

    /// Ingest a partial FE into the current blobs.
    ///
    /// # Panics
    ///
    /// If the data is >=32 bytes. Or if there are not enough free FEs to
    /// encode the data.
    pub fn ingest_partial_fe(&mut self, data: &[u8]) {
        self.alloc_fes(1);
        let fe = self.next_unused_fe_mut();
        fe[1..1 + data.len()].copy_from_slice(data);
        self.fe += 1;
    }
}

/// A strategy for coding and decoding data into sidecars.
///
/// Coder instances are responsible for encoding and decoding data into and from the sidecar. They
/// are called by the [`SidecarBuilder`] during the [`ingest`], [`take`], and (if `c_kzg` feature
/// enabled) `build` methods.
///
/// This trait allows different downstream users to use different bit-packing
/// strategies. For example, a simple coder might only use the last 31 bytes of
/// each blob, while a more complex coder might use a more sophisticated
/// strategy to pack data into the low 6 bits of the top byte.
///
/// [`ingest`]: SidecarBuilder::ingest
/// [`take`]: SidecarBuilder::take
pub trait SidecarCoder {
    /// Calculate the number of field elements required to store the given
    /// data.
    fn required_fe(&self, data: &[u8]) -> usize;

    /// Code a slice of data into the builder.
    fn code(&mut self, builder: &mut PartialSidecar, data: &[u8]);

    /// Finish the sidecar, and commit to the data. This method should empty
    /// any buffer or scratch space in the coder, and is called by
    /// [`SidecarBuilder`]'s `take` and `build` methods.
    fn finish(self, builder: &mut PartialSidecar);

    /// Decode all slices of data from the blobs.
    fn decode_all(&mut self, blobs: &[Blob]) -> Option<Vec<Vec<u8>>>;
}

/// Simple coder that only uses the last 31 bytes of each blob. This is the
/// default coder for the [`SidecarBuilder`].
///
/// # Note
///
/// Because this coder sacrifices around 3% of total sidecar space, we do not
/// recommend its use in production. It is provided for convenience and
/// non-prod environments.
///
/// # Behavior
///
/// This coder encodes data as follows:
/// - The first byte of every 32-byte word is empty.
/// - Data is pre-pended with a 64-bit big-endian length prefix, which is right padded with zeros to
///   form a complete word.
/// - The rest of the data is packed into the remaining 31 bytes of each word.
/// - If the data is not a multiple of 31 bytes, the last word is right-padded with zeros.
///
/// This means that the following regions cannot be used to store data, and are
/// considered "wasted":
///
/// - The first byte of every 32-byte word.
/// - The right padding on the header word containing the data length.
/// - Any right padding on the last word for each piece of data.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SimpleCoder;

impl SimpleCoder {
    /// Decode an some bytes from an iterator of valid FEs.
    ///
    /// Returns `Ok(Some(data))` if there is some data.
    /// Returns `Ok(None)` if there is no data (empty iterator, length prefix is 0).
    /// Returns `Err(())` if there is an error.
    fn decode_one<'a>(mut fes: impl Iterator<Item = WholeFe<'a>>) -> Result<Option<Vec<u8>>, ()> {
        let Some(first) = fes.next() else {
            return Ok(None);
        };
        let mut num_bytes = u64::from_be_bytes(first.as_ref()[1..9].try_into().unwrap()) as usize;

        // if no more bytes is 0, we're done
        if num_bytes == 0 {
            return Ok(None);
        }

        // if there are too many bytes
        const MAX_ALLOCATION_SIZE: usize = 2_097_152; //2 MiB
        if num_bytes > MAX_ALLOCATION_SIZE {
            return Err(());
        }

        let mut res = Vec::with_capacity(num_bytes);
        while num_bytes > 0 {
            let to_copy = cmp::min(31, num_bytes);
            let fe = fes.next().ok_or(())?;
            res.extend_from_slice(&fe.as_ref()[1..1 + to_copy]);
            num_bytes -= to_copy;
        }
        Ok(Some(res))
    }
}

impl SidecarCoder for SimpleCoder {
    fn required_fe(&self, data: &[u8]) -> usize {
        data.len().div_ceil(31) + 1
    }

    fn code(&mut self, builder: &mut PartialSidecar, mut data: &[u8]) {
        if data.is_empty() {
            return;
        }

        // first FE is the number of following bytes
        builder.ingest_partial_fe(&(data.len() as u64).to_be_bytes());

        // ingest the rest of the data
        while !data.is_empty() {
            let (left, right) = data.split_at(cmp::min(31, data.len()));
            builder.ingest_partial_fe(left);
            data = right
        }
    }

    /// No-op
    fn finish(self, _builder: &mut PartialSidecar) {}

    fn decode_all(&mut self, blobs: &[Blob]) -> Option<Vec<Vec<u8>>> {
        if blobs.is_empty() {
            return None;
        }

        if blobs
            .iter()
            .flat_map(|blob| blob.chunks(FIELD_ELEMENT_BYTES_USIZE).map(WholeFe::new))
            .any(|fe| fe.is_none())
        {
            return None;
        }

        let mut fes = blobs
            .iter()
            .flat_map(|blob| blob.chunks(FIELD_ELEMENT_BYTES_USIZE).map(WholeFe::new_unchecked));

        let mut res = Vec::new();
        loop {
            match Self::decode_one(&mut fes) {
                Ok(Some(data)) => res.push(data),
                Ok(None) => break,
                Err(()) => return None,
            }
        }
        Some(res)
    }
}

/// Build a [`BlobTransactionSidecar`] from an arbitrary amount of data.
///
/// This is useful for creating a sidecar from a large amount of data,
/// which is then split into blobs. It delays KZG commitments and proofs
/// until all data is ready.
///
/// [`BlobTransactionSidecar`]: crate::eip4844::BlobTransactionSidecar
#[derive(Clone, Debug)]
pub struct SidecarBuilder<T = SimpleCoder> {
    /// The blob array we will code data into
    inner: PartialSidecar,
    /// The coder to use for ingesting and decoding data.
    coder: T,
}

impl<T> Default for SidecarBuilder<T>
where
    T: Default + SidecarCoder,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, T: arbitrary::Arbitrary<'a> + Clone> SidecarBuilder<T> {
    /// Builds an arbitrary realization for BlobTransactionSidecar.
    pub fn build_arbitrary(&self) -> BlobTransactionSidecar {
        <BlobTransactionSidecar as arbitrary::Arbitrary>::arbitrary(
            &mut arbitrary::Unstructured::new(&[]),
        )
        .unwrap()
    }
}

impl<T: SidecarCoder + Default> SidecarBuilder<T> {
    /// Instantiate a new builder and new coder instance.
    ///
    /// By default, this allocates space for 2 blobs (256 KiB). If you want to
    /// preallocate a specific number of blobs, use
    /// [`SidecarBuilder::with_capacity`].
    pub fn new() -> Self {
        T::default().into()
    }

    /// Create a new builder from a slice of data by calling
    /// [`SidecarBuilder::from_coder_and_data`]
    pub fn from_slice(data: &[u8]) -> Self {
        Self::from_coder_and_data(T::default(), data)
    }

    /// Create a new builder with a pre-allocated capacity. This capacity is
    /// measured in blobs, each of which is 128 KiB.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::from_coder_and_capacity(T::default(), capacity)
    }
}

impl<T: SidecarCoder> SidecarBuilder<T> {
    /// Instantiate a new builder with the provided coder and capacity. This
    /// capacity is measured in blobs, each of which is 128 KiB.
    pub fn from_coder_and_capacity(coder: T, capacity: usize) -> Self {
        Self { inner: PartialSidecar::with_capacity(capacity), coder }
    }

    /// Calculate the length of bytes used by field elements in the builder.
    ///
    /// This is always strictly greater than the number of bytes that have been
    /// ingested.
    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if the builder is empty.
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Create a new builder from a slice of data.
    pub fn from_coder_and_data(coder: T, data: &[u8]) -> Self {
        let required_fe = coder.required_fe(data);
        let mut this = Self::from_coder_and_capacity(
            coder,
            required_fe.div_ceil(FIELD_ELEMENTS_PER_BLOB as usize),
        );
        this.ingest(data);
        this
    }

    /// Ingest a slice of data into the builder.
    pub fn ingest(&mut self, data: &[u8]) {
        self.inner.alloc_fes(self.coder.required_fe(data));
        self.coder.code(&mut self.inner, data);
    }

    /// Build the sidecar from the data with the provided settings.
    #[cfg(feature = "kzg")]
    pub fn build_with_settings(
        self,
        settings: &c_kzg::KzgSettings,
    ) -> Result<BlobTransactionSidecar, c_kzg::Error> {
        let mut commitments = Vec::with_capacity(self.inner.blobs.len());
        let mut proofs = Vec::with_capacity(self.inner.blobs.len());
        for blob in &self.inner.blobs {
            // SAFETY: same size
            let blob = unsafe { core::mem::transmute::<&Blob, &c_kzg::Blob>(blob) };
            let commitment = KzgCommitment::blob_to_kzg_commitment(blob, settings)?;
            let proof = KzgProof::compute_blob_kzg_proof(blob, &commitment.to_bytes(), settings)?;

            // SAFETY: same size
            unsafe {
                commitments
                    .push(core::mem::transmute::<c_kzg::Bytes48, Bytes48>(commitment.to_bytes()));
                proofs.push(core::mem::transmute::<c_kzg::Bytes48, Bytes48>(proof.to_bytes()));
            }
        }

        Ok(BlobTransactionSidecar::new(self.inner.blobs, commitments, proofs))
    }

    /// Build the sidecar from the data, with default (Ethereum Mainnet)
    /// settings.
    #[cfg(feature = "kzg")]
    pub fn build(self) -> Result<BlobTransactionSidecar, c_kzg::Error> {
        self.build_with_settings(EnvKzgSettings::Default.get())
    }

    /// Take the blobs from the builder, without committing them to a KZG proof.
    pub fn take(self) -> Vec<Blob> {
        self.inner.blobs
    }
}

impl<T: SidecarCoder> From<T> for SidecarBuilder<T> {
    /// Instantiate a new builder with the provided coder.
    ///
    /// This is equivalent to calling
    /// [`SidecarBuilder::from_coder_and_capacity`] with a capacity of 1.
    /// If you want to preallocate a specific number of blobs, use
    /// [`SidecarBuilder::from_coder_and_capacity`].
    fn from(coder: T) -> Self {
        Self::from_coder_and_capacity(coder, 1)
    }
}

impl<T, R> FromIterator<R> for SidecarBuilder<T>
where
    T: SidecarCoder + Default,
    R: AsRef<[u8]>,
{
    fn from_iter<I: IntoIterator<Item = R>>(iter: I) -> Self {
        let mut this = Self::new();
        for data in iter {
            this.ingest(data.as_ref());
        }
        this
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eip4844::USABLE_BYTES_PER_BLOB;

    #[test]
    fn ingestion_strategy() {
        let mut builder = PartialSidecar::new();
        let data = &[
            vec![1u8; 32],
            vec![2u8; 372],
            vec![3u8; 17],
            vec![4u8; 5],
            vec![5u8; 126_945],
            vec![6u8; 2 * 126_945],
        ];

        data.iter().for_each(|data| SimpleCoder.code(&mut builder, data.as_slice()));

        let decoded = SimpleCoder.decode_all(builder.blobs()).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn big_ingestion_strategy() {
        let data = vec![1u8; 126_945];
        let builder = SidecarBuilder::<SimpleCoder>::from_slice(&data);

        let blobs = builder.take();
        let decoded = SimpleCoder.decode_all(&blobs).unwrap().concat();

        assert_eq!(decoded, data);
    }

    #[test]
    fn decode_all_rejects_invalid_data() {
        assert_eq!(SimpleCoder.decode_all(&[]), None);
        assert_eq!(SimpleCoder.decode_all(&[Blob::new([0xffu8; BYTES_PER_BLOB])]), None);
    }

    #[test]
    fn it_ingests() {
        // test ingesting a lot of data.
        let data = [
            vec![1u8; 32],
            vec![2u8; 372],
            vec![3u8; 17],
            vec![4u8; 5],
            vec![5u8; USABLE_BYTES_PER_BLOB + 2],
        ];

        let mut builder = data.iter().collect::<SidecarBuilder<SimpleCoder>>();

        let expected_fe = data.iter().map(|d| SimpleCoder.required_fe(d)).sum::<usize>();
        assert_eq!(builder.len(), expected_fe * 32);

        // consume 2 more
        builder.ingest(b"hello");
        assert_eq!(builder.len(), expected_fe * 32 + 64);
    }
}
