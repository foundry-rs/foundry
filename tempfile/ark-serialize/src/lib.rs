#![cfg_attr(not(feature = "std"), no_std)]
#![warn(
    unused,
    future_incompatible,
    nonstandard_style,
    rust_2018_idioms,
    rust_2021_compatibility
)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
mod error;
mod flags;
mod impls;

use ark_std::borrow::ToOwned;
pub use ark_std::io::{Read, Write};

pub use error::*;
pub use flags::*;

#[cfg(feature = "derive")]
#[doc(hidden)]
pub use ark_serialize_derive::*;

use digest::{generic_array::GenericArray, Digest, OutputSizeUser};

/// Whether to use a compressed version of the serialization algorithm. Specific behavior depends
/// on implementation. If no compressed version exists (e.g. on `Fp`), mode is ignored.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Compress {
    Yes,
    No,
}

/// Whether to validate the element after deserializing it. Specific behavior depends on
/// implementation. If no validation algorithm exists (e.g. on `Fp`), mode is ignored.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Validate {
    Yes,
    No,
}

pub trait Valid: Sized + Sync {
    fn check(&self) -> Result<(), SerializationError>;

    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        #[cfg(feature = "parallel")]
        {
            use rayon::{iter::ParallelBridge, prelude::ParallelIterator};
            batch.par_bridge().try_for_each(|e| e.check())?;
        }
        #[cfg(not(feature = "parallel"))]
        {
            for item in batch {
                item.check()?;
            }
        }
        Ok(())
    }
}

/// Serializer in little endian format.
/// This trait can be derived if all fields of a struct implement
/// `CanonicalSerialize` and the `derive` feature is enabled.
///
/// # Example
/// ```
/// // The `derive` feature must be set for the derivation to work.
/// use ark_serialize::*;
///
/// # #[cfg(feature = "derive")]
/// #[derive(CanonicalSerialize)]
/// struct TestStruct {
///     a: u64,
///     b: (u64, (u64, u64)),
/// }
/// ```
pub trait CanonicalSerialize {
    /// The general serialize method that takes in customization flags.
    fn serialize_with_mode<W: Write>(
        &self,
        writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError>;

    fn serialized_size(&self, compress: Compress) -> usize;

    fn serialize_compressed<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.serialize_with_mode(writer, Compress::Yes)
    }

    fn compressed_size(&self) -> usize {
        self.serialized_size(Compress::Yes)
    }

    fn serialize_uncompressed<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.serialize_with_mode(writer, Compress::No)
    }

    fn uncompressed_size(&self) -> usize {
        self.serialized_size(Compress::No)
    }
}

/// Deserializer in little endian format.
/// This trait can be derived if all fields of a struct implement
/// `CanonicalDeserialize` and the `derive` feature is enabled.
///
/// # Example
/// ```
/// // The `derive` feature must be set for the derivation to work.
/// use ark_serialize::*;
///
/// # #[cfg(feature = "derive")]
/// #[derive(CanonicalDeserialize)]
/// struct TestStruct {
///     a: u64,
///     b: (u64, (u64, u64)),
/// }
/// ```
pub trait CanonicalDeserialize: Valid {
    /// The general deserialize method that takes in customization flags.
    fn deserialize_with_mode<R: Read>(
        reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError>;

    fn deserialize_compressed<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Self::deserialize_with_mode(reader, Compress::Yes, Validate::Yes)
    }

    fn deserialize_compressed_unchecked<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Self::deserialize_with_mode(reader, Compress::Yes, Validate::No)
    }

    fn deserialize_uncompressed<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Self::deserialize_with_mode(reader, Compress::No, Validate::Yes)
    }

    fn deserialize_uncompressed_unchecked<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Self::deserialize_with_mode(reader, Compress::No, Validate::No)
    }
}

/// Serializer in little endian format allowing to encode flags.
pub trait CanonicalSerializeWithFlags: CanonicalSerialize {
    /// Serializes `self` and `flags` into `writer`.
    fn serialize_with_flags<W: Write, F: Flags>(
        &self,
        writer: W,
        flags: F,
    ) -> Result<(), SerializationError>;

    /// Serializes `self` and `flags` into `writer`.
    fn serialized_size_with_flags<F: Flags>(&self) -> usize;
}

/// Deserializer in little endian format allowing flags to be encoded.
pub trait CanonicalDeserializeWithFlags: Sized {
    /// Reads `Self` and `Flags` from `reader`.
    /// Returns empty flags by default.
    fn deserialize_with_flags<R: Read, F: Flags>(
        reader: R,
    ) -> Result<(Self, F), SerializationError>;
}

// This private struct works around Serialize taking the pre-existing
// std::io::Write instance of most digest::Digest implementations by value
struct HashMarshaller<'a, H: Digest>(&'a mut H);

