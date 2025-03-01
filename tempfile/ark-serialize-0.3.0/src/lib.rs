#![cfg_attr(not(feature = "std"), no_std)]
#![warn(unused, future_incompatible, nonstandard_style, rust_2018_idioms)]
#![forbid(unsafe_code)]
mod error;
mod flags;

pub use ark_std::io::{Read, Write};
use ark_std::{
    borrow::{Cow, ToOwned},
    collections::{BTreeMap, BTreeSet},
    convert::TryFrom,
    rc::Rc,
    string::String,
    vec::Vec,
};
pub use error::*;
pub use flags::*;

#[cfg(feature = "derive")]
#[doc(hidden)]
pub use ark_serialize_derive::*;

use digest::{generic_array::GenericArray, Digest};

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

/// Serializer in little endian format.
/// The serialization format must be 'length-extension' safe.
/// e.g. if T implements Canonical Serialize and Deserialize,
/// then for all strings `x, y`, if `a = T::deserialize(Reader(x))` and `a` is not an error,
/// then it must be the case that `a = T::deserialize(Reader(x || y))`,
/// and that both readers read the same number of bytes.
///
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
///
/// If your code depends on `algebra` instead, the example works analogously
/// when importing `algebra::serialize::*`.
pub trait CanonicalSerialize {
    /// Serializes `self` into `writer`.
    /// It is left up to a particular type for how it strikes the
    /// serialization efficiency vs compression tradeoff.
    /// For standard types (e.g. `bool`, lengths, etc.) typically an uncompressed
    /// form is used, whereas for algebraic types compressed forms are used.
    ///
    /// Particular examples of interest:
    /// `bool` - 1 byte encoding
    /// uints - Direct encoding
    /// Length prefixing (for any container implemented by default) - 8 byte encoding
    /// Elliptic curves - compressed point encoding
    fn serialize<W: Write>(&self, writer: W) -> Result<(), SerializationError>;

    fn serialized_size(&self) -> usize;

    /// Serializes `self` into `writer` without compression.
    #[inline]
    fn serialize_uncompressed<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.serialize(writer)
    }

    /// Serializes `self` into `writer` without compression, and without
    /// performing validity checks. Should be used *only* when there is no
    /// danger of adversarial manipulation of the output.
    #[inline]
    fn serialize_unchecked<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.serialize_uncompressed(writer)
    }

    #[inline]
    fn uncompressed_size(&self) -> usize {
        self.serialized_size()
    }
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
    fn hash<H: Digest>(&self) -> GenericArray<u8, <H as Digest>::OutputSize> {
        let mut hasher = H::new();
        self.serialize(HashMarshaller(&mut hasher))
            .expect("HashMarshaller::flush should be infaillible!");
        hasher.finalize()
    }

    fn hash_uncompressed<H: Digest>(&self) -> GenericArray<u8, <H as Digest>::OutputSize> {
        let mut hasher = H::new();
        self.serialize_uncompressed(HashMarshaller(&mut hasher))
            .expect("HashMarshaller::flush should be infaillible!");
        hasher.finalize()
    }
}

/// CanonicalSerializeHashExt is a (blanket) extension trait of
/// CanonicalSerialize
impl<T: CanonicalSerialize> CanonicalSerializeHashExt for T {}

/// Deserializer in little endian format allowing flags to be encoded.
pub trait CanonicalDeserializeWithFlags: Sized {
    /// Reads `Self` and `Flags` from `reader`.
    /// Returns empty flags by default.
    fn deserialize_with_flags<R: Read, F: Flags>(
        reader: R,
    ) -> Result<(Self, F), SerializationError>;
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
///
/// If your code depends on `algebra` instead, the example works analogously
/// when importing `algebra::serialize::*`.
pub trait CanonicalDeserialize: Sized {
    /// Reads `Self` from `reader`.
    fn deserialize<R: Read>(reader: R) -> Result<Self, SerializationError>;

    /// Reads `Self` from `reader` without compression.
    #[inline]
    fn deserialize_uncompressed<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Self::deserialize(reader)
    }

