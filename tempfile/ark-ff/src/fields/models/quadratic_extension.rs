use ark_serialize::{
    CanonicalDeserialize, CanonicalDeserializeWithFlags, CanonicalSerialize,
    CanonicalSerializeWithFlags, Compress, EmptyFlags, Flags, SerializationError, Valid, Validate,
};
use ark_std::{
    cmp::{Ord, Ordering, PartialOrd},
    fmt,
    io::{Read, Write},
    iter::Chain,
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign},
    vec::Vec,
};

use num_traits::{One, Zero};
use zeroize::Zeroize;

use ark_std::rand::{
    distributions::{Distribution, Standard},
    Rng,
};

use crate::{
    biginteger::BigInteger,
    fields::{Field, LegendreSymbol, PrimeField},
    SqrtPrecomputation, ToConstraintField, UniformRand,
};

/// Defines a Quadratic extension field from a quadratic non-residue.
pub trait QuadExtConfig: 'static + Send + Sync + Sized {
    /// The prime field that this quadratic extension is eventually an extension of.
    type BasePrimeField: PrimeField;
    /// The base field that this field is a quadratic extension of.
    ///
    /// Note: while for simple instances of quadratic extensions such as `Fp2`
    /// we might see `BaseField == BasePrimeField`, it won't always hold true.
    /// E.g. for an extension tower: `BasePrimeField == Fp`, but `BaseField == Fp3`.
    type BaseField: Field<BasePrimeField = Self::BasePrimeField>;
    /// The type of the coefficients for an efficient implemntation of the
    /// Frobenius endomorphism.
    type FrobCoeff: Field;

    /// The degree of the extension over the base prime field.
    const DEGREE_OVER_BASE_PRIME_FIELD: usize;

    /// The quadratic non-residue used to construct the extension.
    const NONRESIDUE: Self::BaseField;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff];

    /// A specializable method for multiplying an element of the base field by
    /// the quadratic non-residue. This is used in Karatsuba multiplication
    /// and in complex squaring.
    #[inline(always)]
    fn mul_base_field_by_nonresidue_in_place(fe: &mut Self::BaseField) -> &mut Self::BaseField {
        *fe *= &Self::NONRESIDUE;
        fe
    }

    /// A specializable method for setting `y = x + NONRESIDUE * y`.
    /// This allows for optimizations when the non-residue is
    /// canonically negative in the field.
    #[inline(always)]
    fn mul_base_field_by_nonresidue_and_add(y: &mut Self::BaseField, x: &Self::BaseField) {
        Self::mul_base_field_by_nonresidue_in_place(y);
        *y += x;
    }

    /// A specializable method for computing x + mul_base_field_by_nonresidue(y) + y
    /// This allows for optimizations when the non-residue is not -1.
    #[inline(always)]
    fn mul_base_field_by_nonresidue_plus_one_and_add(y: &mut Self::BaseField, x: &Self::BaseField) {
        let old_y = *y;
        Self::mul_base_field_by_nonresidue_and_add(y, x);
        *y += old_y;
    }

    /// A specializable method for computing x - mul_base_field_by_nonresidue(y)
    /// This allows for optimizations when the non-residue is
    /// canonically negative in the field.
    #[inline(always)]
    fn sub_and_mul_base_field_by_nonresidue(y: &mut Self::BaseField, x: &Self::BaseField) {
        Self::mul_base_field_by_nonresidue_in_place(y);
        let mut result = *x;
        result -= &*y;
        *y = result;
    }

    /// A specializable method for multiplying an element of the base field by
    /// the appropriate Frobenius coefficient.
    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize);
}

/// An element of a quadratic extension field F_p\[X\]/(X^2 - P::NONRESIDUE) is
/// represented as c0 + c1 * X, for c0, c1 in `P::BaseField`.
#[derive(Derivative)]
#[derivative(
    Default(bound = "P: QuadExtConfig"),
    Hash(bound = "P: QuadExtConfig"),
    Clone(bound = "P: QuadExtConfig"),
    Copy(bound = "P: QuadExtConfig"),
    Debug(bound = "P: QuadExtConfig"),
    PartialEq(bound = "P: QuadExtConfig"),
    Eq(bound = "P: QuadExtConfig")
)]
pub struct QuadExtField<P: QuadExtConfig> {
    /// Coefficient `c0` in the representation of the field element `c = c0 + c1 * X`
    pub c0: P::BaseField,
    /// Coefficient `c1` in the representation of the field element `c = c0 + c1 * X`
    pub c1: P::BaseField,
}