impl<'a, H: Digest> ark_std::io::Write for HashMarshaller<'a, H> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> ark_std::io::Result<usize> {
        Digest::update(self.0, buf);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> ark_std::io::Result<()> {
        Ok(())
    }
}

/// The CanonicalSerialize induces a natural way to hash the
/// corresponding value, of which this is the convenience trait.
pub trait CanonicalSerializeHashExt: CanonicalSerialize {
    fn hash<H: Digest>(&self) -> GenericArray<u8, <H as OutputSizeUser>::OutputSize> {
        let mut hasher = H::new();
        self.serialize_compressed(HashMarshaller(&mut hasher))
            .expect("HashMarshaller::flush should be infaillible!");
        hasher.finalize()
    }

    fn hash_uncompressed<H: Digest>(&self) -> GenericArray<u8, <H as OutputSizeUser>::OutputSize> {
        let mut hasher = H::new();
        self.serialize_uncompressed(HashMarshaller(&mut hasher))
            .expect("HashMarshaller::flush should be infaillible!");
        hasher.finalize()
    }
}

/// CanonicalSerializeHashExt is a (blanket) extension trait of
/// CanonicalSerialize
impl<T: CanonicalSerialize> CanonicalSerializeHashExt for T {}

#[inline]
pub fn buffer_bit_byte_size(modulus_bits: usize) -> (usize, usize) {
    let byte_size = buffer_byte_size(modulus_bits);
    ((byte_size * 8), byte_size)
}

/// Converts the number of bits required to represent a number
/// into the number of bytes required to represent it.
#[inline]
pub const fn buffer_byte_size(modulus_bits: usize) -> usize {
    (modulus_bits + 7) / 8
}

#[cfg(test)]
mod test {
    use super::*;
    use ark_std::{
        collections::{BTreeMap, BTreeSet},
        rand::RngCore,
        string::String,
        vec,
        vec::Vec,
    };
    use num_bigint::BigUint;

    #[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
    struct Dummy;

    impl CanonicalSerialize for Dummy {
        #[inline]
        fn serialize_with_mode<W: Write>(
            &self,
            mut writer: W,
            compress: Compress,
        ) -> Result<(), SerializationError> {
            match compress {
                Compress::Yes => 100u8.serialize_compressed(&mut writer),
                Compress::No => [100u8, 200u8].serialize_compressed(&mut writer),
            }
        }

        fn serialized_size(&self, compress: Compress) -> usize {
            match compress {
                Compress::Yes => 1,
                Compress::No => 2,
            }
        }
    }

    impl Valid for Dummy {
        fn check(&self) -> Result<(), SerializationError> {
            Ok(())
        }
    }
    impl CanonicalDeserialize for Dummy {
        #[inline]
        fn deserialize_with_mode<R: Read>(
            reader: R,
            compress: Compress,
            _validate: Validate,
        ) -> Result<Self, SerializationError> {
            match compress {
                Compress::Yes => assert_eq!(u8::deserialize_compressed(reader)?, 100u8),
                Compress::No => {
                    assert_eq!(<[u8; 2]>::deserialize_compressed(reader)?, [100u8, 200u8])
                },
            }
            Ok(Dummy)
        }
    }

    fn test_serialize<
        T: PartialEq + core::fmt::Debug + CanonicalSerialize + CanonicalDeserialize,
    >(
        data: T,
    ) {
        for compress in [Compress::Yes, Compress::No] {
            for validate in [Validate::Yes, Validate::No] {
                let mut serialized = vec![0; data.serialized_size(compress)];
                data.serialize_with_mode(&mut serialized[..], compress)
                    .unwrap();
                let de = T::deserialize_with_mode(&serialized[..], compress, validate).unwrap();
                assert_eq!(data, de);
            }
        }
    }

    fn test_hash<T: CanonicalSerialize, H: Digest + core::fmt::Debug>(data: T) {
        let h1 = data.hash::<H>();

        let mut hash = H::new();
        let mut serialized = vec![0; data.serialized_size(Compress::Yes)];
        data.serialize_compressed(&mut serialized[..]).unwrap();
        hash.update(&serialized);
        let h2 = hash.finalize();

        assert_eq!(h1, h2);

        let h3 = data.hash_uncompressed::<H>();

        let mut hash = H::new();
        serialized = vec![0; data.uncompressed_size()];
        data.serialize_uncompressed(&mut serialized[..]).unwrap();
        hash.update(&serialized);
        let h4 = hash.finalize();

        assert_eq!(h3, h4);
    }

