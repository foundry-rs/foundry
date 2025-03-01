use crate::{BigInteger, FftField, Field};

use ark_std::{cmp::min, str::FromStr};
use num_bigint::BigUint;

/// The interface for a prime field, i.e. the field of integers modulo a prime $p$.  
/// In the following example we'll use the prime field underlying the BLS12-381 G1 curve.
/// ```rust
/// use ark_ff::{BigInteger, Field, PrimeField};
/// use ark_std::{test_rng, One, UniformRand, Zero};
/// use ark_test_curves::bls12_381::Fq as F;
///
/// let mut rng = test_rng();
/// let a = F::rand(&mut rng);
/// // We can access the prime modulus associated with `F`:
/// let modulus = <F as PrimeField>::MODULUS;
/// assert_eq!(a.pow(&modulus), a); // the Euler-Fermat theorem tells us: a^{p-1} = 1 mod p
///
/// // We can convert field elements to integers in the range [0, MODULUS - 1]:
/// let one: num_bigint::BigUint = F::one().into();
/// assert_eq!(one, num_bigint::BigUint::one());
///
/// // We can construct field elements from an arbitrary sequence of bytes:
/// let n = F::from_le_bytes_mod_order(&modulus.to_bytes_le());
/// assert_eq!(n, F::zero());
/// ```
pub trait PrimeField:
    Field<BasePrimeField = Self>
    + FftField
    + FromStr
    + From<<Self as PrimeField>::BigInt>
    + Into<<Self as PrimeField>::BigInt>
    + From<BigUint>
    + Into<BigUint>
{
    /// A `BigInteger` type that can represent elements of this field.
    type BigInt: BigInteger;

    /// The modulus `p`.
    const MODULUS: Self::BigInt;

    /// The value `(p - 1)/ 2`.
    const MODULUS_MINUS_ONE_DIV_TWO: Self::BigInt;

    /// The size of the modulus in bits.
    const MODULUS_BIT_SIZE: u32;

    /// The trace of the field is defined as the smallest integer `t` such that by
    /// `2^s * t = p - 1`, and `t` is coprime to 2.
    const TRACE: Self::BigInt;
    /// The value `(t - 1)/ 2`.
    const TRACE_MINUS_ONE_DIV_TWO: Self::BigInt;

    /// Construct a prime field element from an integer in the range 0..(p - 1).
    fn from_bigint(repr: Self::BigInt) -> Option<Self>;

    /// Converts an element of the prime field into an integer in the range 0..(p - 1).
    fn into_bigint(self) -> Self::BigInt;

    /// Reads bytes in big-endian, and converts them to a field element.
    /// If the integer represented by `bytes` is larger than the modulus `p`, this method
    /// performs the appropriate reduction.
    fn from_be_bytes_mod_order(bytes: &[u8]) -> Self {
        let mut bytes_copy = bytes.to_vec();
        bytes_copy.reverse();
        Self::from_le_bytes_mod_order(&bytes_copy)
    }

    /// Reads bytes in little-endian, and converts them to a field element.
    /// If the integer represented by `bytes` is larger than the modulus `p`, this method
    /// performs the appropriate reduction.
    fn from_le_bytes_mod_order(bytes: &[u8]) -> Self {
        let num_modulus_bytes = ((Self::MODULUS_BIT_SIZE + 7) / 8) as usize;
        let num_bytes_to_directly_convert = min(num_modulus_bytes - 1, bytes.len());
        // Copy the leading little-endian bytes directly into a field element.
        // The number of bytes directly converted must be less than the
        // number of bytes needed to represent the modulus, as we must begin
        // modular reduction once the data is of the same number of bytes as the
        // modulus.
        let (bytes, bytes_to_directly_convert) =
            bytes.split_at(bytes.len() - num_bytes_to_directly_convert);
        // Guaranteed to not be None, as the input is less than the modulus size.
        let mut res = Self::from_random_bytes(&bytes_to_directly_convert).unwrap();

        // Update the result, byte by byte.
        // We go through existing field arithmetic, which handles the reduction.
        // TODO: If we need higher speeds, parse more bytes at once, or implement
        // modular multiplication by a u64
        let window_size = Self::from(256u64);
        for byte in bytes.iter().rev() {
            res *= window_size;
            res += Self::from(*byte);
        }
        res
    }
}