impl<P: QuadExtConfig> QuadExtField<P> {
    /// Create a new field element from coefficients `c0` and `c1`,
    /// so that the result is of the form `c0 + c1 * X`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ark_std::test_rng;
    /// # use ark_test_curves::bls12_381::{Fq as Fp, Fq2 as Fp2};
    /// # use ark_std::UniformRand;
    /// let c0: Fp = Fp::rand(&mut test_rng());
    /// let c1: Fp = Fp::rand(&mut test_rng());
    /// // `Fp2` a degree-2 extension over `Fp`.
    /// let c: Fp2 = Fp2::new(c0, c1);
    /// ```
    pub const fn new(c0: P::BaseField, c1: P::BaseField) -> Self {
        Self { c0, c1 }
    }

    /// This is only to be used when the element is *known* to be in the
    /// cyclotomic subgroup.
    pub fn conjugate_in_place(&mut self) -> &mut Self {
        self.c1 = -self.c1;
        self
    }

    /// Norm of QuadExtField over `P::BaseField`:`Norm(a) = a * a.conjugate()`.
    /// This simplifies to: `Norm(a) = a.x^2 - P::NON_RESIDUE * a.y^2`.
    /// This is alternatively expressed as `Norm(a) = a^(1 + p)`.
    ///
    /// # Examples
    /// ```
    /// # use ark_std::test_rng;
    /// # use ark_std::{UniformRand, Zero};
    /// # use ark_test_curves::{Field, bls12_381::Fq2 as Fp2};
    /// let c: Fp2 = Fp2::rand(&mut test_rng());
    /// let norm = c.norm();
    /// // We now compute the norm using the `a * a.conjugate()` approach.
    /// // A Frobenius map sends an element of `Fp2` to one of its p_th powers:
    /// // `a.frobenius_map_in_place(1) -> a^p` and `a^p` is also `a`'s Galois conjugate.
    /// let mut c_conjugate = c;
    /// c_conjugate.frobenius_map_in_place(1);
    /// let norm2 = c * c_conjugate;
    /// // Computing the norm of an `Fp2` element should result in an element
    /// // in BaseField `Fp`, i.e. `c1 == 0`
    /// assert!(norm2.c1.is_zero());
    /// assert_eq!(norm, norm2.c0);
    /// ```
    pub fn norm(&self) -> P::BaseField {
        // t1 = c0.square() - P::NON_RESIDUE * c1^2
        let mut result = self.c1.square();
        P::sub_and_mul_base_field_by_nonresidue(&mut result, &self.c0.square());
        result
    }

    /// In-place multiply both coefficients `c0` & `c1` of the quadratic
    /// extension field by an element from the base field.
    pub fn mul_assign_by_basefield(&mut self, element: &P::BaseField) {
        self.c0 *= element;
        self.c1 *= element;
    }
}

impl<P: QuadExtConfig> Zero for QuadExtField<P> {
    fn zero() -> Self {
        QuadExtField::new(P::BaseField::zero(), P::BaseField::zero())
    }

    fn is_zero(&self) -> bool {
        self.c0.is_zero() && self.c1.is_zero()
    }
}

impl<P: QuadExtConfig> One for QuadExtField<P> {
    fn one() -> Self {
        QuadExtField::new(P::BaseField::one(), P::BaseField::zero())
    }

    fn is_one(&self) -> bool {
        self.c0.is_one() && self.c1.is_zero()
    }
}

type BaseFieldIter<P> = <<P as QuadExtConfig>::BaseField as Field>::BasePrimeFieldIter;
impl<P: QuadExtConfig> Field for QuadExtField<P> {
    type BasePrimeField = P::BasePrimeField;

