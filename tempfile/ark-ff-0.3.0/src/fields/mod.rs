use crate::{
    biginteger::BigInteger,
    bytes::{FromBytes, ToBytes},
    fields::utils::k_adicity,
    UniformRand,
};
use ark_serialize::{
    CanonicalDeserialize, CanonicalDeserializeWithFlags, CanonicalSerialize,
    CanonicalSerializeWithFlags, EmptyFlags, Flags,
};
use ark_std::{
    cmp::min,
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign},
    str::FromStr,
    vec::Vec,
};

pub use ark_ff_macros;
use num_traits::{One, Zero};
use zeroize::Zeroize;

#[macro_use]
pub mod macros;
pub mod utils;

#[macro_use]
pub mod arithmetic;

pub mod models;
pub use self::models::*;

#[cfg(feature = "parallel")]
use ark_std::cmp::max;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[macro_export]
macro_rules! field_new {
    ($name:ident, $c0:expr) => {{
        use $crate::FpParameters;
        type Params = <$name as $crate::PrimeField>::Params;
        let (is_positive, limbs) = $crate::ark_ff_macros::to_sign_and_limbs!($c0);
        $name::const_from_str(
            &limbs,
            is_positive,
            Params::R2,
            Params::MODULUS,
            Params::INV,
        )
    }};
    ($name:ident, $c0:expr, $c1:expr $(,)?) => {
        $name {
            c0: $c0,
            c1: $c1,
            _parameters: core::marker::PhantomData,
        }
    };
    ($name:ident, $c0:expr, $c1:expr, $c2:expr $(,)?) => {
        $name {
            c0: $c0,
            c1: $c1,
            c2: $c2,
            _parameters: core::marker::PhantomData,
        }
    };
}

