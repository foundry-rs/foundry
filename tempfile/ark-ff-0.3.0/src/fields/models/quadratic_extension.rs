use ark_serialize::{
    CanonicalDeserialize, CanonicalDeserializeWithFlags, CanonicalSerialize,
    CanonicalSerializeWithFlags, EmptyFlags, Flags, SerializationError,
};
use ark_std::{
    cmp::{Ord, Ordering, PartialOrd},
    fmt,
    io::{Read, Result as IoResult, Write},
    marker::PhantomData,
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
    bytes::{FromBytes, ToBytes},
    fields::{Field, LegendreSymbol, PrimeField, SquareRootField},
    ToConstraintField, UniformRand,
};

/// Defines a Quadratic extension field from a quadratic non-residue.
pub trait QuadExtParameters: 'static + Send + Sync + Sized {
    /// The prime field that this quadratic extension is eventually an extension of.
    type BasePrimeField: PrimeField;
    /// The base field that this field is a quadratic extension of.
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
    fn mul_base_field_by_nonresidue(fe: &Self::BaseField) -> Self::BaseField {
        Self::NONRESIDUE * fe
    }

    /// A specializable method for computing x + mul_base_field_by_nonresidue(y)
    /// This allows for optimizations when the non-residue is
    /// canonically negative in the field.
    #[inline(always)]
    fn add_and_mul_base_field_by_nonresidue(
        x: &Self::BaseField,
        y: &Self::BaseField,
    ) -> Self::BaseField {
        *x + Self::mul_base_field_by_nonresidue(y)
    }

    /// A specializable method for computing x + mul_base_field_by_nonresidue(y) + y
    /// This allows for optimizations when the non-residue is not -1.
    #[inline(always)]
    fn add_and_mul_base_field_by_nonresidue_plus_one(
        x: &Self::BaseField,
        y: &Self::BaseField,
    ) -> Self::BaseField {
        let mut tmp = *x;
        tmp += y;
        Self::add_and_mul_base_field_by_nonresidue(&tmp, &y)
    }

    /// A specializable method for computing x - mul_base_field_by_nonresidue(y)
    /// This allows for optimizations when the non-residue is
    /// canonically negative in the field.
    #[inline(always)]
    fn sub_and_mul_base_field_by_nonresidue(
        x: &Self::BaseField,
        y: &Self::BaseField,
    ) -> Self::BaseField {
        *x - Self::mul_base_field_by_nonresidue(y)
    }

    /// A specializable method for multiplying an element of the base field by
    /// the appropriate Frobenius coefficient.
    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize);

    /// A specializable method for exponentiating that is to be used
    /// *only* when `fe` is known to be in the cyclotommic subgroup.
    fn cyclotomic_exp(fe: &QuadExtField<Self>, exponent: impl AsRef<[u64]>) -> QuadExtField<Self> {
        let mut res = QuadExtField::one();
        let mut self_inverse = fe.clone();
        self_inverse.conjugate();

        let mut found_nonzero = false;
        let naf = crate::biginteger::arithmetic::find_wnaf(exponent.as_ref());

        for &value in naf.iter().rev() {
            if found_nonzero {
                res.square_in_place();
            }

            if value != 0 {
                found_nonzero = true;

                if value > 0 {
                    res *= fe;
                } else {
                    res *= &self_inverse;
                }
            }
        }
        res
    }
}

/// An element of a quadratic extension field F_p\[X\]/(X^2 - P::NONRESIDUE) is
/// represented as c0 + c1 * X, for c0, c1 in `P::BaseField`.
#[derive(Derivative)]
#[derivative(
    Default(bound = "P: QuadExtParameters"),
    Hash(bound = "P: QuadExtParameters"),
    Clone(bound = "P: QuadExtParameters"),
    Copy(bound = "P: QuadExtParameters"),
    Debug(bound = "P: QuadExtParameters"),
    PartialEq(bound = "P: QuadExtParameters"),
    Eq(bound = "P: QuadExtParameters")
)]
pub struct QuadExtField<P: QuadExtParameters> {
    pub c0: P::BaseField,
    pub c1: P::BaseField,
    #[derivative(Debug = "ignore")]
    #[doc(hidden)]
    pub _parameters: PhantomData<P>,
}