    type BasePrimeFieldIter = Chain<BaseFieldIter<P>, BaseFieldIter<P>>;

    const SQRT_PRECOMP: Option<SqrtPrecomputation<Self>> = None;

    const ZERO: Self = Self::new(P::BaseField::ZERO, P::BaseField::ZERO);
    const ONE: Self = Self::new(P::BaseField::ONE, P::BaseField::ZERO);

    fn extension_degree() -> u64 {
        2 * P::BaseField::extension_degree()
    }

    fn from_base_prime_field(elem: Self::BasePrimeField) -> Self {
        let fe = P::BaseField::from_base_prime_field(elem);
        Self::new(fe, P::BaseField::ZERO)
    }

    fn to_base_prime_field_elements(&self) -> Self::BasePrimeFieldIter {
        self.c0
            .to_base_prime_field_elements()
            .chain(self.c1.to_base_prime_field_elements())
    }

    fn from_base_prime_field_elems(elems: &[Self::BasePrimeField]) -> Option<Self> {
        if elems.len() != (Self::extension_degree() as usize) {
            return None;
        }
        let base_ext_deg = P::BaseField::extension_degree() as usize;
        Some(Self::new(
            P::BaseField::from_base_prime_field_elems(&elems[0..base_ext_deg]).unwrap(),
            P::BaseField::from_base_prime_field_elems(&elems[base_ext_deg..]).unwrap(),
        ))
    }

    fn double(&self) -> Self {
        let mut result = *self;
        result.double_in_place();
        result
    }

    fn double_in_place(&mut self) -> &mut Self {
        self.c0.double_in_place();
        self.c1.double_in_place();
        self
    }

    fn neg_in_place(&mut self) -> &mut Self {
        self.c0.neg_in_place();
        self.c1.neg_in_place();
        self
    }

    fn square(&self) -> Self {
        let mut result = *self;
        result.square_in_place();
        result
    }

    #[inline]
    fn from_random_bytes_with_flags<F: Flags>(bytes: &[u8]) -> Option<(Self, F)> {
        let split_at = bytes.len() / 2;
        if let Some(c0) = P::BaseField::from_random_bytes(&bytes[..split_at]) {
            if let Some((c1, flags)) =
                P::BaseField::from_random_bytes_with_flags(&bytes[split_at..])
            {
                return Some((QuadExtField::new(c0, c1), flags));
            }
        }
        None
    }