/// The interface for a generic field.
pub trait Field:
    ToBytes
    + 'static
    + FromBytes
    + Copy
    + Clone
    + Debug
    + Display
    + Default
    + Send
    + Sync
    + Eq
    + Zero
    + One
    + Ord
    + Neg<Output = Self>
    + UniformRand
    + Zeroize
    + Sized
    + Hash
    + CanonicalSerialize
    + CanonicalSerializeWithFlags
    + CanonicalDeserialize
    + CanonicalDeserializeWithFlags
    + Add<Self, Output = Self>
    + Sub<Self, Output = Self>
    + Mul<Self, Output = Self>
    + Div<Self, Output = Self>
    + AddAssign<Self>
    + SubAssign<Self>
    + MulAssign<Self>
    + DivAssign<Self>
    + for<'a> Add<&'a Self, Output = Self>
    + for<'a> Sub<&'a Self, Output = Self>
    + for<'a> Mul<&'a Self, Output = Self>
    + for<'a> Div<&'a Self, Output = Self>
    + for<'a> AddAssign<&'a Self>
    + for<'a> SubAssign<&'a Self>
    + for<'a> MulAssign<&'a Self>
    + for<'a> DivAssign<&'a Self>
    + core::iter::Sum<Self>
    + for<'a> core::iter::Sum<&'a Self>
    + core::iter::Product<Self>
    + for<'a> core::iter::Product<&'a Self>
    + From<u128>
    + From<u64>
    + From<u32>
    + From<u16>
    + From<u8>
    + From<bool>
{
    type BasePrimeField: PrimeField;

    /// Returns the characteristic of the field,
    /// in little-endian representation.
    fn characteristic() -> &'static [u64] {
        Self::BasePrimeField::characteristic()
    }

    /// Returns the extension degree of this field with respect
    /// to `Self::BasePrimeField`.
    fn extension_degree() -> u64;

    /// Convert a slice of base prime field elements into a field element.
    /// If the slice length != Self::extension_degree(), must return None.
    fn from_base_prime_field_elems(elems: &[Self::BasePrimeField]) -> Option<Self>;

    /// Returns `self + self`.
    #[must_use]
    fn double(&self) -> Self;

    /// Doubles `self` in place.
    fn double_in_place(&mut self) -> &mut Self;

    /// Returns a field element if the set of bytes forms a valid field element,
    /// otherwise returns None. This function is primarily intended for sampling
    /// random field elements from a hash-function or RNG output.
    fn from_random_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_random_bytes_with_flags::<EmptyFlags>(bytes).map(|f| f.0)
    }

    /// Returns a field element with an extra sign bit used for group parsing if
    /// the set of bytes forms a valid field element, otherwise returns
    /// None. This function is primarily intended for sampling
    /// random field elements from a hash-function or RNG output.
    fn from_random_bytes_with_flags<F: Flags>(bytes: &[u8]) -> Option<(Self, F)>;

    /// Returns `self * self`.
    #[must_use]
    fn square(&self) -> Self;

    /// Squares `self` in place.
    fn square_in_place(&mut self) -> &mut Self;

    /// Computes the multiplicative inverse of `self` if `self` is nonzero.
    #[must_use]
    fn inverse(&self) -> Option<Self>;

    // If `self.inverse().is_none()`, this just returns `None`. Otherwise, it sets
    // `self` to `self.inverse().unwrap()`.
    fn inverse_in_place(&mut self) -> Option<&mut Self>;

    /// Exponentiates this element by a power of the base prime modulus via
    /// the Frobenius automorphism.
    fn frobenius_map(&mut self, power: usize);

    /// Exponentiates this element by a number represented with `u64` limbs,
    /// least significant limb first.
    #[must_use]
    fn pow<S: AsRef<[u64]>>(&self, exp: S) -> Self {
        let mut res = Self::one();

        for i in BitIteratorBE::without_leading_zeros(exp) {
            res.square_in_place();

            if i {
                res *= self;
            }
        }
        res
    }

    /// Exponentiates a field element `f` by a number represented with `u64` limbs,
    /// using a precomputed table containing as many powers of 2 of `f`
    /// as the 1 + the floor of log2 of the exponent `exp`, starting from the 1st power.
    /// That is, `powers_of_2` should equal `&[p, p^2, p^4, ..., p^(2^n)]`
    /// when `exp` has at most `n` bits.
    ///
    /// This returns `None` when a power is missing from the table.
    #[inline]
    fn pow_with_table<S: AsRef<[u64]>>(powers_of_2: &[Self], exp: S) -> Option<Self> {
        let mut res = Self::one();
        for (pow, bit) in BitIteratorLE::without_trailing_zeros(exp).enumerate() {
            if bit {
                res *= powers_of_2.get(pow)?;
            }
        }
        Some(res)
    }
}

/// A trait that defines parameters for a field that can be used for FFTs.
pub trait FftParameters: 'static + Send + Sync + Sized {
    type BigInt: BigInteger;

    /// Let `N` be the size of the multiplicative group defined by the field.
    /// Then `TWO_ADICITY` is the two-adicity of `N`, i.e. the integer `s`
    /// such that `N = 2^s * t` for some odd integer `t`.
    const TWO_ADICITY: u32;

    /// 2^s root of unity computed by GENERATOR^t
    const TWO_ADIC_ROOT_OF_UNITY: Self::BigInt;

    /// An integer `b` such that there exists a multiplicative subgroup
    /// of size `b^k` for some integer `k`.
    const SMALL_SUBGROUP_BASE: Option<u32> = None;

    /// The integer `k` such that there exists a multiplicative subgroup
    /// of size `Self::SMALL_SUBGROUP_BASE^k`.
    const SMALL_SUBGROUP_BASE_ADICITY: Option<u32> = None;

    /// GENERATOR^((MODULUS-1) / (2^s *
    /// SMALL_SUBGROUP_BASE^SMALL_SUBGROUP_BASE_ADICITY)) Used for mixed-radix FFT.
    const LARGE_SUBGROUP_ROOT_OF_UNITY: Option<Self::BigInt> = None;
}

/// A trait that defines parameters for a prime field.
pub trait FpParameters: FftParameters {
    /// The modulus of the field.
    const MODULUS: Self::BigInt;

    /// The number of bits needed to represent the `Self::MODULUS`.
    const MODULUS_BITS: u32;

    /// The number of bits that must be shaved from the beginning of
    /// the representation when randomly sampling.
    const REPR_SHAVE_BITS: u32;