    // Serialize T, randomly mutate the data, and deserialize it.
    // Ensure it fails.
    // Up to the caller to provide a valid mutation criterion
    // to ensure that this test always fails.
    // This method requires a concrete instance of the data to be provided,
    // to get the serialized size.
    fn ensure_non_malleable_encoding<
        T: PartialEq + core::fmt::Debug + CanonicalSerialize + CanonicalDeserialize,
    >(
        data: T,
        valid_mutation: fn(&[u8]) -> bool,
    ) {
        let mut r = ark_std::test_rng();
        let mut serialized = vec![0; data.compressed_size()];
        r.fill_bytes(&mut serialized);
        while !valid_mutation(&serialized) {
            r.fill_bytes(&mut serialized);
        }
        let de = T::deserialize_compressed(&serialized[..]);
        assert!(de.is_err());

        let mut serialized = vec![0; data.uncompressed_size()];
        r.fill_bytes(&mut serialized);
        while !valid_mutation(&serialized) {
            r.fill_bytes(&mut serialized);
        }
        let de = T::deserialize_uncompressed(&serialized[..]);
        assert!(de.is_err());
    }

    #[test]
    fn test_array() {
        test_serialize([1u64, 2, 3, 4, 5]);
        test_serialize([1u8; 33]);
    }

    #[test]
    fn test_vec() {
        test_serialize(vec![1u64, 2, 3, 4, 5]);
        test_serialize(Vec::<u64>::new());
    }

    #[test]
    fn test_uint() {
        test_serialize(192830918usize);
        test_serialize(192830918u64);
        test_serialize(192830918u32);
        test_serialize(22313u16);
        test_serialize(123u8);
    }

    #[test]
    fn test_string() {
        test_serialize(String::from("arkworks"));
    }

    #[test]
    fn test_tuple() {
        test_serialize(());
        test_serialize((123u64, Dummy));
        test_serialize((123u64, 234u32, Dummy));
    }

    #[test]
    fn test_tuple_vec() {
        test_serialize(vec![
            (Dummy, Dummy, Dummy),
            (Dummy, Dummy, Dummy),
            (Dummy, Dummy, Dummy),
        ]);
        test_serialize(vec![
            (86u8, 98u64, Dummy),
            (86u8, 98u64, Dummy),
            (86u8, 98u64, Dummy),
        ]);
    }

    #[test]
    fn test_option() {
        test_serialize(Some(Dummy));
        test_serialize(None::<Dummy>);

        test_serialize(Some(10u64));
        test_serialize(None::<u64>);
    }

    #[test]
    fn test_bool() {
        test_serialize(true);
        test_serialize(false);

        let valid_mutation = |data: &[u8]| -> bool { data.len() == 1 && data[0] > 1 };
        for _ in 0..10 {
            ensure_non_malleable_encoding(true, valid_mutation);
            ensure_non_malleable_encoding(false, valid_mutation);
        }
    }

    #[test]
    fn test_btreemap() {
        let mut map = BTreeMap::new();
        map.insert(0u64, Dummy);
        map.insert(5u64, Dummy);
        test_serialize(map);
        let mut map = BTreeMap::new();
        map.insert(10u64, vec![1u8, 2u8, 3u8]);
        map.insert(50u64, vec![4u8, 5u8, 6u8]);
        test_serialize(map);
    }

    #[test]
    fn test_btreeset() {
        let mut set = BTreeSet::new();
        set.insert(Dummy);
        set.insert(Dummy);
        test_serialize(set);
        let mut set = BTreeSet::new();
        set.insert(vec![1u8, 2u8, 3u8]);
        set.insert(vec![4u8, 5u8, 6u8]);
        test_serialize(set);
    }

    #[test]
    fn test_phantomdata() {
        test_serialize(core::marker::PhantomData::<Dummy>);
    }

    #[test]
    fn test_sha2() {
        test_hash::<_, sha2::Sha256>(Dummy);
        test_hash::<_, sha2::Sha512>(Dummy);
    }

    #[test]
    fn test_blake2() {
        test_hash::<_, blake2::Blake2b512>(Dummy);
        test_hash::<_, blake2::Blake2s256>(Dummy);
    }

    #[test]
    fn test_sha3() {
        test_hash::<_, sha3::Sha3_256>(Dummy);
        test_hash::<_, sha3::Sha3_512>(Dummy);
    }

    #[test]
    fn test_biguint() {
        let biguint = BigUint::from(123456u64);
        test_serialize(biguint.clone());

        let mut expected = (biguint.to_bytes_le().len() as u64).to_le_bytes().to_vec();
        expected.extend_from_slice(&biguint.to_bytes_le());

        let mut bytes = Vec::new();
        biguint
            .serialize_with_mode(&mut bytes, Compress::Yes)
            .unwrap();
        assert_eq!(bytes, expected);

        let mut bytes = Vec::new();
        biguint
            .serialize_with_mode(&mut bytes, Compress::No)
            .unwrap();
        assert_eq!(bytes, expected);
    }
}