    #[inline]
    fn from_random_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_random_bytes_with_flags::<EmptyFlags>(bytes).map(|f| f.0)
    }

    fn square_in_place(&mut self) -> &mut Self {
        // (c0, c1)^2 = (c0 + x*c1)^2
        //            = c0^2 + 2 c0 c1 x + c1^2 x^2
        //            = c0^2 + beta * c1^2 + 2 c0 * c1 * x
        //            = (c0^2 + beta * c1^2, 2 c0 * c1)
        // Where beta is P::NONRESIDUE.
        // When beta = -1, we can re-use intermediate additions to improve performance.
        if P::NONRESIDUE == -P::BaseField::ONE {
            // When the non-residue is -1, we save 2 intermediate additions,
            // and use one fewer intermediate variable

            let c0_copy = self.c0;
            // v0 = c0 - c1
            let mut v0 = self.c0;
            v0 -= &self.c1;
            self.c0 += self.c1;
            // result.c0 *= (c0 - c1)
            // result.c0 = (c0 - c1) * (c0 + c1) = c0^2 - c1^2
            self.c0 *= &v0;

            // result.c1 = 2 c1
            self.c1.double_in_place();
            // result.c1 *= c0
            // result.c1 = (2 * c1) * c0
            self.c1 *= &c0_copy;

            self
        } else {
            // v0 = c0 - c1
            let mut v0 = self.c0 - &self.c1;
            // v3 = c0 - beta * c1
            let mut v3 = self.c1;
            P::sub_and_mul_base_field_by_nonresidue(&mut v3, &self.c0);
            // v2 = c0 * c1
            let v2 = self.c0 * &self.c1;

            // v0 = (v0 * v3)
            // v0 = (c0 - c1) * (c0 - beta*c1)
            // v0 = c0^2 - beta * c0 * c1 - c0 * c1 + beta * c1^2
            v0 *= &v3;

            // result.c1 = 2 * c0 * c1
            self.c1 = v2;
            self.c1.double_in_place();
            // result.c0 = (c0^2 - beta * c0 * c1 - c0 * c1 + beta * c1^2) + ((beta + 1) c0 * c1)
            // result.c0 = (c0^2 - beta * c0 * c1 + beta * c1^2) + (beta * c0 * c1)
            // result.c0 = c0^2 + beta * c1^2
            self.c0 = v2;
            P::mul_base_field_by_nonresidue_plus_one_and_add(&mut self.c0, &v0);

            self
        }
    }

    fn inverse(&self) -> Option<Self> {
        if self.is_zero() {
            None
        } else {
            // Guide to Pairing-based Cryptography, Algorithm 5.19.
            // v1 = c1.square()
            let v1 = self.c1.square();
            // v0 = c0.square() - beta * v1
            let mut v0 = v1;
            P::sub_and_mul_base_field_by_nonresidue(&mut v0, &self.c0.square());

            v0.inverse().map(|v1| {
                let c0 = self.c0 * &v1;
                let c1 = -(self.c1 * &v1);
                Self::new(c0, c1)
            })
        }
    }

    fn inverse_in_place(&mut self) -> Option<&mut Self> {
        if let Some(inverse) = self.inverse() {
            *self = inverse;
            Some(self)
        } else {
            None
        }
    }

    fn frobenius_map_in_place(&mut self, power: usize) {
        self.c0.frobenius_map_in_place(power);
        self.c1.frobenius_map_in_place(power);
        P::mul_base_field_by_frob_coeff(&mut self.c1, power);
    }

    fn legendre(&self) -> LegendreSymbol {
        // The LegendreSymbol in a field of order q for an element x can be
        // computed as x^((q-1)/2).
        // Since we are in a quadratic extension of a field F_p,
        // we have that q = p^2.
        // Notice then that (q-1)/2 = ((p-1)/2) * (1 + p).
        // This implies that we can compute the symbol as (x^(1+p))^((p-1)/2).
        // Recall that computing x^(1 + p) is equivalent to taking the norm of x,
        // and it will output an element in the base field F_p.
        // Then exponentiating by (p-1)/2 in the base field is equivalent to computing
        // the legendre symbol in the base field.
        self.norm().legendre()
    }

    fn sqrt(&self) -> Option<Self> {
        // Square root based on the complex method. See
        // https://eprint.iacr.org/2012/685.pdf (page 15, algorithm 8)
        if self.c1.is_zero() {
            // for c = c0 + c1 * x, we have c1 = 0
            // sqrt(c) == sqrt(c0) is an element of Fp2, i.e. sqrt(c0) = a = a0 + a1 * x for some a0, a1 in Fp
            // squaring both sides: c0 = a0^2 + a1^2 * x^2 + (2 * a0 * a1 * x) = a0^2 + (a1^2 * P::NONRESIDUE)
            // since there are no `x` terms on LHS, a0 * a1 = 0
            // so either a0 = sqrt(c0) or a1 = sqrt(c0/P::NONRESIDUE)
            if self.c0.legendre().is_qr() {
                // either c0 is a valid sqrt in the base field
                return self.c0.sqrt().map(|c0| Self::new(c0, P::BaseField::ZERO));
            } else {
                // or we need to compute sqrt(c0/P::NONRESIDUE)
                return (self.c0.div(P::NONRESIDUE))
                    .sqrt()
                    .map(|res| Self::new(P::BaseField::ZERO, res));
            }
        }
        // Try computing the square root
        // Check at the end of the algorithm if it was a square root
        let alpha = self.norm();

        // Compute `(p+1)/2` as `1/2`.
        // This is cheaper than `P::BaseField::one().double().inverse()`
        let mut two_inv = P::BasePrimeField::MODULUS;

        two_inv.add_with_carry(&1u64.into());
        two_inv.div2();

        let two_inv = P::BasePrimeField::from(two_inv);
        let two_inv = P::BaseField::from_base_prime_field(two_inv);

        alpha.sqrt().and_then(|alpha| {
            let mut delta = (alpha + &self.c0) * &two_inv;
            if delta.legendre().is_qnr() {
                delta -= &alpha;
            }
            let c0 = delta.sqrt().expect("Delta must have a square root");
            let c0_inv = c0.inverse().expect("c0 must have an inverse");
            let sqrt_cand = Self::new(c0, self.c1 * &two_inv * &c0_inv);
            // Check if sqrt_cand is actually the square root
            // if not, there exists no square root.
            if sqrt_cand.square() == *self {
                Some(sqrt_cand)
            } else {
                #[cfg(debug_assertions)]
                {
                    use crate::fields::LegendreSymbol::*;
                    if self.legendre() != QuadraticNonResidue {
                        panic!(
                            "Input has a square root per its legendre symbol, but it was not found"
                        )
                    }
                }
                None
            }
        })
    }

    fn sqrt_in_place(&mut self) -> Option<&mut Self> {
        (*self).sqrt().map(|sqrt| {
            *self = sqrt;
            self
        })
    }
}

