use ark_std::Zero;

use super::quadratic_extension::*;
use crate::{fields::PrimeField, CyclotomicMultSubgroup};
use core::{marker::PhantomData, ops::Not};

/// Trait that specifies constants and methods for defining degree-two extension fields.
pub trait Fp2Config: 'static + Send + Sync + Sized {
    /// Base prime field underlying this extension.
    type Fp: PrimeField;

    /// Quadratic non-residue in [`Self::Fp`] used to construct the extension
    /// field. That is, `NONRESIDUE` is such that the quadratic polynomial
    /// `f(X) = X^2 - Self::NONRESIDUE` in Fp\[X\] is irreducible in `Self::Fp`.
    const NONRESIDUE: Self::Fp;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_FP2_C1: &'static [Self::Fp];

    /// Return `fe * Self::NONRESIDUE`.
    /// Intended for specialization when [`Self::NONRESIDUE`] has a special
    /// structure that can speed up multiplication
    #[inline(always)]
    fn mul_fp_by_nonresidue_in_place(fe: &mut Self::Fp) -> &mut Self::Fp {
        *fe *= Self::NONRESIDUE;
        fe
    }

    /// A specializable method for setting `y = x + NONRESIDUE * y`.
    /// This allows for optimizations when the non-residue is
    /// canonically negative in the field.
    #[inline(always)]
    fn mul_fp_by_nonresidue_and_add(y: &mut Self::Fp, x: &Self::Fp) {
        Self::mul_fp_by_nonresidue_in_place(y);
        *y += x;
    }

    /// A specializable method for computing x + mul_fp_by_nonresidue(y) + y
    /// This allows for optimizations when the non-residue is not -1.
    #[inline(always)]
    fn mul_fp_by_nonresidue_plus_one_and_add(y: &mut Self::Fp, x: &Self::Fp) {
        let old_y = *y;
        Self::mul_fp_by_nonresidue_and_add(y, x);
        *y += old_y;
    }

    /// A specializable method for computing x - mul_fp_by_nonresidue(y)
    /// This allows for optimizations when the non-residue is
    /// canonically negative in the field.
    #[inline(always)]
    fn sub_and_mul_fp_by_nonresidue(y: &mut Self::Fp, x: &Self::Fp) {
        *y = *x - Self::mul_fp_by_nonresidue_in_place(y);
    }
}

/// Wrapper for [`Fp2Config`], allowing combination of the [`Fp2Config`] and [`QuadExtConfig`] traits.
pub struct Fp2ConfigWrapper<P: Fp2Config>(PhantomData<P>);

impl<P: Fp2Config> QuadExtConfig for Fp2ConfigWrapper<P> {
    type BasePrimeField = P::Fp;
    type BaseField = P::Fp;
    type FrobCoeff = P::Fp;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 2;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP2_C1;

    #[inline(always)]
    fn mul_base_field_by_nonresidue_in_place(fe: &mut Self::BaseField) -> &mut Self::BaseField {
        P::mul_fp_by_nonresidue_in_place(fe)
    }

    #[inline(always)]
    fn mul_base_field_by_nonresidue_and_add(y: &mut Self::BaseField, x: &Self::BaseField) {
        P::mul_fp_by_nonresidue_and_add(y, x)
    }

    #[inline(always)]
    fn mul_base_field_by_nonresidue_plus_one_and_add(y: &mut Self::BaseField, x: &Self::BaseField) {
        P::mul_fp_by_nonresidue_plus_one_and_add(y, x)
    }

    #[inline(always)]
    fn sub_and_mul_base_field_by_nonresidue(y: &mut Self::BaseField, x: &Self::BaseField) {
        P::sub_and_mul_fp_by_nonresidue(y, x)
    }

    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize) {
        *fe *= &Self::FROBENIUS_COEFF_C1[power % Self::DEGREE_OVER_BASE_PRIME_FIELD];
    }
}

/// Alias for instances of quadratic extension fields. Helpful for omitting verbose
/// instantiations involving `Fp2ConfigWrapper`.
pub type Fp2<P> = QuadExtField<Fp2ConfigWrapper<P>>;

impl<P: Fp2Config> Fp2<P> {
    /// In-place multiply both coefficients `c0` and `c1` of `self`
    /// by an element from [`Fp`](`Fp2Config::Fp`).
    ///
    /// # Examples
    ///
    /// ```
    /// # use ark_std::test_rng;
    /// # use ark_test_curves::bls12_381::{Fq as Fp, Fq2 as Fp2};
    /// # use ark_std::UniformRand;
    /// let c0: Fp = Fp::rand(&mut test_rng());
    /// let c1: Fp = Fp::rand(&mut test_rng());
    /// let mut ext_element: Fp2 = Fp2::new(c0, c1);
    ///
    /// let base_field_element: Fp = Fp::rand(&mut test_rng());
    /// ext_element.mul_assign_by_fp(&base_field_element);
    ///
    /// assert_eq!(ext_element.c0, c0 * base_field_element);
    /// assert_eq!(ext_element.c1, c1 * base_field_element);
    /// ```
    pub fn mul_assign_by_fp(&mut self, other: &P::Fp) {
        self.c0 *= other;
        self.c1 *= other;
    }
}

impl<P: Fp2Config> CyclotomicMultSubgroup for Fp2<P> {
    const INVERSE_IS_FAST: bool = true;
    fn cyclotomic_inverse_in_place(&mut self) -> Option<&mut Self> {
        // As the multiplicative subgroup is of order p^2 - 1, the
        // only non-trivial cyclotomic subgroup is of order p+1
        // Therefore, for any element in the cyclotomic subgroup, we have that `x^(p+1) = 1`.
        // Recall that `x^(p+1)` in a quadratic extension field is equal
        // to the norm in the base field, so we have that
        // `x * x.conjugate() = 1`. By uniqueness of inverses,
        // for this subgroup, x.inverse() = x.conjugate()

        self.is_zero().not().then(|| {
            self.conjugate_in_place();
            self
        })
    }
}