    /// Reads `self` from `reader` without compression, and without performing
    /// validity checks. Should be used *only* when the input is trusted.
    #[inline]
    fn deserialize_unchecked<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Self::deserialize_uncompressed(reader)
    }
}

// Macro for implementing serialize for u8, u16, u32, u64
macro_rules! impl_uint {
    ($ty: ident) => {
        impl CanonicalSerialize for $ty {
            #[inline]
            fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
                Ok(writer.write_all(&self.to_le_bytes())?)
            }

            #[inline]
            fn serialized_size(&self) -> usize {
                core::mem::size_of::<$ty>()
            }
        }

        impl CanonicalDeserialize for $ty {
            #[inline]
            fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
                let mut bytes = [0u8; core::mem::size_of::<$ty>()];
                reader.read_exact(&mut bytes)?;
                Ok($ty::from_le_bytes(bytes))
            }
        }
    };
}

impl_uint!(u8);
impl_uint!(u16);
impl_uint!(u32);
impl_uint!(u64);

// Serialize usize with 8 bytes
impl CanonicalSerialize for usize {
    #[inline]
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        Ok(writer.write_all(&(*self as u64).to_le_bytes())?)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        core::mem::size_of::<u64>()
    }
}

impl CanonicalDeserialize for usize {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let mut bytes = [0u8; core::mem::size_of::<u64>()];
        reader.read_exact(&mut bytes)?;
        usize::try_from(u64::from_le_bytes(bytes)).map_err(|_| SerializationError::InvalidData)
    }
}

// Implement Serialization for `String`
// It is serialized by obtaining its byte representation as a Vec<u8> and
// serializing that. This yields an end serialization of
// `string.len() || string_bytes`.
impl CanonicalSerialize for String {
    #[inline]
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.clone().into_bytes().serialize(&mut writer)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.clone().into_bytes().serialized_size()
    }
}

impl CanonicalDeserialize for String {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        String::from_utf8(Vec::<u8>::deserialize(&mut reader)?)
            .map_err(|_| SerializationError::InvalidData)
    }
}

impl<T: CanonicalSerialize> CanonicalSerialize for [T] {
    #[inline]
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize(&mut writer)?;
        for item in self.iter() {
            item.serialize(&mut writer)?;
        }
        Ok(())
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        8 + self
            .iter()
            .map(|item| item.serialized_size())
            .sum::<usize>()
    }

    #[inline]
    fn serialize_uncompressed<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize(&mut writer)?;
        for item in self.iter() {
            item.serialize_uncompressed(&mut writer)?;
        }
        Ok(())
    }

    #[inline]
    fn serialize_unchecked<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize(&mut writer)?;
        for item in self.iter() {
            item.serialize_unchecked(&mut writer)?;
        }
        Ok(())
    }

    #[inline]
    fn uncompressed_size(&self) -> usize {
        8 + self
            .iter()
            .map(|item| item.uncompressed_size())
            .sum::<usize>()
    }
}

impl<T: CanonicalSerialize> CanonicalSerialize for Vec<T> {
    #[inline]
    fn serialize<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.as_slice().serialize(writer)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.as_slice().serialized_size()
    }

    #[inline]
    fn serialize_uncompressed<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.as_slice().serialize_uncompressed(writer)
    }

    #[inline]
    fn serialize_unchecked<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.as_slice().serialize_unchecked(writer)
    }

    #[inline]
    fn uncompressed_size(&self) -> usize {
        self.as_slice().uncompressed_size()
    }
}

impl<T: CanonicalDeserialize> CanonicalDeserialize for Vec<T> {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize(&mut reader)?;
        let mut values = Vec::new();
        for _ in 0..len {
            values.push(T::deserialize(&mut reader)?);
        }
        Ok(values)
    }

    #[inline]
    fn deserialize_uncompressed<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize(&mut reader)?;
        let mut values = Vec::new();
        for _ in 0..len {
            values.push(T::deserialize_uncompressed(&mut reader)?);
        }
        Ok(values)
    }

    #[inline]
    fn deserialize_unchecked<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize(&mut reader)?;
        let mut values = Vec::new();
        for _ in 0..len {
            values.push(T::deserialize_unchecked(&mut reader)?);
        }
        Ok(values)
    }
}

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