/// `QuadExtField` elements are ordered lexicographically.
impl<P: QuadExtConfig> Ord for QuadExtField<P> {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> Ordering {
        match self.c1.cmp(&other.c1) {
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
            Ordering::Equal => self.c0.cmp(&other.c0),
        }
    }
}

impl<P: QuadExtConfig> PartialOrd for QuadExtField<P> {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: QuadExtConfig> Zeroize for QuadExtField<P> {
    // The phantom data does not contain element-specific data
    // and thus does not need to be zeroized.
    fn zeroize(&mut self) {
        self.c0.zeroize();
        self.c1.zeroize();
    }
}

impl<P: QuadExtConfig> From<u128> for QuadExtField<P> {
    fn from(other: u128) -> Self {
        Self::new(other.into(), P::BaseField::ZERO)
    }
}

impl<P: QuadExtConfig> From<i128> for QuadExtField<P> {
    #[inline]
    fn from(val: i128) -> Self {
        let abs = Self::from(val.unsigned_abs());
        if val.is_positive() {
            abs
        } else {
            -abs
        }
    }
}

impl<P: QuadExtConfig> From<u64> for QuadExtField<P> {
    fn from(other: u64) -> Self {
        Self::new(other.into(), P::BaseField::ZERO)
    }
}

impl<P: QuadExtConfig> From<i64> for QuadExtField<P> {
    #[inline]
    fn from(val: i64) -> Self {
        let abs = Self::from(val.unsigned_abs());
        if val.is_positive() {
            abs
        } else {
            -abs
        }
    }
}

impl<P: QuadExtConfig> From<u32> for QuadExtField<P> {
    fn from(other: u32) -> Self {
        Self::new(other.into(), P::BaseField::ZERO)
    }
}

impl<P: QuadExtConfig> From<i32> for QuadExtField<P> {
    #[inline]
    fn from(val: i32) -> Self {
        let abs = Self::from(val.unsigned_abs());
        if val.is_positive() {
            abs
        } else {
            -abs
        }
    }
}

impl<P: QuadExtConfig> From<u16> for QuadExtField<P> {
    fn from(other: u16) -> Self {
        Self::new(other.into(), P::BaseField::ZERO)
    }
}

impl<P: QuadExtConfig> From<i16> for QuadExtField<P> {
    #[inline]
    fn from(val: i16) -> Self {
        let abs = Self::from(val.unsigned_abs());
        if val.is_positive() {
            abs
        } else {
            -abs
        }
    }
}

impl<P: QuadExtConfig> From<u8> for QuadExtField<P> {
    fn from(other: u8) -> Self {
        Self::new(other.into(), P::BaseField::ZERO)
    }
}

impl<P: QuadExtConfig> From<i8> for QuadExtField<P> {
    #[inline]
    fn from(val: i8) -> Self {
        let abs = Self::from(val.unsigned_abs());
        if val.is_positive() {
            abs
        } else {
            -abs
        }
    }
}

impl<P: QuadExtConfig> From<bool> for QuadExtField<P> {
    fn from(other: bool) -> Self {
        Self::new(u8::from(other).into(), P::BaseField::ZERO)
    }
}

impl<P: QuadExtConfig> Neg for QuadExtField<P> {
    type Output = Self;
    #[inline]
    #[must_use]
    fn neg(mut self) -> Self {
        self.c0.neg_in_place();
        self.c1.neg_in_place();
        self
    }
}

impl<P: QuadExtConfig> Distribution<QuadExtField<P>> for Standard {
    #[inline]
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> QuadExtField<P> {
        QuadExtField::new(UniformRand::rand(rng), UniformRand::rand(rng))
    }
}

impl<'a, P: QuadExtConfig> Add<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline]
    fn add(mut self, other: &Self) -> Self {
        self += other;
        self
    }
}