    /// Let `M` be the power of 2^64 nearest to `Self::MODULUS_BITS`. Then
    /// `R = M % Self::MODULUS`.
    const R: Self::BigInt;

    /// R2 = R^2 % Self::MODULUS
    const R2: Self::BigInt;

    /// INV = -MODULUS^{-1} mod 2^64
    const INV: u64;

    /// A multiplicative generator of the field.
    /// `Self::GENERATOR` is an element having multiplicative order
    /// `Self::MODULUS - 1`.
    const GENERATOR: Self::BigInt;

    /// The number of bits that can be reliably stored.
    /// (Should equal `SELF::MODULUS_BITS - 1`)
    const CAPACITY: u32;

    /// t for 2^s * t = MODULUS - 1, and t coprime to 2.
    const T: Self::BigInt;

    /// (t - 1) / 2
    const T_MINUS_ONE_DIV_TWO: Self::BigInt;

    /// (Self::MODULUS - 1) / 2
    const MODULUS_MINUS_ONE_DIV_TWO: Self::BigInt;
}

/// The interface for fields that are able to be used in FFTs.
pub trait FftField: Field {
    type FftParams: FftParameters;

    /// Returns the 2^s root of unity.
    fn two_adic_root_of_unity() -> Self;

    /// Returns the 2^s * small_subgroup_base^small_subgroup_base_adicity root of unity
    /// if a small subgroup is defined.
    fn large_subgroup_root_of_unity() -> Option<Self>;

    /// Returns the multiplicative generator of `char()` - 1 order.
    fn multiplicative_generator() -> Self;

    /// Returns the root of unity of order n, if one exists.
    /// If no small multiplicative subgroup is defined, this is the 2-adic root of unity of order n
    /// (for n a power of 2).
    /// If a small multiplicative subgroup is defined, this is the root of unity of order n for
    /// the larger subgroup generated by `FftParams::LARGE_SUBGROUP_ROOT_OF_UNITY`
    /// (for n = 2^i * FftParams::SMALL_SUBGROUP_BASE^j for some i, j).
    fn get_root_of_unity(n: usize) -> Option<Self> {
        let mut omega: Self;
        if let Some(large_subgroup_root_of_unity) = Self::large_subgroup_root_of_unity() {
            let q = Self::FftParams::SMALL_SUBGROUP_BASE.expect(
                "LARGE_SUBGROUP_ROOT_OF_UNITY should only be set in conjunction with SMALL_SUBGROUP_BASE",
            ) as usize;
            let small_subgroup_base_adicity = Self::FftParams::SMALL_SUBGROUP_BASE_ADICITY.expect(
                "LARGE_SUBGROUP_ROOT_OF_UNITY should only be set in conjunction with SMALL_SUBGROUP_BASE_ADICITY",
            );

            let q_adicity = k_adicity(q, n);
            let q_part = q.pow(q_adicity);

            let two_adicity = k_adicity(2, n);
            let two_part = 1 << two_adicity;

            if n != two_part * q_part
                || (two_adicity > Self::FftParams::TWO_ADICITY)
                || (q_adicity > small_subgroup_base_adicity)
            {
                return None;
            }

            omega = large_subgroup_root_of_unity;
            for _ in q_adicity..small_subgroup_base_adicity {
                omega = omega.pow(&[q as u64]);
            }

            for _ in two_adicity..Self::FftParams::TWO_ADICITY {
                omega.square_in_place();
            }
        } else {
            use core::convert::TryFrom;
            // Compute the next power of 2.
            let size = n.next_power_of_two() as u64;
            let log_size_of_group = ark_std::log2(usize::try_from(size).expect("too large"));

            if n != size as usize || log_size_of_group > Self::FftParams::TWO_ADICITY {
                return None;
            }

            // Compute the generator for the multiplicative subgroup.
            // It should be 2^(log_size_of_group) root of unity.
            omega = Self::two_adic_root_of_unity();
            for _ in log_size_of_group..Self::FftParams::TWO_ADICITY {
                omega.square_in_place();
            }
        }
        Some(omega)
    }
}

