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
    fields::{Field, PrimeField},
    ToConstraintField, UniformRand,
};

pub trait CubicExtParameters: 'static + Send + Sync {
    /// The prime field that this cubic extension is eventually an extension of.
    type BasePrimeField: PrimeField;
    /// The base field that this field is a cubic extension of.
    type BaseField: Field<BasePrimeField = Self::BasePrimeField>;
    /// The type of the coefficients for an efficient implemntation of the
    /// Frobenius endomorphism.
    type FrobCoeff: Field;

    /// The degree of the extension over the base prime field.
    const DEGREE_OVER_BASE_PRIME_FIELD: usize;

    /// The cubic non-residue used to construct the extension.
    const NONRESIDUE: Self::BaseField;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff];
    const FROBENIUS_COEFF_C2: &'static [Self::FrobCoeff];

    /// A specializable method for multiplying an element of the base field by
    /// the quadratic non-residue. This is used in multiplication and squaring.
    #[inline(always)]
    fn mul_base_field_by_nonresidue(fe: &Self::BaseField) -> Self::BaseField {
        Self::NONRESIDUE * fe
    }

    /// A specializable method for multiplying an element of the base field by
    /// the appropriate Frobenius coefficient.
    fn mul_base_field_by_frob_coeff(
        c1: &mut Self::BaseField,
        c2: &mut Self::BaseField,
        power: usize,
    );
}

#[derive(Derivative)]
#[derivative(
    Default(bound = "P: CubicExtParameters"),
    Hash(bound = "P: CubicExtParameters"),
    Clone(bound = "P: CubicExtParameters"),
    Copy(bound = "P: CubicExtParameters"),
    Debug(bound = "P: CubicExtParameters"),
    PartialEq(bound = "P: CubicExtParameters"),
    Eq(bound = "P: CubicExtParameters")
)]
pub struct CubicExtField<P: CubicExtParameters> {
    pub c0: P::BaseField,
    pub c1: P::BaseField,
    pub c2: P::BaseField,
    #[derivative(Debug = "ignore")]
    #[doc(hidden)]
    pub _parameters: PhantomData<P>,
}

impl<P: CubicExtParameters> CubicExtField<P> {
    pub fn new(c0: P::BaseField, c1: P::BaseField, c2: P::BaseField) -> Self {
        CubicExtField {
            c0,
            c1,
            c2,
            _parameters: PhantomData,
        }
    }

    pub fn mul_assign_by_base_field(&mut self, value: &P::BaseField) {
        self.c0.mul_assign(value);
        self.c1.mul_assign(value);
        self.c2.mul_assign(value);
    }

    /// Calculate the norm of an element with respect to the base field `P::BaseField`.
    pub fn norm(&self) -> P::BaseField {
        let mut self_to_p = *self;
        self_to_p.frobenius_map(1);
        let mut self_to_p2 = *self;
        self_to_p2.frobenius_map(2);
        self_to_p *= &(self_to_p2 * self);
        assert!(self_to_p.c1.is_zero() && self_to_p.c2.is_zero());
        self_to_p.c0
    }
}

impl<P: CubicExtParameters> Zero for CubicExtField<P> {
    fn zero() -> Self {
        CubicExtField {
            c0: P::BaseField::zero(),
            c1: P::BaseField::zero(),
            c2: P::BaseField::zero(),
            _parameters: PhantomData,
        }
    }

    fn is_zero(&self) -> bool {
        self.c0.is_zero() && self.c1.is_zero() && self.c2.is_zero()
    }
}

impl<P: CubicExtParameters> One for CubicExtField<P> {
    fn one() -> Self {
        CubicExtField {
            c0: P::BaseField::one(),
            c1: P::BaseField::zero(),
            c2: P::BaseField::zero(),
            _parameters: PhantomData,
        }
    }

    fn is_one(&self) -> bool {
        self.c0.is_one() && self.c1.is_zero() && self.c2.is_zero()
    }
}

