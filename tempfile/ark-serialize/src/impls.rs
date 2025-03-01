use ark_std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    marker::PhantomData,
    rc::Rc,
    string::String,
    vec::Vec,
};
use num_bigint::BigUint;

use crate::*;

impl Valid for bool {
    fn check(&self) -> Result<(), SerializationError> {
        Ok(())
    }
}

impl CanonicalSerialize for bool {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        _compress: Compress,
    ) -> Result<(), SerializationError> {
        writer.write(&[*self as u8])?;
        Ok(())
    }

    #[inline]
    fn serialized_size(&self, _compress: Compress) -> usize {
        1
    }
}

impl CanonicalDeserialize for bool {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        match u8::deserialize_with_mode(reader, compress, validate)? {
            0u8 => Ok(false),
            1u8 => Ok(true),
            _ => Err(SerializationError::InvalidData),
        }
    }
}

macro_rules! impl_uint {
    ($type:ty) => {
        impl CanonicalSerialize for $type {
            #[inline]
            fn serialize_with_mode<W: Write>(
                &self,
                mut writer: W,
                _compress: Compress,
            ) -> Result<(), SerializationError> {
                Ok(writer.write_all(&self.to_le_bytes())?)
            }

            #[inline]
            fn serialized_size(&self, _compress: Compress) -> usize {
                core::mem::size_of::<$type>()
            }
        }

        impl Valid for $type {
            #[inline]
            fn check(&self) -> Result<(), SerializationError> {
                Ok(())
            }

            #[inline]
            fn batch_check<'a>(
                _batch: impl Iterator<Item = &'a Self>,
            ) -> Result<(), SerializationError>
            where
                Self: 'a,
            {
                Ok(())
            }
        }

        impl CanonicalDeserialize for $type {
            #[inline]
            fn deserialize_with_mode<R: Read>(
                mut reader: R,
                _compress: Compress,
                _validate: Validate,
            ) -> Result<Self, SerializationError> {
                let mut bytes = [0u8; core::mem::size_of::<$type>()];
                reader.read_exact(&mut bytes)?;
                Ok(<$type>::from_le_bytes(bytes))
            }
        }
    };
}

impl_uint!(u8);
impl_uint!(u16);
impl_uint!(u32);
impl_uint!(u64);

impl CanonicalSerialize for usize {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        _compress: Compress,
    ) -> Result<(), SerializationError> {
        Ok(writer.write_all(&(*self as u64).to_le_bytes())?)
    }

    #[inline]
    fn serialized_size(&self, _compress: Compress) -> usize {
        core::mem::size_of::<u64>()
    }
}

impl Valid for usize {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        Ok(())
    }

    #[inline]
    fn batch_check<'a>(_batch: impl Iterator<Item = &'a Self>) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        Ok(())
    }
}

impl CanonicalDeserialize for usize {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        _compress: Compress,
        _validate: Validate,
    ) -> Result<Self, SerializationError> {
        let mut bytes = [0u8; core::mem::size_of::<u64>()];
        reader.read_exact(&mut bytes)?;
        Ok(<u64>::from_le_bytes(bytes) as usize)
    }
}

impl CanonicalSerialize for BigUint {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.to_bytes_le().serialize_with_mode(writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.to_bytes_le().serialized_size(compress)
    }
}

impl CanonicalDeserialize for BigUint {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        Ok(BigUint::from_bytes_le(&Vec::<u8>::deserialize_with_mode(
            reader, compress, validate,
        )?))
    }
}

impl Valid for BigUint {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        Ok(())
    }

    #[inline]
    fn batch_check<'a>(_batch: impl Iterator<Item = &'a Self>) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        Ok(())
    }
}

impl<T: CanonicalSerialize> CanonicalSerialize for Option<T> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.is_some().serialize_with_mode(&mut writer, compress)?;
        if let Some(item) = self {
            item.serialize_with_mode(&mut writer, compress)?;
        }

        Ok(())
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        1 + self
            .as_ref()
            .map(|s| s.serialized_size(compress))
            .unwrap_or(0)
    }
}

impl<T: Valid> Valid for Option<T> {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        match self {
            Some(v) => v.check(),
            None => Ok(()),
        }
    }

    #[inline]
    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        T::batch_check(batch.map(Option::as_ref).filter(Option::is_some).flatten())
    }
}

impl<T: CanonicalDeserialize> CanonicalDeserialize for Option<T> {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let is_some = bool::deserialize_with_mode(&mut reader, compress, validate)?;
        let data = if is_some {
            Some(T::deserialize_with_mode(&mut reader, compress, validate)?)
        } else {
            None
        };

        Ok(data)
    }
}

// No-op
impl<T> CanonicalSerialize for PhantomData<T> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        _writer: W,
        _compress: Compress,
    ) -> Result<(), SerializationError> {
        Ok(())
    }

    #[inline]
    fn serialized_size(&self, _compress: Compress) -> usize {
        0
    }
}

impl<T: Sync> Valid for PhantomData<T> {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        Ok(())
    }
}

impl<T: Send + Sync> CanonicalDeserialize for PhantomData<T> {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        _reader: R,
        _compress: Compress,
        _validate: Validate,
    ) -> Result<Self, SerializationError> {
        Ok(PhantomData)
    }
}