/// The interface for a prime field.
pub trait PrimeField:
    Field<BasePrimeField = Self>
    + FftField<FftParams = <Self as PrimeField>::Params>
    + FromStr
    + From<<Self as PrimeField>::BigInt>
    + Into<<Self as PrimeField>::BigInt>
    + From<BigUint>
    + Into<BigUint>
{
    type Params: FpParameters<BigInt = Self::BigInt>;
    type BigInt: BigInteger;

    /// Returns a prime field element from its underlying representation.
    fn from_repr(repr: Self::BigInt) -> Option<Self>;

    /// Returns the underlying representation of the prime field element.
    fn into_repr(&self) -> Self::BigInt;

    /// Reads bytes in big-endian, and converts them to a field element.
    /// If the bytes are larger than the modulus, it will reduce them.
    fn from_be_bytes_mod_order(bytes: &[u8]) -> Self {
        let num_modulus_bytes = ((Self::Params::MODULUS_BITS + 7) / 8) as usize;
        let num_bytes_to_directly_convert = min(num_modulus_bytes - 1, bytes.len());
        // Copy the leading big-endian bytes directly into a field element.
        // The number of bytes directly converted must be less than the
        // number of bytes needed to represent the modulus, as we must begin
        // modular reduction once the data is of the same number of bytes as the modulus.
        let mut bytes_to_directly_convert = Vec::new();
        bytes_to_directly_convert.extend(bytes[..num_bytes_to_directly_convert].iter().rev());
        // Guaranteed to not be None, as the input is less than the modulus size.
        let mut res = Self::from_random_bytes(&bytes_to_directly_convert).unwrap();

        // Update the result, byte by byte.
        // We go through existing field arithmetic, which handles the reduction.
        // TODO: If we need higher speeds, parse more bytes at once, or implement
        // modular multiplication by a u64
        let window_size = Self::from(256u64);
        for byte in bytes[num_bytes_to_directly_convert..].iter() {
            res *= window_size;
            res += Self::from(*byte);
        }
        res
    }

    /// Reads bytes in little-endian, and converts them to a field element.
    /// If the bytes are larger than the modulus, it will reduce them.
    fn from_le_bytes_mod_order(bytes: &[u8]) -> Self {
        let mut bytes_copy = bytes.to_vec();
        bytes_copy.reverse();
        Self::from_be_bytes_mod_order(&bytes_copy)
    }

    /// Return the QNR^t, for t defined by
    /// `2^s * t = MODULUS - 1`, and t coprime to 2.
    fn qnr_to_t() -> Self {
        Self::two_adic_root_of_unity()
    }

    /// Returns the field size in bits.
    fn size_in_bits() -> usize {
        Self::Params::MODULUS_BITS as usize
    }

    /// Returns the trace.
    fn trace() -> Self::BigInt {
        Self::Params::T
    }

    /// Returns the trace minus one divided by two.
    fn trace_minus_one_div_two() -> Self::BigInt {
        Self::Params::T_MINUS_ONE_DIV_TWO
    }

    /// Returns the modulus minus one divided by two.
    fn modulus_minus_one_div_two() -> Self::BigInt {
        Self::Params::MODULUS_MINUS_ONE_DIV_TWO
    }
}

/// The interface for a field that supports an efficient square-root operation.
pub trait SquareRootField: Field {
    /// Returns a `LegendreSymbol`, which indicates whether this field element is
    ///  1 : a quadratic residue
    ///  0 : equal to 0
    /// -1 : a quadratic non-residue
    fn legendre(&self) -> LegendreSymbol;

    /// Returns the square root of self, if it exists.
    #[must_use]
    fn sqrt(&self) -> Option<Self>;

    /// Sets `self` to be the square root of `self`, if it exists.
    fn sqrt_in_place(&mut self) -> Option<&mut Self>;
}

#[derive(Debug, PartialEq)]
pub enum LegendreSymbol {
    Zero = 0,
    QuadraticResidue = 1,
    QuadraticNonResidue = -1,
}

impl LegendreSymbol {
    pub fn is_zero(&self) -> bool {
        *self == LegendreSymbol::Zero
    }

    pub fn is_qnr(&self) -> bool {
        *self == LegendreSymbol::QuadraticNonResidue
    }