impl<'a, P: QuadExtConfig> Sub<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline(always)]
    fn sub(mut self, other: &Self) -> Self {
        self -= other;
        self
    }
}

impl<'a, P: QuadExtConfig> Mul<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline(always)]
    fn mul(mut self, other: &Self) -> Self {
        self *= other;
        self
    }
}

impl<'a, P: QuadExtConfig> Div<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline]
    fn div(mut self, other: &Self) -> Self {
        self.mul_assign(&other.inverse().unwrap());
        self
    }
}

impl<'a, P: QuadExtConfig> AddAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn add_assign(&mut self, other: &Self) {
        self.c0 += &other.c0;
        self.c1 += &other.c1;
    }
}

impl<'a, P: QuadExtConfig> SubAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn sub_assign(&mut self, other: &Self) {
        self.c0 -= &other.c0;
        self.c1 -= &other.c1;
    }
}

impl_additive_ops_from_ref!(QuadExtField, QuadExtConfig);
impl_multiplicative_ops_from_ref!(QuadExtField, QuadExtConfig);

impl<'a, P: QuadExtConfig> MulAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn mul_assign(&mut self, other: &Self) {
        if Self::extension_degree() == 2 {
            let c1_input = [self.c0, self.c1];
            P::mul_base_field_by_nonresidue_in_place(&mut self.c1);
            *self = Self::new(
                P::BaseField::sum_of_products(&[self.c0, self.c1], &[other.c0, other.c1]),
                P::BaseField::sum_of_products(&c1_input, &[other.c1, other.c0]),
            )
        } else {
            // Karatsuba multiplication;
            // Guide to Pairing-based cryprography, Algorithm 5.16.
            let mut v0 = self.c0;
            v0 *= &other.c0;
            let mut v1 = self.c1;
            v1 *= &other.c1;

            self.c1 += &self.c0;
            self.c1 *= &(other.c0 + &other.c1);
            self.c1 -= &v0;
            self.c1 -= &v1;
            self.c0 = v1;
            P::mul_base_field_by_nonresidue_and_add(&mut self.c0, &v0);
        }
    }
}

impl<'a, P: QuadExtConfig> DivAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn div_assign(&mut self, other: &Self) {
        self.mul_assign(&other.inverse().unwrap());
    }
}

impl<P: QuadExtConfig> fmt::Display for QuadExtField<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QuadExtField({} + {} * u)", self.c0, self.c1)
    }
}

impl<P: QuadExtConfig> CanonicalSerializeWithFlags for QuadExtField<P> {
    #[inline]
    fn serialize_with_flags<W: Write, F: Flags>(
        &self,
        mut writer: W,
        flags: F,
    ) -> Result<(), SerializationError> {
        self.c0.serialize_compressed(&mut writer)?;
        self.c1.serialize_with_flags(&mut writer, flags)?;
        Ok(())
    }

    #[inline]
    fn serialized_size_with_flags<F: Flags>(&self) -> usize {
        self.c0.compressed_size() + self.c1.serialized_size_with_flags::<F>()
    }
}

