use super::cubic_extension::*;
use crate::fields::*;
use core::marker::PhantomData;

/// Trait that specifies constants and methods for defining degree-three extension fields.
pub trait Fp3Config: 'static + Send + Sync + Sized {
    /// Base prime field underlying this extension.
    type Fp: PrimeField;
    /// Cubic non-residue in `Self::Fp` used to construct the extension
    /// field. That is, `NONRESIDUE` is such that the cubic polynomial
    /// `f(X) = X^3 - Self::NONRESIDUE` in Fp\[X\] is irreducible in `Self::Fp`.
    const NONRESIDUE: Self::Fp;

    const FROBENIUS_COEFF_FP3_C1: &'static [Self::Fp];
    const FROBENIUS_COEFF_FP3_C2: &'static [Self::Fp];

    /// p^3 - 1 = 2^s * t, where t is odd.
    const TWO_ADICITY: u32;
    const TRACE_MINUS_ONE_DIV_TWO: &'static [u64];
    /// t-th power of a quadratic nonresidue in Fp3.
    const QUADRATIC_NONRESIDUE_TO_T: Fp3<Self>;

    /// Return `fe * Self::NONRESIDUE`.
    /// The default implementation can be specialized if [`Self::NONRESIDUE`] has a special
    /// structure that can speed up multiplication
    #[inline(always)]
    fn mul_fp_by_nonresidue_in_place(fe: &mut Self::Fp) -> &mut Self::Fp {
        *fe *= Self::NONRESIDUE;
        fe
    }
}

/// Wrapper for [`Fp3Config`], allowing combination of the [`Fp3Config`] and [`CubicExtConfig`] traits.
pub struct Fp3ConfigWrapper<P: Fp3Config>(PhantomData<P>);

impl<P: Fp3Config> CubicExtConfig for Fp3ConfigWrapper<P> {
    type BasePrimeField = P::Fp;
    type BaseField = P::Fp;
    type FrobCoeff = P::Fp;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 3;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const SQRT_PRECOMP: Option<SqrtPrecomputation<CubicExtField<Self>>> =
        Some(SqrtPrecomputation::TonelliShanks {
            two_adicity: P::TWO_ADICITY,
            quadratic_nonresidue_to_trace: P::QUADRATIC_NONRESIDUE_TO_T,
            trace_of_modulus_minus_one_div_two: P::TRACE_MINUS_ONE_DIV_TWO,
        });

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP3_C1;
    const FROBENIUS_COEFF_C2: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP3_C2;

    #[inline(always)]
    fn mul_base_field_by_nonresidue_in_place(fe: &mut Self::BaseField) -> &mut Self::BaseField {
        P::mul_fp_by_nonresidue_in_place(fe)
    }

    fn mul_base_field_by_frob_coeff(
        c1: &mut Self::BaseField,
        c2: &mut Self::BaseField,
        power: usize,
    ) {
        *c1 *= &Self::FROBENIUS_COEFF_C1[power % Self::DEGREE_OVER_BASE_PRIME_FIELD];
        *c2 *= &Self::FROBENIUS_COEFF_C2[power % Self::DEGREE_OVER_BASE_PRIME_FIELD];
    }
}

pub type Fp3<P> = CubicExtField<Fp3ConfigWrapper<P>>;

impl<P: Fp3Config> Fp3<P> {
    /// In-place multiply all coefficients `c0`, `c1`, and `c2` of `self`
    /// by an element from [`Fp`](`Fp3Config::Fp`).
    ///
    /// # Examples
    ///
    /// ```
    /// # use ark_std::test_rng;
    /// # use ark_std::UniformRand;
    /// # use ark_test_curves::mnt6_753 as ark_mnt6_753;
    /// use ark_mnt6_753::{Fq as Fp, Fq3 as Fp3};
    /// let c0: Fp = Fp::rand(&mut test_rng());
    /// let c1: Fp = Fp::rand(&mut test_rng());
    /// let c2: Fp = Fp::rand(&mut test_rng());
    /// let mut ext_element: Fp3 = Fp3::new(c0, c1, c2);
    ///
    /// let base_field_element: Fp = Fp::rand(&mut test_rng());
    /// ext_element.mul_assign_by_fp(&base_field_element);
    ///
    /// assert_eq!(ext_element.c0, c0 * base_field_element);
    /// assert_eq!(ext_element.c1, c1 * base_field_element);
    /// assert_eq!(ext_element.c2, c2 * base_field_element);
    /// ```
    pub fn mul_assign_by_fp(&mut self, value: &P::Fp) {
        self.c0.mul_assign(value);
        self.c1.mul_assign(value);
        self.c2.mul_assign(value);
    }
}

// We just use the default algorithms; there don't seem to be any faster ones.
impl<P: Fp3Config> CyclotomicMultSubgroup for Fp3<P> {}