// Implement Serialization for tuples
macro_rules! impl_tuple {
    ($( $ty: ident : $no: tt, )*) => {
        impl<$($ty, )*> CanonicalSerialize for ($($ty,)*) where
            $($ty: CanonicalSerialize,)*
        {
            #[inline]
            fn serialize<W: Write>(&self, mut _writer: W) -> Result<(), SerializationError> {
                $(self.$no.serialize(&mut _writer)?;)*
                Ok(())
            }

            #[inline]
            fn serialized_size(&self) -> usize {
                [$(
                    self.$no.serialized_size(),
                )*].iter().sum()
            }

            #[inline]
            fn serialize_uncompressed<W: Write>(&self, mut _writer: W) -> Result<(), SerializationError> {
                $(self.$no.serialize_uncompressed(&mut _writer)?;)*
                Ok(())
            }

            #[inline]
            fn serialize_unchecked<W: Write>(&self, mut _writer: W) -> Result<(), SerializationError> {
                $(self.$no.serialize_unchecked(&mut _writer)?;)*
                Ok(())
            }

            #[inline]
            fn uncompressed_size(&self) -> usize {
                [$(
                    self.$no.uncompressed_size(),
                )*].iter().sum()
            }
        }

        impl<$($ty, )*> CanonicalDeserialize for ($($ty,)*) where
            $($ty: CanonicalDeserialize,)*
        {
            #[inline]
            fn deserialize<R: Read>(mut _reader: R) -> Result<Self, SerializationError> {
                Ok(($(
                    $ty::deserialize(&mut _reader)?,
                )*))
            }

            #[inline]
            fn deserialize_uncompressed<R: Read>(mut _reader: R) -> Result<Self, SerializationError> {
                Ok(($(
                    $ty::deserialize_uncompressed(&mut _reader)?,
                )*))
            }

            #[inline]
            fn deserialize_unchecked<R: Read>(mut _reader: R) -> Result<Self, SerializationError> {
                Ok(($(
                    $ty::deserialize_unchecked(&mut _reader)?,
                )*))
            }
        }
    }
}

impl_tuple!();
impl_tuple!(A:0, B:1,);
impl_tuple!(A:0, B:1, C:2,);
impl_tuple!(A:0, B:1, C:2, D:3,);

// No-op
impl<T> CanonicalSerialize for core::marker::PhantomData<T> {
    #[inline]
    fn serialize<W: Write>(&self, _writer: W) -> Result<(), SerializationError> {
        Ok(())
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        0
    }
}

impl<T> CanonicalDeserialize for core::marker::PhantomData<T> {
    #[inline]
    fn deserialize<R: Read>(_reader: R) -> Result<Self, SerializationError> {
        Ok(core::marker::PhantomData)
    }
}

// Serialize cow objects by serializing the underlying object.
impl<'a, T: CanonicalSerialize + ToOwned> CanonicalSerialize for Cow<'a, T> {
    #[inline]
    fn serialize<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.as_ref().serialize(writer)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.as_ref().serialized_size()
    }

    #[inline]
    fn serialize_uncompressed<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.as_ref().serialize_uncompressed(writer)
    }

    #[inline]
    fn serialize_unchecked<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.as_ref().serialize_unchecked(writer)
    }

    fn uncompressed_size(&self) -> usize {
        self.as_ref().uncompressed_size()
    }
}