    pub fn is_qr(&self) -> bool {
        *self == LegendreSymbol::QuadraticResidue
    }
}

/// Iterates over a slice of `u64` in *big-endian* order.
#[derive(Debug)]
pub struct BitIteratorBE<Slice: AsRef<[u64]>> {
    s: Slice,
    n: usize,
}

impl<Slice: AsRef<[u64]>> BitIteratorBE<Slice> {
    pub fn new(s: Slice) -> Self {
        let n = s.as_ref().len() * 64;
        BitIteratorBE { s, n }
    }

    /// Construct an iterator that automatically skips any leading zeros.
    /// That is, it skips all zeros before the most-significant one.
    pub fn without_leading_zeros(s: Slice) -> impl Iterator<Item = bool> {
        Self::new(s).skip_while(|b| !b)
    }
}

impl<Slice: AsRef<[u64]>> Iterator for BitIteratorBE<Slice> {
    type Item = bool;

    fn next(&mut self) -> Option<bool> {
        if self.n == 0 {
            None
        } else {
            self.n -= 1;
            let part = self.n / 64;
            let bit = self.n - (64 * part);

            Some(self.s.as_ref()[part] & (1 << bit) > 0)
        }
    }
}

/// Iterates over a slice of `u64` in *little-endian* order.
#[derive(Debug)]
pub struct BitIteratorLE<Slice: AsRef<[u64]>> {
    s: Slice,
    n: usize,
    max_len: usize,
}

impl<Slice: AsRef<[u64]>> BitIteratorLE<Slice> {
    pub fn new(s: Slice) -> Self {
        let n = 0;
        let max_len = s.as_ref().len() * 64;
        BitIteratorLE { s, n, max_len }
    }

    /// Construct an iterator that automatically skips any trailing zeros.
    /// That is, it skips all zeros after the most-significant one.
    pub fn without_trailing_zeros(s: Slice) -> impl Iterator<Item = bool> {
        let mut first_trailing_zero = 0;
        for (i, limb) in s.as_ref().iter().enumerate().rev() {
            first_trailing_zero = i * 64 + (64 - limb.leading_zeros()) as usize;
            if *limb != 0 {
                break;
            }
        }
        let mut iter = Self::new(s);
        iter.max_len = first_trailing_zero;
        iter
    }
}

impl<Slice: AsRef<[u64]>> Iterator for BitIteratorLE<Slice> {
    type Item = bool;

    fn next(&mut self) -> Option<bool> {
        if self.n == self.max_len {
            None
        } else {
            let part = self.n / 64;
            let bit = self.n - (64 * part);
            self.n += 1;

            Some(self.s.as_ref()[part] & (1 << bit) > 0)
        }
    }
}

use crate::biginteger::{
    BigInteger256, BigInteger320, BigInteger384, BigInteger448, BigInteger64, BigInteger768,
    BigInteger832,
};
use num_bigint::BigUint;

impl_field_bigint_conv!(Fp64, BigInteger64, Fp64Parameters);
impl_field_bigint_conv!(Fp256, BigInteger256, Fp256Parameters);
impl_field_bigint_conv!(Fp320, BigInteger320, Fp320Parameters);
impl_field_bigint_conv!(Fp384, BigInteger384, Fp384Parameters);
impl_field_bigint_conv!(Fp448, BigInteger448, Fp448Parameters);
impl_field_bigint_conv!(Fp768, BigInteger768, Fp768Parameters);
impl_field_bigint_conv!(Fp832, BigInteger832, Fp832Parameters);

// Given a vector of field elements {v_i}, compute the vector {v_i^(-1)}
pub fn batch_inversion<F: Field>(v: &mut [F]) {
    batch_inversion_and_mul(v, &F::one());
}

#[cfg(not(feature = "parallel"))]
// Given a vector of field elements {v_i}, compute the vector {coeff * v_i^(-1)}
pub fn batch_inversion_and_mul<F: Field>(v: &mut [F], coeff: &F) {
    serial_batch_inversion_and_mul(v, coeff);
}

