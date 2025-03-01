use crate::UniformRand;
use ark_serialize::{
    CanonicalDeserialize, CanonicalDeserializeWithFlags, CanonicalSerialize,
    CanonicalSerializeWithFlags, EmptyFlags, Flags,
};
use ark_std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign},
    vec::Vec,
};

pub use ark_ff_macros;
use num_traits::{One, Zero};
use zeroize::Zeroize;

pub mod utils;

#[macro_use]
pub mod arithmetic;

#[macro_use]
pub mod models;
pub use self::models::*;

pub mod field_hashers;

mod prime;
pub use prime::*;

mod fft_friendly;
pub use fft_friendly::*;

mod cyclotomic;
pub use cyclotomic::*;

mod sqrt;
pub use sqrt::*;

#[cfg(feature = "parallel")]
use ark_std::cmp::max;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// The interface for a generic field.  
/// Types implementing [`Field`] support common field operations such as addition, subtraction, multiplication, and inverses.
///
/// ## Defining your own field
/// To demonstrate the various field operations, we can first define a prime ordered field $\mathbb{F}_{p}$ with $p = 17$. When defining a field $\mathbb{F}_p$, we need to provide the modulus(the $p$ in $\mathbb{F}_p$) and a generator. Recall that a generator $g \in \mathbb{F}_p$ is a field element whose powers comprise the entire field: $\mathbb{F}_p =\\{g, g^1, \ldots, g^{p-1}\\}$.
/// We can then manually construct the field element associated with an integer with `Fp::from` and perform field addition, subtraction, multiplication, and inversion on it.
/// ```rust
/// use ark_ff::fields::{Field, Fp64, MontBackend, MontConfig};
///
/// #[derive(MontConfig)]
/// #[modulus = "17"]
/// #[generator = "3"]
/// pub struct FqConfig;
/// pub type Fq = Fp64<MontBackend<FqConfig, 1>>;
///
/// # fn main() {
/// let a = Fq::from(9);
/// let b = Fq::from(10);
///
/// assert_eq!(a, Fq::from(26));          // 26 =  9 mod 17
/// assert_eq!(a - b, Fq::from(16));      // -1 = 16 mod 17
/// assert_eq!(a + b, Fq::from(2));       // 19 =  2 mod 17
/// assert_eq!(a * b, Fq::from(5));       // 90 =  5 mod 17
/// assert_eq!(a.square(), Fq::from(13)); // 81 = 13 mod 17
/// assert_eq!(b.double(), Fq::from(3));  // 20 =  3 mod 17
/// assert_eq!(a / b, a * b.inverse().unwrap()); // need to unwrap since `b` could be 0 which is not invertible
/// # }
/// ```
///
/// ## Using pre-defined fields
/// In the following example, we’ll use the field associated with the BLS12-381 pairing-friendly group.
/// ```rust
/// use ark_ff::Field;
/// use ark_test_curves::bls12_381::Fq as F;
/// use ark_std::{One, UniformRand, test_rng};
///
/// let mut rng = test_rng();
/// // Let's sample uniformly random field elements:
/// let a = F::rand(&mut rng);
/// let b = F::rand(&mut rng);
///
/// let c = a + b;
/// let d = a - b;
/// assert_eq!(c + d, a.double());
///
/// let e = c * d;
/// assert_eq!(e, a.square() - b.square());         // (a + b)(a - b) = a^2 - b^2
/// assert_eq!(a.inverse().unwrap() * a, F::one()); // Euler-Fermat theorem tells us: a * a^{-1} = 1 mod p
/// ```
pub trait Field:
    'static
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
    + for<'a> Add<&'a mut Self, Output = Self>
    + for<'a> Sub<&'a mut Self, Output = Self>
    + for<'a> Mul<&'a mut Self, Output = Self>
    + for<'a> Div<&'a mut Self, Output = Self>
    + for<'a> AddAssign<&'a mut Self>
    + for<'a> SubAssign<&'a mut Self>
    + for<'a> MulAssign<&'a mut Self>
    + for<'a> DivAssign<&'a mut Self>
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

    type BasePrimeFieldIter: Iterator<Item = Self::BasePrimeField>;

    /// Determines the algorithm for computing square roots.
    const SQRT_PRECOMP: Option<SqrtPrecomputation<Self>>;

    /// The additive identity of the field.
    const ZERO: Self;
    /// The multiplicative identity of the field.
    const ONE: Self;

    /// Returns the characteristic of the field,
    /// in little-endian representation.
    fn characteristic() -> &'static [u64] {
        Self::BasePrimeField::characteristic()
    }

    /// Returns the extension degree of this field with respect
    /// to `Self::BasePrimeField`.
    fn extension_degree() -> u64;

    fn to_base_prime_field_elements(&self) -> Self::BasePrimeFieldIter;

    /// Convert a slice of base prime field elements into a field element.
    /// If the slice length != Self::extension_degree(), must return None.
    fn from_base_prime_field_elems(elems: &[Self::BasePrimeField]) -> Option<Self>;

    /// Constructs a field element from a single base prime field elements.
    /// ```
    /// # use ark_ff::Field;
    /// # use ark_test_curves::bls12_381::Fq as F;
    /// # use ark_test_curves::bls12_381::Fq2 as F2;
    /// # use ark_std::One;
    /// assert_eq!(F2::from_base_prime_field(F::one()), F2::one());
    /// ```
    fn from_base_prime_field(elem: Self::BasePrimeField) -> Self;

    /// Returns `self + self`.
    #[must_use]
    fn double(&self) -> Self;

    /// Doubles `self` in place.
    fn double_in_place(&mut self) -> &mut Self;

    /// Negates `self` in place.
    fn neg_in_place(&mut self) -> &mut Self;

    /// Attempt to deserialize a field element. Returns `None` if the
    /// deserialization fails.
    ///
    /// This function is primarily intended for sampling random field elements
    /// from a hash-function or RNG output.
    fn from_random_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_random_bytes_with_flags::<EmptyFlags>(bytes).map(|f| f.0)
    }

    /// Attempt to deserialize a field element, splitting the bitflags metadata
    /// according to `F` specification. Returns `None` if the deserialization
    /// fails.
    ///
    /// This function is primarily intended for sampling random field elements
    /// from a hash-function or RNG output.
    fn from_random_bytes_with_flags<F: Flags>(bytes: &[u8]) -> Option<(Self, F)>;

    /// Returns a `LegendreSymbol`, which indicates whether this field element
    /// is  1 : a quadratic residue
    ///  0 : equal to 0
    /// -1 : a quadratic non-residue
    fn legendre(&self) -> LegendreSymbol;

    /// Returns the square root of self, if it exists.
    #[must_use]
    fn sqrt(&self) -> Option<Self> {
        match Self::SQRT_PRECOMP {
            Some(tv) => tv.sqrt(self),
            None => unimplemented!(),
        }
    }

    /// Sets `self` to be the square root of `self`, if it exists.
    fn sqrt_in_place(&mut self) -> Option<&mut Self> {
        (*self).sqrt().map(|sqrt| {
            *self = sqrt;
            self
        })
    }

    /// Returns `self * self`.
    #[must_use]
    fn square(&self) -> Self;

    /// Squares `self` in place.
    fn square_in_place(&mut self) -> &mut Self;

    /// Computes the multiplicative inverse of `self` if `self` is nonzero.
    #[must_use]
    fn inverse(&self) -> Option<Self>;

    /// If `self.inverse().is_none()`, this just returns `None`. Otherwise, it sets
    /// `self` to `self.inverse().unwrap()`.
    fn inverse_in_place(&mut self) -> Option<&mut Self>;

    /// Returns `sum([a_i * b_i])`.
    #[inline]
    fn sum_of_products<const T: usize>(a: &[Self; T], b: &[Self; T]) -> Self {
        let mut sum = Self::zero();
        for i in 0..a.len() {
            sum += a[i] * b[i];
        }
        sum
    }

    /// Sets `self` to `self^s`, where `s = Self::BasePrimeField::MODULUS^power`.
    /// This is also called the Frobenius automorphism.
    fn frobenius_map_in_place(&mut self, power: usize);

    /// Returns `self^s`, where `s = Self::BasePrimeField::MODULUS^power`.
    /// This is also called the Frobenius automorphism.
    #[must_use]
    fn frobenius_map(&self, power: usize) -> Self {
        let mut this = *self;
        this.frobenius_map_in_place(power);
        this
    }

    /// Returns `self^exp`, where `exp` is an integer represented with `u64` limbs,
    /// least significant limb first.
    #[must_use]
    fn pow<S: AsRef<[u64]>>(&self, exp: S) -> Self {
        let mut res = Self::one();

        for i in crate::BitIteratorBE::without_leading_zeros(exp) {
            res.square_in_place();

            if i {
                res *= self;
            }
        }
        res
    }

    /// Exponentiates a field element `f` by a number represented with `u64`
    /// limbs, using a precomputed table containing as many powers of 2 of
    /// `f` as the 1 + the floor of log2 of the exponent `exp`, starting
    /// from the 1st power. That is, `powers_of_2` should equal `&[p, p^2,
    /// p^4, ..., p^(2^n)]` when `exp` has at most `n` bits.
    ///
    /// This returns `None` when a power is missing from the table.
    #[inline]
    fn pow_with_table<S: AsRef<[u64]>>(powers_of_2: &[Self], exp: S) -> Option<Self> {
        let mut res = Self::one();
        for (pow, bit) in crate::BitIteratorLE::without_trailing_zeros(exp).enumerate() {
            if bit {
                res *= powers_of_2.get(pow)?;
            }
        }
        Some(res)
    }
}

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