impl<'a, T> CanonicalDeserialize for Cow<'a, T>
where
    T: ToOwned,
    <T as ToOwned>::Owned: CanonicalDeserialize,
{
    #[inline]
    fn deserialize<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Ok(Cow::Owned(<T as ToOwned>::Owned::deserialize(reader)?))
    }

    #[inline]
    fn deserialize_uncompressed<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Ok(Cow::Owned(<T as ToOwned>::Owned::deserialize_uncompressed(
            reader,
        )?))
    }

    #[inline]
    fn deserialize_unchecked<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Ok(Cow::Owned(<T as ToOwned>::Owned::deserialize_unchecked(
            reader,
        )?))
    }
}

// If Option<T> is None, serialize as serialize(False).
// If its Some, serialize as serialize(True) || serialize(T)
impl<T: CanonicalSerialize> CanonicalSerialize for Option<T> {
    #[inline]
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.is_some().serialize(&mut writer)?;
        if let Some(item) = self {
            item.serialize(&mut writer)?;
        }

        Ok(())
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.is_some().serialized_size()
            + if let Some(item) = self {
                item.serialized_size()
            } else {
                0
            }
    }

    #[inline]
    fn serialize_uncompressed<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.is_some().serialize_uncompressed(&mut writer)?;
        if let Some(item) = self {
            item.serialize_uncompressed(&mut writer)?;
        }

        Ok(())
    }

    #[inline]
    fn uncompressed_size(&self) -> usize {
        self.is_some().uncompressed_size()
            + if let Some(item) = self {
                item.uncompressed_size()
            } else {
                0
            }
    }

    #[inline]
    fn serialize_unchecked<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.is_some().serialize_unchecked(&mut writer)?;
        if let Some(item) = self {
            item.serialize_unchecked(&mut writer)?;
        }

        Ok(())
    }
}

impl<T: CanonicalDeserialize> CanonicalDeserialize for Option<T> {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let is_some = bool::deserialize(&mut reader)?;
        let data = if is_some {
            Some(T::deserialize(&mut reader)?)
        } else {
            None
        };

        Ok(data)
    }

    #[inline]
    fn deserialize_uncompressed<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let is_some = bool::deserialize_uncompressed(&mut reader)?;
        let data = if is_some {
            Some(T::deserialize_uncompressed(&mut reader)?)
        } else {
            None
        };

        Ok(data)
    }

    #[inline]
    fn deserialize_unchecked<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let is_some = bool::deserialize_unchecked(&mut reader)?;
        let data = if is_some {
            Some(T::deserialize_unchecked(&mut reader)?)
        } else {
            None
        };

        Ok(data)
    }
}

// Implement Serialization for `Rc<T>`
impl<T: CanonicalSerialize> CanonicalSerialize for Rc<T> {
    #[inline]
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.as_ref().serialize(&mut writer)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.as_ref().serialized_size()
    }

    #[inline]
    fn serialize_uncompressed<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.as_ref().serialize_uncompressed(&mut writer)
    }

    #[inline]
    fn uncompressed_size(&self) -> usize {
        self.as_ref().uncompressed_size()
    }

    #[inline]
    fn serialize_unchecked<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        self.as_ref().serialize_unchecked(&mut writer)
    }
}

impl<T: CanonicalDeserialize> CanonicalDeserialize for Rc<T> {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        Ok(Rc::new(T::deserialize(&mut reader)?))
    }

    #[inline]
    fn deserialize_uncompressed<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        Ok(Rc::new(T::deserialize_uncompressed(&mut reader)?))
    }

    #[inline]
    fn deserialize_unchecked<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        Ok(Rc::new(T::deserialize_unchecked(&mut reader)?))
    }
}

// Serialize boolean with a full byte
impl CanonicalSerialize for bool {
    #[inline]
    fn serialize<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        (*self as u8).serialize(writer)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        1
    }
}

impl CanonicalDeserialize for bool {
    #[inline]
    fn deserialize<R: Read>(reader: R) -> Result<Self, SerializationError> {
        let val = u8::deserialize(reader)?;
        if val == 0 {
            return Ok(false);
        } else if val == 1 {
            return Ok(true);
        }

        Err(SerializationError::InvalidData)
    }

    #[inline]
    fn deserialize_unchecked<R: Read>(reader: R) -> Result<Self, SerializationError> {
        Ok(u8::deserialize(reader)? == 1)
    }
}