#[cfg(feature = "parallel")]
// Given a vector of field elements {v_i}, compute the vector {coeff * v_i^(-1)}
pub fn batch_inversion_and_mul<F: Field>(v: &mut [F], coeff: &F) {
    // Divide the vector v evenly between all available cores
    let min_elements_per_thread = 1;
    let num_cpus_available = rayon::current_num_threads();
    let num_elems = v.len();
    let num_elem_per_thread = max(num_elems / num_cpus_available, min_elements_per_thread);

    // Batch invert in parallel, without copying the vector
    v.par_chunks_mut(num_elem_per_thread).for_each(|mut chunk| {
        serial_batch_inversion_and_mul(&mut chunk, coeff);
    });
}

/// Given a vector of field elements {v_i}, compute the vector {coeff * v_i^(-1)}
/// This method is explicitly single core.
fn serial_batch_inversion_and_mul<F: Field>(v: &mut [F], coeff: &F) {
    // Montgomery’s Trick and Fast Implementation of Masked AES
    // Genelle, Prouff and Quisquater
    // Section 3.2
    // but with an optimization to multiply every element in the returned vector by coeff

    // First pass: compute [a, ab, abc, ...]
    let mut prod = Vec::with_capacity(v.len());
    let mut tmp = F::one();
    for f in v.iter().filter(|f| !f.is_zero()) {
        tmp.mul_assign(f);
        prod.push(tmp);
    }

    // Invert `tmp`.
    tmp = tmp.inverse().unwrap(); // Guaranteed to be nonzero.

    // Multiply product by coeff, so all inverses will be scaled by coeff
    tmp *= coeff;

    // Second pass: iterate backwards to compute inverses
    for (f, s) in v.iter_mut()
        // Backwards
        .rev()
        // Ignore normalized elements
        .filter(|f| !f.is_zero())
        // Backwards, skip last element, fill in one for last term.
        .zip(prod.into_iter().rev().skip(1).chain(Some(F::one())))
    {
        // tmp := tmp * f; f := tmp * s = 1/f
        let new_tmp = tmp * *f;
        *f = tmp * &s;
        tmp = new_tmp;
    }
}

#[cfg(all(test, feature = "std"))]
mod std_tests {
    use super::BitIteratorLE;

    #[test]
    fn bit_iterator_le() {
        let bits = BitIteratorLE::new(&[0, 1 << 10]).collect::<Vec<_>>();
        dbg!(&bits);
        assert!(bits[74]);
        for (i, bit) in bits.into_iter().enumerate() {
            if i != 74 {
                assert!(!bit)
            } else {
                assert!(bit)
            }
        }
    }
}

#[cfg(test)]
mod no_std_tests {
    use super::*;
    use crate::test_field::{Fr, FrParameters};
    use ark_std::test_rng;

    #[test]
    fn test_batch_inversion() {
        let mut random_coeffs = Vec::<Fr>::new();
        let vec_size = 1000;

        for _ in 0..=vec_size {
            random_coeffs.push(Fr::rand(&mut test_rng()));
        }

        let mut random_coeffs_inv = random_coeffs.clone();
        batch_inversion::<Fr>(&mut random_coeffs_inv);
        for i in 0..=vec_size {
            assert_eq!(random_coeffs_inv[i] * random_coeffs[i], Fr::one());
        }
        let rand_multiplier = Fr::rand(&mut test_rng());
        let mut random_coeffs_inv_shifted = random_coeffs.clone();
        batch_inversion_and_mul(&mut random_coeffs_inv_shifted, &rand_multiplier);
        for i in 0..=vec_size {
            assert_eq!(
                random_coeffs_inv_shifted[i] * random_coeffs[i],
                rand_multiplier
            );
        }
    }

    #[test]
    fn test_from_into_biguint() {
        let mut rng = ark_std::test_rng();

        let modulus_bits = FrParameters::MODULUS_BITS;
        let modulus: num_bigint::BigUint = FrParameters::MODULUS.into();

        let mut rand_bytes = Vec::new();
        for _ in 0..(2 * modulus_bits / 8) {
            rand_bytes.push(u8::rand(&mut rng));
        }

        let rand = BigUint::from_bytes_le(&rand_bytes);

        let a: BigUint = Fr::from(rand.clone()).into();
        let b = rand % modulus;

        assert_eq!(a, b);
    }

