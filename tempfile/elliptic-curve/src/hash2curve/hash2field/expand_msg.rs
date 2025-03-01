//! `expand_message` interface `for hash_to_field`.

pub(super) mod xmd;
pub(super) mod xof;

use crate::{Error, Result};
use digest::{Digest, ExtendableOutput, Update, XofReader};
use generic_array::typenum::{IsLess, U256};
use generic_array::{ArrayLength, GenericArray};

/// Salt when the DST is too long
const OVERSIZE_DST_SALT: &[u8] = b"H2C-OVERSIZE-DST-";
/// Maximum domain separation tag length
const MAX_DST_LEN: usize = 255;

/// Trait for types implementing expand_message interface for `hash_to_field`.
///
/// # Errors
/// See implementors of [`ExpandMsg`] for errors.
pub trait ExpandMsg<'a> {
    /// Type holding data for the [`Expander`].
    type Expander: Expander + Sized;

    /// Expands `msg` to the required number of bytes.
    ///
    /// Returns an expander that can be used to call `read` until enough
    /// bytes have been consumed
    fn expand_message(
        msgs: &[&[u8]],
        dsts: &'a [&'a [u8]],
        len_in_bytes: usize,
    ) -> Result<Self::Expander>;
}

/// Expander that, call `read` until enough bytes have been consumed.
pub trait Expander {
    /// Fill the array with the expanded bytes
    fn fill_bytes(&mut self, okm: &mut [u8]);
}

/// The domain separation tag
///
/// Implements [section 5.4.3 of `draft-irtf-cfrg-hash-to-curve-13`][dst].
///
/// [dst]: https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-hash-to-curve-13#section-5.4.3
pub(crate) enum Domain<'a, L>
where
    L: ArrayLength<u8> + IsLess<U256>,
{
    /// > 255
    Hashed(GenericArray<u8, L>),
    /// <= 255
    Array(&'a [&'a [u8]]),
}

impl<'a, L> Domain<'a, L>
where
    L: ArrayLength<u8> + IsLess<U256>,
{
    pub fn xof<X>(dsts: &'a [&'a [u8]]) -> Result<Self>
    where
        X: Default + ExtendableOutput + Update,
    {
        if dsts.is_empty() {
            Err(Error)
        } else if dsts.iter().map(|dst| dst.len()).sum::<usize>() > MAX_DST_LEN {
            let mut data = GenericArray::<u8, L>::default();
            let mut hash = X::default();
            hash.update(OVERSIZE_DST_SALT);

            for dst in dsts {
                hash.update(dst);
            }

            hash.finalize_xof().read(&mut data);

            Ok(Self::Hashed(data))
        } else {
            Ok(Self::Array(dsts))
        }
    }

    pub fn xmd<X>(dsts: &'a [&'a [u8]]) -> Result<Self>
    where
        X: Digest<OutputSize = L>,
    {
        if dsts.is_empty() {
            Err(Error)
        } else if dsts.iter().map(|dst| dst.len()).sum::<usize>() > MAX_DST_LEN {
            Ok(Self::Hashed({
                let mut hash = X::new();
                hash.update(OVERSIZE_DST_SALT);

                for dst in dsts {
                    hash.update(dst);
                }

                hash.finalize()
            }))
        } else {
            Ok(Self::Array(dsts))
        }
    }

    pub fn update_hash<HashT: Update>(&self, hash: &mut HashT) {
        match self {
            Self::Hashed(d) => hash.update(d),
            Self::Array(d) => {
                for d in d.iter() {
                    hash.update(d)
                }
            }
        }
    }

    pub fn len(&self) -> u8 {
        match self {
            // Can't overflow because it's enforced on a type level.
            Self::Hashed(_) => L::to_u8(),
            // Can't overflow because it's checked on creation.
            Self::Array(d) => {
                u8::try_from(d.iter().map(|d| d.len()).sum::<usize>()).expect("length overflow")
            }
        }
    }

    #[cfg(test)]
    pub fn assert(&self, bytes: &[u8]) {
        let data = match self {
            Domain::Hashed(d) => d.to_vec(),
            Domain::Array(d) => d.iter().copied().flatten().copied().collect(),
        };
        assert_eq!(data, bytes);
    }

    #[cfg(test)]
    pub fn assert_dst(&self, bytes: &[u8]) {
        let data = match self {
            Domain::Hashed(d) => d.to_vec(),
            Domain::Array(d) => d.iter().copied().flatten().copied().collect(),
        };
        assert_eq!(data, &bytes[..bytes.len() - 1]);
        assert_eq!(self.len(), bytes[bytes.len() - 1]);
    }
}