impl<P: QuadExtParameters> QuadExtField<P> {
    pub fn new(c0: P::BaseField, c1: P::BaseField) -> Self {
        QuadExtField {
            c0,
            c1,
            _parameters: PhantomData,
        }
    }

    /// This is only to be used when the element is *known* to be in the cyclotomic subgroup.
    pub fn conjugate(&mut self) {
        self.c1 = -self.c1;
    }

    /// This is only to be used when the element is *known* to be in the cyclotomic subgroup.
    pub fn cyclotomic_exp(&self, exponent: impl AsRef<[u64]>) -> Self {
        P::cyclotomic_exp(self, exponent)
    }

    /// Norm of QuadExtField over `P::BaseField`:`Norm(a) = a * a.conjugate()`.
    /// This simplifies to: `Norm(a) = a.x^2 - P::NON_RESIDUE * a.y^2`.
    /// This is alternatively expressed as `Norm(a) = a^(1 + p)`.
    pub fn norm(&self) -> P::BaseField {
        let t0 = self.c0.square();
        // t1 = t0 - P::NON_RESIDUE * c1^2
        let mut t1 = self.c1.square();
        t1 = P::sub_and_mul_base_field_by_nonresidue(&t0, &t1);
        t1
    }

    pub fn mul_assign_by_basefield(&mut self, element: &P::BaseField) {
        self.c0.mul_assign(element);
        self.c1.mul_assign(element);
    }
}

impl<P: QuadExtParameters> Zero for QuadExtField<P> {
    fn zero() -> Self {
        QuadExtField::new(P::BaseField::zero(), P::BaseField::zero())
    }

    fn is_zero(&self) -> bool {
        self.c0.is_zero() && self.c1.is_zero()
    }
}

impl<P: QuadExtParameters> One for QuadExtField<P> {
    fn one() -> Self {
        QuadExtField::new(P::BaseField::one(), P::BaseField::zero())
    }

    fn is_one(&self) -> bool {
        self.c0.is_one() && self.c1.is_zero()
    }
}

impl<P: QuadExtParameters> Field for QuadExtField<P> {
    type BasePrimeField = P::BasePrimeField;