impl<P: CubicExtParameters> Field for CubicExtField<P> {
    type BasePrimeField = P::BasePrimeField;

    fn extension_degree() -> u64 {
        3 * P::BaseField::extension_degree()
    }

    fn from_base_prime_field_elems(elems: &[Self::BasePrimeField]) -> Option<Self> {
        if elems.len() != (Self::extension_degree() as usize) {
            return None;
        }
        let base_ext_deg = P::BaseField::extension_degree() as usize;
        Some(Self::new(
            P::BaseField::from_base_prime_field_elems(&elems[0..base_ext_deg]).unwrap(),
            P::BaseField::from_base_prime_field_elems(&elems[base_ext_deg..2 * base_ext_deg])
                .unwrap(),
            P::BaseField::from_base_prime_field_elems(&elems[2 * base_ext_deg..]).unwrap(),
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
        self.c2.double_in_place();
        self
    }

    #[inline]
    fn from_random_bytes_with_flags<F: Flags>(bytes: &[u8]) -> Option<(Self, F)> {
        let split_at = bytes.len() / 3;
        if let Some(c0) = P::BaseField::from_random_bytes(&bytes[..split_at]) {
            if let Some(c1) = P::BaseField::from_random_bytes(&bytes[split_at..2 * split_at]) {
                if let Some((c2, flags)) =
                    P::BaseField::from_random_bytes_with_flags(&bytes[2 * split_at..])
                {
                    return Some((CubicExtField::new(c0, c1, c2), flags));
                }
            }
        }
        None
    }

    #[inline]
    fn from_random_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_random_bytes_with_flags::<EmptyFlags>(bytes).map(|f| f.0)
    }

    fn square(&self) -> Self {
        let mut result = *self;
        result.square_in_place();
        result
    }

    fn square_in_place(&mut self) -> &mut Self {
        // Devegili OhEig Scott Dahab --- Multiplication and Squaring on
        // AbstractPairing-Friendly
        // Fields.pdf; Section 4 (CH-SQR2)
        let a = self.c0;
        let b = self.c1;
        let c = self.c2;

        let s0 = a.square();
        let ab = a * &b;
        let s1 = ab.double();
        let s2 = (a - &b + &c).square();
        let bc = b * &c;
        let s3 = bc.double();
        let s4 = c.square();

        self.c0 = s0 + &P::mul_base_field_by_nonresidue(&s3);
        self.c1 = s1 + &P::mul_base_field_by_nonresidue(&s4);
        self.c2 = s1 + &s2 + &s3 - &s0 - &s4;
        self
    }

    fn inverse(&self) -> Option<Self> {
        if self.is_zero() {
            None
        } else {
            // From "High-Speed Software Implementation of the Optimal Ate AbstractPairing
            // over
            // Barreto-Naehrig Curves"; Algorithm 17
            let t0 = self.c0.square();
            let t1 = self.c1.square();
            let t2 = self.c2.square();
            let t3 = self.c0 * &self.c1;
            let t4 = self.c0 * &self.c2;
            let t5 = self.c1 * &self.c2;
            let n5 = P::mul_base_field_by_nonresidue(&t5);

            let s0 = t0 - &n5;
            let s1 = P::mul_base_field_by_nonresidue(&t2) - &t3;
            let s2 = t1 - &t4; // typo in paper referenced above. should be "-" as per Scott, but is "*"

            let a1 = self.c2 * &s1;
            let a2 = self.c1 * &s2;
            let mut a3 = a1 + &a2;
            a3 = P::mul_base_field_by_nonresidue(&a3);
            let t6 = (self.c0 * &s0 + &a3).inverse().unwrap();

            let c0 = t6 * &s0;
            let c1 = t6 * &s1;
            let c2 = t6 * &s2;

            Some(Self::new(c0, c1, c2))
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
        self.c2.frobenius_map(power);

        P::mul_base_field_by_frob_coeff(&mut self.c1, &mut self.c2, power);
    }
}

/// `CubicExtField` elements are ordered lexicographically.
impl<P: CubicExtParameters> Ord for CubicExtField<P> {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> Ordering {
        let c2_cmp = self.c2.cmp(&other.c2);
        let c1_cmp = self.c1.cmp(&other.c1);
        let c0_cmp = self.c0.cmp(&other.c0);
        if c2_cmp == Ordering::Equal {
            if c1_cmp == Ordering::Equal {
                c0_cmp
            } else {
                c1_cmp
            }
        } else {
            c2_cmp
        }
    }
}

impl<P: CubicExtParameters> PartialOrd for CubicExtField<P> {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: CubicExtParameters> Zeroize for CubicExtField<P> {
    // The phantom data does not contain element-specific data
    // and thus does not need to be zeroized.
    fn zeroize(&mut self) {
        self.c0.zeroize();
        self.c1.zeroize();
        self.c2.zeroize();
    }
}

impl<P: CubicExtParameters> From<u128> for CubicExtField<P> {
    fn from(other: u128) -> Self {
        let fe: P::BaseField = other.into();
        Self::new(fe, P::BaseField::zero(), P::BaseField::zero())
    }
}

impl<P: CubicExtParameters> From<i128> for CubicExtField<P> {
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

impl<P: CubicExtParameters> From<u64> for CubicExtField<P> {
    fn from(other: u64) -> Self {
        let fe: P::BaseField = other.into();
        Self::new(fe, P::BaseField::zero(), P::BaseField::zero())
    }
}

impl<P: CubicExtParameters> From<i64> for CubicExtField<P> {
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

impl<P: CubicExtParameters> From<u32> for CubicExtField<P> {
    fn from(other: u32) -> Self {
        let fe: P::BaseField = other.into();
        Self::new(fe, P::BaseField::zero(), P::BaseField::zero())
    }
}

impl<P: CubicExtParameters> From<i32> for CubicExtField<P> {
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

impl<P: CubicExtParameters> From<u16> for CubicExtField<P> {
    fn from(other: u16) -> Self {
        let fe: P::BaseField = other.into();
        Self::new(fe, P::BaseField::zero(), P::BaseField::zero())
    }
}

impl<P: CubicExtParameters> From<i16> for CubicExtField<P> {
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

impl<P: CubicExtParameters> From<u8> for CubicExtField<P> {
    fn from(other: u8) -> Self {
        let fe: P::BaseField = other.into();
        Self::new(fe, P::BaseField::zero(), P::BaseField::zero())
    }
}

impl<P: CubicExtParameters> From<i8> for CubicExtField<P> {
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

impl<P: CubicExtParameters> From<bool> for CubicExtField<P> {
    fn from(other: bool) -> Self {
        Self::new(
            u8::from(other).into(),
            P::BaseField::zero(),
            P::BaseField::zero(),
        )
    }
}

impl<P: CubicExtParameters> ToBytes for CubicExtField<P> {
    #[inline]
    fn write<W: Write>(&self, mut writer: W) -> IoResult<()> {
        self.c0.write(&mut writer)?;
        self.c1.write(&mut writer)?;
        self.c2.write(writer)
    }
}

impl<P: CubicExtParameters> FromBytes for CubicExtField<P> {
    #[inline]
    fn read<R: Read>(mut reader: R) -> IoResult<Self> {
        let c0 = P::BaseField::read(&mut reader)?;
        let c1 = P::BaseField::read(&mut reader)?;
        let c2 = P::BaseField::read(reader)?;
        Ok(CubicExtField::new(c0, c1, c2))
    }
}

impl<P: CubicExtParameters> Neg for CubicExtField<P> {
    type Output = Self;
    #[inline]
    fn neg(mut self) -> Self {
        self.c0 = -self.c0;
        self.c1 = -self.c1;
        self.c2 = -self.c2;
        self
    }
}

impl<P: CubicExtParameters> Distribution<CubicExtField<P>> for Standard {
    #[inline]
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> CubicExtField<P> {
        CubicExtField::new(
            UniformRand::rand(rng),
            UniformRand::rand(rng),
            UniformRand::rand(rng),
        )
    }
}

impl<'a, P: CubicExtParameters> Add<&'a CubicExtField<P>> for CubicExtField<P> {
    type Output = Self;

    #[inline]
    fn add(mut self, other: &Self) -> Self {
        self.add_assign(other);
        self
    }
}

impl<'a, P: CubicExtParameters> Sub<&'a CubicExtField<P>> for CubicExtField<P> {
    type Output = Self;

    #[inline]
    fn sub(mut self, other: &Self) -> Self {
        self.sub_assign(other);
        self
    }
}

impl<'a, P: CubicExtParameters> Mul<&'a CubicExtField<P>> for CubicExtField<P> {
    type Output = Self;

    #[inline]
    fn mul(mut self, other: &Self) -> Self {
        self.mul_assign(other);
        self
    }
}

impl<'a, P: CubicExtParameters> Div<&'a CubicExtField<P>> for CubicExtField<P> {
    type Output = Self;

    #[inline]
    fn div(mut self, other: &Self) -> Self {
        self.mul_assign(&other.inverse().unwrap());
        self
    }
}

impl_additive_ops_from_ref!(CubicExtField, CubicExtParameters);
impl_multiplicative_ops_from_ref!(CubicExtField, CubicExtParameters);
impl<'a, P: CubicExtParameters> AddAssign<&'a Self> for CubicExtField<P> {
    #[inline]
    fn add_assign(&mut self, other: &Self) {
        self.c0.add_assign(&other.c0);
        self.c1.add_assign(&other.c1);
        self.c2.add_assign(&other.c2);
    }
}

impl<'a, P: CubicExtParameters> SubAssign<&'a Self> for CubicExtField<P> {
    #[inline]
    fn sub_assign(&mut self, other: &Self) {
        self.c0.sub_assign(&other.c0);
        self.c1.sub_assign(&other.c1);
        self.c2.sub_assign(&other.c2);
    }
}

impl<'a, P: CubicExtParameters> MulAssign<&'a Self> for CubicExtField<P> {
    #[inline]
    #[allow(clippy::many_single_char_names)]
    fn mul_assign(&mut self, other: &Self) {
        // Devegili OhEig Scott Dahab --- Multiplication and Squaring on
        // AbstractPairing-Friendly
        // Fields.pdf; Section 4 (Karatsuba)

        let a = other.c0;
        let b = other.c1;
        let c = other.c2;

        let d = self.c0;
        let e = self.c1;
        let f = self.c2;

        let ad = d * &a;
        let be = e * &b;
        let cf = f * &c;

        let x = (e + &f) * &(b + &c) - &be - &cf;
        let y = (d + &e) * &(a + &b) - &ad - &be;
        let z = (d + &f) * &(a + &c) - &ad + &be - &cf;

        self.c0 = ad + &P::mul_base_field_by_nonresidue(&x);
        self.c1 = y + &P::mul_base_field_by_nonresidue(&cf);
        self.c2 = z;
    }
}

impl<'a, P: CubicExtParameters> DivAssign<&'a Self> for CubicExtField<P> {
    #[inline]
    fn div_assign(&mut self, other: &Self) {
        self.mul_assign(&other.inverse().unwrap());
    }
}

impl<P: CubicExtParameters> fmt::Display for CubicExtField<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CubicExtField({}, {}, {})", self.c0, self.c1, self.c2)
    }
}

impl<P: CubicExtParameters> CanonicalSerializeWithFlags for CubicExtField<P> {
    #[inline]
    fn serialize_with_flags<W: Write, F: Flags>(
        &self,
        mut writer: W,
        flags: F,
    ) -> Result<(), SerializationError> {
        self.c0.serialize(&mut writer)?;
        self.c1.serialize(&mut writer)?;
        self.c2.serialize_with_flags(&mut writer, flags)?;
        Ok(())
    }

    #[inline]
    fn serialized_size_with_flags<F: Flags>(&self) -> usize {
        self.c0.serialized_size()
            + self.c1.serialized_size()
            + self.c2.serialized_size_with_flags::<F>()
    }
}

impl<P: CubicExtParameters> CanonicalSerialize for CubicExtField<P> {
    #[inline]
    fn serialize<W: Write>(&self, writer: W) -> Result<(), SerializationError> {
        self.serialize_with_flags(writer, EmptyFlags)
    }

    #[inline]
    fn serialized_size(&self) -> usize {
        self.serialized_size_with_flags::<EmptyFlags>()
    }
}

impl<P: CubicExtParameters> CanonicalDeserializeWithFlags for CubicExtField<P> {
    #[inline]
    fn deserialize_with_flags<R: Read, F: Flags>(
        mut reader: R,
    ) -> Result<(Self, F), SerializationError> {
        let c0 = CanonicalDeserialize::deserialize(&mut reader)?;
        let c1 = CanonicalDeserialize::deserialize(&mut reader)?;
        let (c2, flags) = CanonicalDeserializeWithFlags::deserialize_with_flags(&mut reader)?;
        Ok((CubicExtField::new(c0, c1, c2), flags))
    }
}

impl<P: CubicExtParameters> CanonicalDeserialize for CubicExtField<P> {
    #[inline]
    fn deserialize<R: Read>(mut reader: R) -> Result<Self, SerializationError> {
        let c0: P::BaseField = CanonicalDeserialize::deserialize(&mut reader)?;
        let c1: P::BaseField = CanonicalDeserialize::deserialize(&mut reader)?;
        let c2: P::BaseField = CanonicalDeserialize::deserialize(&mut reader)?;
        Ok(CubicExtField::new(c0, c1, c2))
    }
}

impl<P: CubicExtParameters> ToConstraintField<P::BasePrimeField> for CubicExtField<P>
where
    P::BaseField: ToConstraintField<P::BasePrimeField>,
{
    fn to_field_elements(&self) -> Option<Vec<P::BasePrimeField>> {
        let mut res = Vec::new();
        let mut c0_elems = self.c0.to_field_elements()?;
        let mut c1_elems = self.c1.to_field_elements()?;
        let mut c2_elems = self.c2.to_field_elements()?;

        res.append(&mut c0_elems);
        res.append(&mut c1_elems);
        res.append(&mut c2_elems);

        Some(res)
    }
}

#[cfg(test)]
mod cube_ext_tests {
    use super::*;
    use crate::test_field::{Fq, Fq2, Fq6};
    use ark_std::test_rng;

    #[test]
    fn test_from_base_prime_field_elements() {
        let ext_degree = Fq6::extension_degree() as usize;
        // Test on slice lengths that aren't equal to the extension degree
        let max_num_elems_to_test = 10;
        for d in 0..max_num_elems_to_test {
            if d == ext_degree {
                continue;
            }
            let mut random_coeffs = Vec::<Fq>::new();
            for _ in 0..d {
                random_coeffs.push(Fq::rand(&mut test_rng()));
            }
            let res = Fq6::from_base_prime_field_elems(&random_coeffs);
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
            let actual = Fq6::from_base_prime_field_elems(&random_coeffs).unwrap();

            let expected_0 = Fq2::new(random_coeffs[0], random_coeffs[1]);
            let expected_1 = Fq2::new(random_coeffs[2], random_coeffs[3]);
            let expected_2 = Fq2::new(random_coeffs[3], random_coeffs[4]);
            let expected = Fq6::new(expected_0, expected_1, expected_2);
            assert_eq!(actual, expected);
        }
    }
}
