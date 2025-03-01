//! CTR mode flavors

use cipher::{
    generic_array::{ArrayLength, GenericArray},
    Counter,
};

mod ctr128;
mod ctr32;
mod ctr64;

pub use ctr128::{Ctr128BE, Ctr128LE};
pub use ctr32::{Ctr32BE, Ctr32LE};
pub use ctr64::{Ctr64BE, Ctr64LE};

/// Trait implemented by different CTR flavors.
pub trait CtrFlavor<B: ArrayLength<u8>> {
    /// Inner representation of nonce.
    type CtrNonce: Clone;
    /// Backend numeric type
    type Backend: Counter;
    /// Flavor name
    const NAME: &'static str;

    /// Return number of remaining blocks.
    ///
    /// If result does not fit into `usize`, returns `None`.
    fn remaining(cn: &Self::CtrNonce) -> Option<usize>;

    /// Generate block for given `nonce` and current counter value.
    fn next_block(cn: &mut Self::CtrNonce) -> GenericArray<u8, B>;

    /// Generate block for given `nonce` and current counter value.
    fn current_block(cn: &Self::CtrNonce) -> GenericArray<u8, B>;

    /// Initialize from bytes.
    fn from_nonce(block: &GenericArray<u8, B>) -> Self::CtrNonce;

    /// Convert from a backend value
    fn set_from_backend(cn: &mut Self::CtrNonce, v: Self::Backend);

    /// Convert to a backend value
    fn as_backend(cn: &Self::CtrNonce) -> Self::Backend;
}