    fn extension_degree() -> u64 {
        2 * P::BaseField::extension_degree()
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
        if P::NONRESIDUE == -P::BaseField::one() {
            // When the non-residue is -1, we save 2 intermediate additions,
            // and use one fewer intermediate variable

            let c0_copy = self.c0;
            // v0 = c0 - c1
            let v0 = self.c0 - &self.c1;
            // result.c1 = 2 c1
            self.c1.double_in_place();
            // result.c0 = (c0 - c1) + 2c1 = c0 + c1
            self.c0 = v0 + &self.c1;
            // result.c0 *= (c0 - c1)
            // result.c0 = (c0 - c1) * (c0 + c1) = c0^2 - c1^2
            self.c0 *= &v0;
            // result.c1 *= c0
            // result.c1 = (2 * c1) * c0
            self.c1 *= &c0_copy;

            self
        } else {
            // v0 = c0 - c1
            let mut v0 = self.c0 - &self.c1;
            // v3 = c0 - beta * c1
            let v3 = P::sub_and_mul_base_field_by_nonresidue(&self.c0, &self.c1);
            // v2 = c0 * c1
            let v2 = self.c0 * &self.c1;

            // v0 = (v0 * v3)
            // v0 = (c0 - c1) * (c0 - beta*c1)
            // v0 = c0^2 - beta * c0 * c1 - c0 * c1 + beta * c1^2
            v0 *= &v3;

            // result.c1 = 2 * c0 * c1
            self.c1 = v2.double();
            // result.c0 = (v0) + ((beta + 1) * v2)
            // result.c0 = (c0^2 - beta * c0 * c1 - c0 * c1 + beta * c1^2) + ((beta + 1) c0 * c1)
            // result.c0 = (c0^2 - beta * c0 * c1 + beta * c1^2) + (beta * c0 * c1)
            // result.c0 = c0^2 + beta * c1^2
            self.c0 = P::add_and_mul_base_field_by_nonresidue_plus_one(&v0, &v2);

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
            let v0 = P::sub_and_mul_base_field_by_nonresidue(&self.c0.square(), &v1);

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

    fn frobenius_map(&mut self, power: usize) {
        self.c0.frobenius_map(power);
        self.c1.frobenius_map(power);
        P::mul_base_field_by_frob_coeff(&mut self.c1, power);
    }
}

impl<'a, P: QuadExtParameters> SquareRootField for QuadExtField<P>
where
    P::BaseField: SquareRootField,
{
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
            return self.c0.sqrt().map(|c0| Self::new(c0, P::BaseField::zero()));
        }
        // Try computing the square root
        // Check at the end of the algorithm if it was a square root
        let alpha = self.norm();
        // TODO: Precompute this
        let two_inv = P::BaseField::one()
            .double()
            .inverse()
            .expect("Two should always have an inverse");
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
impl<P: QuadExtParameters> Ord for QuadExtField<P> {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> Ordering {
        match self.c1.cmp(&other.c1) {
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
            Ordering::Equal => self.c0.cmp(&other.c0),
        }
    }
}

impl<P: QuadExtParameters> PartialOrd for QuadExtField<P> {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: QuadExtParameters> Zeroize for QuadExtField<P> {
    // The phantom data does not contain element-specific data
    // and thus does not need to be zeroized.
    fn zeroize(&mut self) {
        self.c0.zeroize();
        self.c1.zeroize();
    }
}

impl<P: QuadExtParameters> From<u128> for QuadExtField<P> {
    fn from(other: u128) -> Self {
        Self::new(other.into(), P::BaseField::zero())
    }
}

impl<P: QuadExtParameters> From<i128> for QuadExtField<P> {
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

impl<P: QuadExtParameters> From<u64> for QuadExtField<P> {
    fn from(other: u64) -> Self {
        Self::new(other.into(), P::BaseField::zero())
    }
}

impl<P: QuadExtParameters> From<i64> for QuadExtField<P> {
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

impl<P: QuadExtParameters> From<u32> for QuadExtField<P> {
    fn from(other: u32) -> Self {
        Self::new(other.into(), P::BaseField::zero())
    }
}

impl<P: QuadExtParameters> From<i32> for QuadExtField<P> {
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

impl<P: QuadExtParameters> From<u16> for QuadExtField<P> {
    fn from(other: u16) -> Self {
        Self::new(other.into(), P::BaseField::zero())
    }
}

impl<P: QuadExtParameters> From<i16> for QuadExtField<P> {
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

impl<P: QuadExtParameters> From<u8> for QuadExtField<P> {
    fn from(other: u8) -> Self {
        Self::new(other.into(), P::BaseField::zero())
    }
}

impl<P: QuadExtParameters> From<i8> for QuadExtField<P> {
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

impl<P: QuadExtParameters> From<bool> for QuadExtField<P> {
    fn from(other: bool) -> Self {
        Self::new(u8::from(other).into(), P::BaseField::zero())
    }
}

impl<P: QuadExtParameters> ToBytes for QuadExtField<P> {
    #[inline]
    fn write<W: Write>(&self, mut writer: W) -> IoResult<()> {
        self.c0.write(&mut writer)?;
        self.c1.write(writer)
    }
}

impl<P: QuadExtParameters> FromBytes for QuadExtField<P> {
    #[inline]
    fn read<R: Read>(mut reader: R) -> IoResult<Self> {
        let c0 = P::BaseField::read(&mut reader)?;
        let c1 = P::BaseField::read(reader)?;
        Ok(QuadExtField::new(c0, c1))
    }
}

impl<P: QuadExtParameters> Neg for QuadExtField<P> {
    type Output = Self;
    #[inline]
    #[must_use]
    fn neg(mut self) -> Self {
        self.c0 = -self.c0;
        self.c1 = -self.c1;
        self
    }
}

impl<P: QuadExtParameters> Distribution<QuadExtField<P>> for Standard {
    #[inline]
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> QuadExtField<P> {
        QuadExtField::new(UniformRand::rand(rng), UniformRand::rand(rng))
    }
}

impl<'a, P: QuadExtParameters> Add<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline]
    fn add(mut self, other: &Self) -> Self {
        self.add_assign(other);
        self
    }
}

impl<'a, P: QuadExtParameters> Sub<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline]
    fn sub(mut self, other: &Self) -> Self {
        self.sub_assign(other);
        self
    }
}

impl<'a, P: QuadExtParameters> Mul<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline]
    fn mul(mut self, other: &Self) -> Self {
        self.mul_assign(other);
        self
    }
}

impl<'a, P: QuadExtParameters> Div<&'a QuadExtField<P>> for QuadExtField<P> {
    type Output = Self;

    #[inline]
    fn div(mut self, other: &Self) -> Self {
        self.mul_assign(&other.inverse().unwrap());
        self
    }
}

impl<'a, P: QuadExtParameters> AddAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn add_assign(&mut self, other: &Self) {
        self.c0 += &other.c0;
        self.c1 += &other.c1;
    }
}

impl<'a, P: QuadExtParameters> SubAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn sub_assign(&mut self, other: &Self) {
        self.c0 -= &other.c0;
        self.c1 -= &other.c1;
    }
}

impl_additive_ops_from_ref!(QuadExtField, QuadExtParameters);
impl_multiplicative_ops_from_ref!(QuadExtField, QuadExtParameters);

impl<'a, P: QuadExtParameters> MulAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn mul_assign(&mut self, other: &Self) {
        // Karatsuba multiplication;
        // Guide to Pairing-based cryprography, Algorithm 5.16.
        let v0 = self.c0 * &other.c0;
        let v1 = self.c1 * &other.c1;