// Serialize BTreeMap as `len(map) || key 1 || value 1 || ... || key n || value n`
impl<K, V> CanonicalSerialize for BTreeMap<K, V>
where
    K: CanonicalSerialize,
    V: CanonicalSerialize,
{
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize(&mut writer)?;
        for (k, v) in self.iter() {
            k.serialize(&mut writer)?;
            v.serialize(&mut writer)?;
        }
        Ok(())
    }

    fn serialized_size(&self) -> usize {
        8 + self
            .iter()
            .map(|(k, v)| k.serialized_size() + v.serialized_size())
            .sum::<usize>()
    }

    fn serialize_uncompressed<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_uncompressed(&mut writer)?;
        for (k, v) in self.iter() {
            k.serialize_uncompressed(&mut writer)?;
            v.serialize_uncompressed(&mut writer)?;
        }
        Ok(())
    }

    fn serialize_unchecked<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_unchecked(&mut writer)?;
        for (k, v) in self.iter() {
            k.serialize_unchecked(&mut writer)?;
            v.serialize_unchecked(&mut writer)?;
        }
        Ok(())
    }

    fn uncompressed_size(&self) -> usize {
        8 + self
            .iter()
            .map(|(k, v)| k.uncompressed_size() + v.uncompressed_size())
            .sum::<usize>()
    }
}

impl<K, V> CanonicalDeserialize for BTreeMap<K, V>
where
    K: Ord + CanonicalDeserialize,
    V: CanonicalDeserialize,
{
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize(&mut reader)?;
        let mut map = BTreeMap::new();
        for _ in 0..len {
            map.insert(K::deserialize(&mut reader)?, V::deserialize(&mut reader)?);
        }
        Ok(map)
    }

    fn deserialize_uncompressed<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize_uncompressed(&mut reader)?;
        let mut map = BTreeMap::new();
        for _ in 0..len {
            map.insert(
                K::deserialize_uncompressed(&mut reader)?,
                V::deserialize_uncompressed(&mut reader)?,
            );
        }
        Ok(map)
    }

    fn deserialize_unchecked<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize_unchecked(&mut reader)?;
        let mut map = BTreeMap::new();
        for _ in 0..len {
            map.insert(
                K::deserialize_unchecked(&mut reader)?,
                V::deserialize_unchecked(&mut reader)?,
            );
        }
        Ok(map)
    }
}

// Serialize BTreeSet as `len(set) || value_1 || ... || value_n`.
impl<T: CanonicalSerialize> CanonicalSerialize for BTreeSet<T> {
    fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize(&mut writer)?;
        for elem in self.iter() {
            elem.serialize(&mut writer)?;
        }
        Ok(())
    }

    fn serialized_size(&self) -> usize {
        8 + self
            .iter()
            .map(|elem| elem.serialized_size())
            .sum::<usize>()
    }

    fn serialize_uncompressed<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_uncompressed(&mut writer)?;
        for elem in self.iter() {
            elem.serialize_uncompressed(&mut writer)?;
        }
        Ok(())
    }

    fn serialize_unchecked<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_unchecked(&mut writer)?;
        for elem in self.iter() {
            elem.serialize_unchecked(&mut writer)?;
        }
        Ok(())
    }

    fn uncompressed_size(&self) -> usize {
        8 + self
            .iter()
            .map(|elem| elem.uncompressed_size())
            .sum::<usize>()
    }
}