impl<T: CanonicalSerialize + ToOwned> CanonicalSerialize for Rc<T> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.as_ref().serialize_with_mode(&mut writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.as_ref().serialized_size(compress)
    }
}

#[cfg(feature = "std")]
impl<T: CanonicalSerialize + ToOwned> CanonicalSerialize for ark_std::sync::Arc<T> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.as_ref().serialize_with_mode(&mut writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.as_ref().serialized_size(compress)
    }
}

#[cfg(feature = "std")]
impl<T: Valid + Sync + Send> Valid for ark_std::sync::Arc<T> {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        self.as_ref().check()
    }

    #[inline]

    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        T::batch_check(batch.map(|v| v.as_ref()))
    }
}

#[cfg(feature = "std")]
impl<T: CanonicalDeserialize + ToOwned + Sync + Send> CanonicalDeserialize
    for ark_std::sync::Arc<T>
{
    #[inline]
    fn deserialize_with_mode<R: Read>(
        reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        Ok(ark_std::sync::Arc::new(T::deserialize_with_mode(
            reader, compress, validate,
        )?))
    }
}

impl<'a, T: CanonicalSerialize + ToOwned> CanonicalSerialize for Cow<'a, T> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.as_ref().serialize_with_mode(&mut writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.as_ref().serialized_size(compress)
    }
}

impl<'b, T> Valid for Cow<'b, T>
where
    T: ToOwned + Sync + Valid + Send,
    <T as ToOwned>::Owned: CanonicalDeserialize + Send,
{
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        <<T as ToOwned>::Owned>::check(&self.as_ref().to_owned())
    }

    #[inline]

    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        let t: Vec<_> = batch.map(|v| v.as_ref().to_owned()).collect();
        <<T as ToOwned>::Owned>::batch_check(t.iter())
    }
}

impl<'a, T> CanonicalDeserialize for Cow<'a, T>
where
    T: ToOwned + Valid + Valid + Sync + Send,
    <T as ToOwned>::Owned: CanonicalDeserialize + Valid + Send,
{
    #[inline]
    fn deserialize_with_mode<R: Read>(
        reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        Ok(Cow::Owned(<T as ToOwned>::Owned::deserialize_with_mode(
            reader, compress, validate,
        )?))
    }
}

impl<T: CanonicalSerialize, const N: usize> CanonicalSerialize for [T; N] {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        for item in self.iter() {
            item.serialize_with_mode(&mut writer, compress)?;
        }
        Ok(())
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.iter()
            .map(|item| item.serialized_size(compress))
            .sum::<usize>()
    }
}
impl<T: CanonicalDeserialize, const N: usize> Valid for [T; N] {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        T::batch_check(self.iter())
    }

    #[inline]
    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        T::batch_check(batch.flat_map(|v| v.iter()))
    }
}

impl<T: CanonicalDeserialize, const N: usize> CanonicalDeserialize for [T; N] {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let result = core::array::from_fn(|_| {
            T::deserialize_with_mode(&mut reader, compress, Validate::No).unwrap()
        });
        if let Validate::Yes = validate {
            T::batch_check(result.iter())?
        }
        Ok(result)
    }
}

impl<T: CanonicalSerialize> CanonicalSerialize for Vec<T> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.as_slice().serialize_with_mode(&mut writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.as_slice().serialized_size(compress)
    }
}

impl<T: Valid> Valid for Vec<T> {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        T::batch_check(self.iter())
    }

    #[inline]
    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        T::batch_check(batch.flat_map(|v| v.iter()))
    }
}

impl<T: CanonicalDeserialize> CanonicalDeserialize for Vec<T> {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let len = u64::deserialize_with_mode(&mut reader, compress, validate)?;
        let mut values = Vec::new();
        for _ in 0..len {
            values.push(T::deserialize_with_mode(
                &mut reader,
                compress,
                Validate::No,
            )?);
        }

        if let Validate::Yes = validate {
            T::batch_check(values.iter())?
        }
        Ok(values)
    }
}

impl<T: CanonicalSerialize> CanonicalSerialize for [T] {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_with_mode(&mut writer, compress)?;
        for item in self.iter() {
            item.serialize_with_mode(&mut writer, compress)?;
        }
        Ok(())
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        8 + self
            .iter()
            .map(|item| item.serialized_size(compress))
            .sum::<usize>()
    }
}

impl<'a, T: CanonicalSerialize> CanonicalSerialize for &'a [T] {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        (*self).serialize_with_mode(&mut writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        (*self).serialized_size(compress)
    }
}

impl CanonicalSerialize for String {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        self.as_bytes().serialize_with_mode(&mut writer, compress)
    }

    #[inline]
    fn serialized_size(&self, compress: Compress) -> usize {
        self.as_bytes().serialized_size(compress)
    }
}

impl Valid for String {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        Ok(())
    }
}

impl CanonicalDeserialize for String {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let bytes = <Vec<u8>>::deserialize_with_mode(reader, compress, validate)?;
        String::from_utf8(bytes).map_err(|_| SerializationError::InvalidData)
    }
}