/// Given a vector of field elements {v_i}, compute the vector {coeff * v_i^(-1)}.
/// This method is explicitly single-threaded.
fn serial_batch_inversion_and_mul<F: Field>(v: &mut [F], coeff: &F) {
    // Montgomery’s Trick and Fast Implementation of Masked AES
    // Genelle, Prouff and Quisquater
    // Section 3.2
    // but with an optimization to multiply every element in the returned vector by
    // coeff

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
    use crate::BitIteratorLE;

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
    use ark_std::{str::FromStr, test_rng};
    use num_bigint::*;

    // TODO: only Fr & FrConfig should need to be imported.
    // The rest of imports are caused by cargo not resolving the deps properly
    // from this crate and from ark_test_curves
    use ark_test_curves::{batch_inversion, batch_inversion_and_mul, bls12_381::Fr, PrimeField};

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

        let modulus_bits = Fr::MODULUS_BIT_SIZE;
        let modulus: num_bigint::BigUint = Fr::MODULUS.into();

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
        // and its tested by parsing it with from_bytes_mod_order, and the num-bigint
        // library. The bytes are currently generated from scripts/test_vectors.py.
        // TODO: Eventually generate all the test vector bytes via computation with the
        // modulus
        use ark_std::{rand::Rng, string::ToString};
        use ark_test_curves::BigInteger;
        use num_bigint::BigUint;

        let ref_modulus = BigUint::from_bytes_be(&Fr::MODULUS.to_bytes_be());

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