impl<T: CanonicalDeserialize + Ord> CanonicalDeserialize for BTreeSet<T> {
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize(&mut reader)?;
        let mut set = BTreeSet::new();
        for _ in 0..len {
            set.insert(T::deserialize(&mut reader)?);
        }
        Ok(set)
    }

    fn deserialize_uncompressed<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize_uncompressed(&mut reader)?;
        let mut set = BTreeSet::new();
        for _ in 0..len {
            set.insert(T::deserialize_uncompressed(&mut reader)?);
        }
        Ok(set)
    }

    fn deserialize_unchecked<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let len = u64::deserialize_unchecked(&mut reader)?;
        let mut set = BTreeSet::new();
        for _ in 0..len {
            set.insert(T::deserialize_unchecked(&mut reader)?);
        }
        Ok(set)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ark_std::rand::RngCore;
    use ark_std::vec;

    #[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
    struct Dummy;

    impl CanonicalSerialize for Dummy {
        #[inline]
        fn serialize<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
            100u8.serialize(&mut writer)
        }

        #[inline]
        fn serialized_size(&self) -> usize {
            100u8.serialized_size()
        }

        #[inline]
        fn serialize_uncompressed<W: Write>(
            &self,
            mut writer: W,
        ) -> Result<(), SerializationError> {
            (&[100u8, 200u8]).serialize_uncompressed(&mut writer)
        }

        #[inline]
        fn uncompressed_size(&self) -> usize {
            (&[100u8, 200u8]).uncompressed_size()
        }

        #[inline]
        fn serialize_unchecked<W: Write>(&self, mut writer: W) -> Result<(), SerializationError> {
            (&[100u8, 200u8]).serialize_unchecked(&mut writer)
        }
    }

    impl CanonicalDeserialize for Dummy {
        #[inline]
        fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
            let result = u8::deserialize(&mut reader)?;
            assert_eq!(result, 100u8);
            Ok(Dummy)
        }

        #[inline]
        fn deserialize_uncompressed<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
            let result = Vec::<u8>::deserialize_uncompressed(&mut reader)?;
            assert_eq!(result.as_slice(), &[100u8, 200u8]);

            Ok(Dummy)
        }

        #[inline]
        fn deserialize_unchecked<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
            let result = Vec::<u8>::deserialize_unchecked(&mut reader)?;
            assert_eq!(result.as_slice(), &[100u8, 200u8]);

            Ok(Dummy)
        }
    }

    fn test_serialize<
        T: PartialEq + core::fmt::Debug + CanonicalSerialize + CanonicalDeserialize,
    >(
        data: T,
    ) {
        let mut serialized = vec![0; data.serialized_size()];
        data.serialize(&mut serialized[..]).unwrap();
        let de = T::deserialize(&serialized[..]).unwrap();
        assert_eq!(data, de);

        let mut serialized = vec![0; data.uncompressed_size()];
        data.serialize_uncompressed(&mut serialized[..]).unwrap();
        let de = T::deserialize_uncompressed(&serialized[..]).unwrap();
        assert_eq!(data, de);

        let mut serialized = vec![0; data.uncompressed_size()];
        data.serialize_unchecked(&mut serialized[..]).unwrap();
        let de = T::deserialize_unchecked(&serialized[..]).unwrap();
        assert_eq!(data, de);
    }

    fn test_hash<T: CanonicalSerialize, H: Digest + core::fmt::Debug>(data: T) {
        let h1 = data.hash::<H>();

        let mut hash = H::new();
        let mut serialized = vec![0; data.serialized_size()];
        data.serialize(&mut serialized[..]).unwrap();
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
        let mut serialized = vec![0; data.serialized_size()];
        r.fill_bytes(&mut serialized);
        while !valid_mutation(&serialized) {
            r.fill_bytes(&mut serialized);
        }
        let de = T::deserialize(&serialized[..]);
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
    fn test_rc() {
        test_serialize(Rc::new(Dummy));
    }

    #[test]
    fn test_bool() {
        test_serialize(true);
        test_serialize(false);

        let valid_mutation = |data: &[u8]| -> bool {
            return data.len() == 1 && data[0] > 1;
        };
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
        test_hash::<_, blake2::Blake2b>(Dummy);
        test_hash::<_, blake2::Blake2s>(Dummy);
    }

    #[test]
    fn test_sha3() {
        test_hash::<_, sha3::Sha3_256>(Dummy);
        test_hash::<_, sha3::Sha3_512>(Dummy);
    }
}