impl<P: QuadExtConfig> CanonicalSerialize for QuadExtField<P> {
    #[inline]
    fn serialize_with_mode<W: Write>(
        &self,
        writer: W,
        _compress: Compress,
    ) -> Result<(), SerializationError> {
        self.serialize_with_flags(writer, EmptyFlags)
    }

    #[inline]
    fn serialized_size(&self, _compress: Compress) -> usize {
        self.serialized_size_with_flags::<EmptyFlags>()
    }
}

impl<P: QuadExtConfig> CanonicalDeserializeWithFlags for QuadExtField<P> {
    #[inline]
    fn deserialize_with_flags<R: Read, F: Flags>(
        mut reader: R,
    ) -> Result<(Self, F), SerializationError> {
        let c0 = CanonicalDeserialize::deserialize_compressed(&mut reader)?;
        let (c1, flags) = CanonicalDeserializeWithFlags::deserialize_with_flags(&mut reader)?;
        Ok((QuadExtField::new(c0, c1), flags))
    }
}

impl<P: QuadExtConfig> Valid for QuadExtField<P> {
    fn check(&self) -> Result<(), SerializationError> {
        self.c0.check()?;
        self.c1.check()
    }
}

impl<P: QuadExtConfig> CanonicalDeserialize for QuadExtField<P> {
    #[inline]
    fn deserialize_with_mode<R: Read>(
        mut reader: R,
        compress: Compress,
        validate: Validate,
    ) -> Result<Self, SerializationError> {
        let c0: P::BaseField =
            CanonicalDeserialize::deserialize_with_mode(&mut reader, compress, validate)?;
        let c1: P::BaseField =
            CanonicalDeserialize::deserialize_with_mode(&mut reader, compress, validate)?;
        Ok(QuadExtField::new(c0, c1))
    }
}

impl<P: QuadExtConfig> ToConstraintField<P::BasePrimeField> for QuadExtField<P>
where
    P::BaseField: ToConstraintField<P::BasePrimeField>,
{
    fn to_field_elements(&self) -> Option<Vec<P::BasePrimeField>> {
        let mut res = Vec::new();
        let mut c0_elems = self.c0.to_field_elements()?;
        let mut c1_elems = self.c1.to_field_elements()?;

        res.append(&mut c0_elems);
        res.append(&mut c1_elems);

        Some(res)
    }
}

#[cfg(test)]
mod quad_ext_tests {
    use super::*;
    use ark_std::test_rng;
    use ark_test_curves::{
        bls12_381::{Fq, Fq2},
        Field,
    };

    #[test]
    fn test_from_base_prime_field_elements() {
        let ext_degree = Fq2::extension_degree() as usize;
        // Test on slice lengths that aren't equal to the extension degree
        let max_num_elems_to_test = 4;
        for d in 0..max_num_elems_to_test {
            if d == ext_degree {
                continue;
            }
            let mut random_coeffs = Vec::<Fq>::new();
            for _ in 0..d {
                random_coeffs.push(Fq::rand(&mut test_rng()));
            }
            let res = Fq2::from_base_prime_field_elems(&random_coeffs);
            assert_eq!(res, None);
        }
        // Test on slice lengths that are equal to the extension degree
        // We test consistency against Fq2::new
        let number_of_tests = 10;
        for _ in 0..number_of_tests {
            let mut random_coeffs = Vec::<Fq>::new();
            for _ in 0..ext_degree {
                random_coeffs.push(Fq::rand(&mut test_rng()));
            }
            let actual = Fq2::from_base_prime_field_elems(&random_coeffs).unwrap();
            let expected = Fq2::new(random_coeffs[0], random_coeffs[1]);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn test_from_base_prime_field_element() {
        let ext_degree = Fq2::extension_degree() as usize;
        let max_num_elems_to_test = 10;
        for _ in 0..max_num_elems_to_test {
            let mut random_coeffs = vec![Fq::zero(); ext_degree];
            let random_coeff = Fq::rand(&mut test_rng());
            let res = Fq2::from_base_prime_field(random_coeff);
            random_coeffs[0] = random_coeff;
            assert_eq!(
                res,
                Fq2::from_base_prime_field_elems(&random_coeffs).unwrap()
            );
        }
    }
}