    #[test]
    fn test_from_be_bytes_mod_order() {
        // Each test vector is a byte array,
        // and its tested by parsing it with from_bytes_mod_order, and the num-bigint library.
        // The bytes are currently generated from scripts/test_vectors.py.
        // TODO: Eventually generate all the test vector bytes via computation with the modulus
        use ark_std::rand::Rng;
        use ark_std::string::ToString;
        use num_bigint::BigUint;

        let ref_modulus =
            BigUint::from_bytes_be(&<Fr as PrimeField>::Params::MODULUS.to_bytes_be());

        let mut test_vectors = vec![
            // 0
            vec![0u8],
            // 1
            vec![1u8],
            // 255
            vec![255u8],
            // 256
            vec![1u8, 0u8],
            // 65791
            vec![1u8, 0u8, 255u8],
            // 204827637402836681560342736360101429053478720705186085244545541796635082752
            vec![
                115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8, 9u8,
                161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 0u8,
            ],
            // 204827637402836681560342736360101429053478720705186085244545541796635082753
            vec![
                115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8, 9u8,
                161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 1u8,
            ],
            // 52435875175126190479447740508185965837690552500527637822603658699938581184512
            vec![
                115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8, 9u8,
                161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 0u8, 0u8,
            ],
            // 52435875175126190479447740508185965837690552500527637822603658699938581184513
            vec![
                115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8, 9u8,
                161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 0u8, 1u8,
            ],
            // 52435875175126190479447740508185965837690552500527637822603658699938581184514
            vec![
                115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8, 9u8,
                161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 0u8, 2u8,
            ],
            // 104871750350252380958895481016371931675381105001055275645207317399877162369026
            vec![
                231u8, 219u8, 78u8, 166u8, 83u8, 58u8, 250u8, 144u8, 102u8, 115u8, 176u8, 16u8,
                19u8, 67u8, 176u8, 10u8, 167u8, 123u8, 72u8, 5u8, 255u8, 252u8, 183u8, 253u8,
                255u8, 255u8, 255u8, 254u8, 0u8, 0u8, 0u8, 2u8,
            ],
            // 13423584044832304762738621570095607254448781440135075282586536627184276783235328
            vec![
                115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8, 9u8,
                161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 0u8, 1u8, 0u8,
            ],
            // 115792089237316195423570985008687907853269984665640564039457584007913129639953
            vec![
                1u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
                0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
                17u8,
            ],
            // 168227964412442385903018725516873873690960537166168201862061242707851710824468
            vec![
                1u8, 115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8, 8u8,
                9u8, 161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8, 255u8,
                255u8, 255u8, 255u8, 0u8, 0u8, 0u8, 20u8,
            ],
            // 29695210719928072218913619902732290376274806626904512031923745164725699769008210
            vec![
                1u8, 0u8, 115u8, 237u8, 167u8, 83u8, 41u8, 157u8, 125u8, 72u8, 51u8, 57u8, 216u8,
                8u8, 9u8, 161u8, 216u8, 5u8, 83u8, 189u8, 164u8, 2u8, 255u8, 254u8, 91u8, 254u8,
                255u8, 255u8, 255u8, 255u8, 0u8, 0u8, 0u8, 82u8,
            ],
        ];
        // Add random bytestrings to the test vector list
        for i in 1..512 {
            let mut rng = test_rng();
            let data: Vec<u8> = (0..i).map(|_| rng.gen()).collect();
            test_vectors.push(data);
        }
        for i in test_vectors {
            let mut expected_biguint = BigUint::from_bytes_be(&i);
            // Reduce expected_biguint using modpow API
            expected_biguint =
                expected_biguint.modpow(&BigUint::from_bytes_be(&[1u8]), &ref_modulus);
            let expected_string = expected_biguint.to_string();
            let expected = Fr::from_str(&expected_string).unwrap();
            let actual = Fr::from_be_bytes_mod_order(&i);
            assert_eq!(expected, actual, "failed on test {:?}", i);
        }
    }
}