// Implement Serialization for tuples
macro_rules! impl_tuple {
    ($( $ty: ident : $no: tt, )*) => {
        impl<$($ty, )*> Valid for ($($ty,)*) where
            $($ty: Valid,)*
        {
            #[inline]
            fn check(&self) -> Result<(), SerializationError> {
                $(self.$no.check()?;)*
                Ok(())
            }
        }

        #[allow(unused)]
        impl<$($ty, )*> CanonicalSerialize for ($($ty,)*) where
            $($ty: CanonicalSerialize,)*
        {
            #[inline]
            fn serialize_with_mode<W: Write>(&self, mut writer: W, compress: Compress) -> Result<(), SerializationError> {
                $(self.$no.serialize_with_mode(&mut writer, compress)?;)*
                Ok(())
            }

            #[inline]
            fn serialized_size(&self, compress: Compress) -> usize {
                [$(
                    self.$no.serialized_size(compress),
                )*].iter().sum()
            }
        }

        impl<$($ty, )*> CanonicalDeserialize for ($($ty,)*) where
            $($ty: CanonicalDeserialize,)*
        {
            #[inline]
            #[allow(unused)]
            fn deserialize_with_mode<R: Read>(mut reader: R, compress: Compress, validate: Validate) -> Result<Self, SerializationError> {
                Ok(($(
                    $ty::deserialize_with_mode(&mut reader, compress, validate)?,
                )*))
            }
        }
    }
}

impl_tuple!();
impl_tuple!(A:0,);
impl_tuple!(A:0, B:1,);
impl_tuple!(A:0, B:1, C:2,);
impl_tuple!(A:0, B:1, C:2, D:3,);

impl<K, V> CanonicalSerialize for BTreeMap<K, V>
where
    K: CanonicalSerialize,
    V: CanonicalSerialize,
{
    /// Serializes a `BTreeMap` as `len(map) || key 1 || value 1 || ... || key n || value n`.
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_with_mode(&mut writer, compress)?;
        for (k, v) in self.iter() {
            k.serialize_with_mode(&mut writer, compress)?;
            v.serialize_with_mode(&mut writer, compress)?;
        }
        Ok(())
    }

    fn serialized_size(&self, compress: Compress) -> usize {
        8 + self
            .iter()
            .map(|(k, v)| k.serialized_size(compress) + v.serialized_size(compress))
            .sum::<usize>()
    }
}

impl<K: Valid, V: Valid> Valid for BTreeMap<K, V> {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        K::batch_check(self.keys())?;
        V::batch_check(self.values())
    }

    #[inline]
    fn batch_check<'a>(batch: impl Iterator<Item = &'a Self>) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        let (keys, values): (Vec<_>, Vec<_>) = batch.map(|b| (b.keys(), b.values())).unzip();
        K::batch_check(keys.into_iter().flatten())?;
        V::batch_check(values.into_iter().flatten())
    }
}

impl<K, V> CanonicalDeserialize for BTreeMap<K, V>
where
    K: Ord + CanonicalDeserialize,
    V: CanonicalDeserialize,
{
    /// Deserializes a `BTreeMap` from `len(map) || key 1 || value 1 || ... || key n || value n`.
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let len = u64::deserialize_with_mode(&mut reader, compress, validate)?;
        (0..len)
            .map(|_| {
                Ok((
                    K::deserialize_with_mode(&mut reader, compress, validate)?,
                    V::deserialize_with_mode(&mut reader, compress, validate)?,
                ))
            })
            .collect()
    }
}

impl<V: CanonicalSerialize> CanonicalSerialize for BTreeSet<V> {
    /// Serializes a `BTreeSet` as `len(set) || value 1 || value 2 || ... || value n`.
    fn serialize_with_mode<W: Write>(
        &self,
        mut writer: W,
        compress: Compress,
    ) -> Result<(), SerializationError> {
        let len = self.len() as u64;
        len.serialize_with_mode(&mut writer, compress)?;
        for v in self {
            v.serialize_with_mode(&mut writer, compress)?;
        }
        Ok(())
    }

    fn serialized_size(&self, compress: Compress) -> usize {
        8 + self
            .iter()
            .map(|v| v.serialized_size(compress))
            .sum::<usize>()
    }
}

impl<V: Valid> Valid for BTreeSet<V> {
    #[inline]
    fn check(&self) -> Result<(), SerializationError> {
        V::batch_check(self.iter())
    }

    #[inline]
    fn batch_check<'a>(
        batch: impl Iterator<Item = &'a Self> + Send,
    ) -> Result<(), SerializationError>
    where
        Self: 'a,
    {
        V::batch_check(batch.flat_map(|s| s.iter()))
    }
}

impl<V> CanonicalDeserialize for BTreeSet<V>
where
    V: Ord + CanonicalDeserialize,
{
    /// Deserializes a `BTreeSet` from `len(map) || value 1 || ... || value n`.
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let len = u64::deserialize_with_mode(&mut reader, compress, validate)?;
        (0..len)
            .map(|_| V::deserialize_with_mode(&mut reader, compress, validate))
            .collect()
    }
}