        self.c1 += &self.c0;
        self.c1 *= &(other.c0 + &other.c1);
        self.c1 -= &v0;
        self.c1 -= &v1;
        self.c0 = P::add_and_mul_base_field_by_nonresidue(&v0, &v1);
    }
}

impl<'a, P: QuadExtParameters> DivAssign<&'a Self> for QuadExtField<P> {
    #[inline]
    fn div_assign(&mut self, other: &Self) {
        self.mul_assign(&other.inverse().unwrap());
    }
}

impl<P: QuadExtParameters> fmt::Display for QuadExtField<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QuadExtField({} + {} * u)", self.c0, self.c1)
    }
}

impl<P: QuadExtParameters> CanonicalSerializeWithFlags for QuadExtField<P> {
    #[inline]
    fn serialize_with_flags<W: Write, F: Flags>(
        &self,
        mut writer: W,
        flags: F,
    ) -> Result<(), SerializationError> {
        self.c0.serialize(&mut writer)?;
        self.c1.serialize_with_flags(&mut writer, flags)?;
        Ok(())
    }

    #[inline]
    fn serialized_size_with_flags<F: Flags>(&self) -> usize {
        self.c0.serialized_size() + self.c1.serialized_size_with_flags::<F>()
    }
}

impl<P: QuadExtParameters> CanonicalSerialize for QuadExtField<P> {
    #[inline]
    fn serialize<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.serialize_with_flags(writer, EmptyFlags)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.serialized_size_with_flags::<EmptyFlags>()
    }
}

impl<P: QuadExtParameters> CanonicalDeserializeWithFlags for QuadExtField<P> {
    #[inline]
    fn deserialize_with_flags<R: Read, F: Flags>(
        mut reader: R,
    ) -> Result<(Self, F), SerializationError> {
        let c0: P::BaseField = CanonicalDeserialize::deserialize(&mut reader)?;
        let (c1, flags): (P::BaseField, _) =
            CanonicalDeserializeWithFlags::deserialize_with_flags(&mut reader)?;
        Ok((QuadExtField::new(c0, c1), flags))
    }
}

impl<P: QuadExtParameters> CanonicalDeserialize for QuadExtField<P> {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let c0: P::BaseField = CanonicalDeserialize::deserialize(&mut reader)?;
        let c1: P::BaseField = CanonicalDeserialize::deserialize(&mut reader)?;
        Ok(QuadExtField::new(c0, c1))
    }
}

impl<P: QuadExtParameters> ToConstraintField<P::BasePrimeField> for QuadExtField<P>
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
    use crate::test_field::{Fq, Fq2};
    use ark_std::test_rng;

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
}
