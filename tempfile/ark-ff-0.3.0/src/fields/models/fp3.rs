use super::cubic_extension::*;
use crate::fields::*;
use core::marker::PhantomData;

pub trait Fp3Parameters: 'static + Send + Sync {
    type Fp: PrimeField + SquareRootField;

    const NONRESIDUE: Self::Fp;

    const FROBENIUS_COEFF_FP3_C1: &'static [Self::Fp];
    const FROBENIUS_COEFF_FP3_C2: &'static [Self::Fp];

    /// p^3 - 1 = 2^s * t, where t is odd.
    const TWO_ADICITY: u32;
    const T_MINUS_ONE_DIV_TWO: &'static [u64];
    /// t-th power of a quadratic nonresidue in Fp3.
    const QUADRATIC_NONRESIDUE_TO_T: (Self::Fp, Self::Fp, Self::Fp);

    #[inline(always)]
    fn mul_fp_by_nonresidue(fe: &Self::Fp) -> Self::Fp {
        Self::NONRESIDUE * fe
    }
}

pub struct Fp3ParamsWrapper<P: Fp3Parameters>(PhantomData<P>);

impl<P: Fp3Parameters> CubicExtParameters for Fp3ParamsWrapper<P> {
    type BasePrimeField = P::Fp;
    type BaseField = P::Fp;
    type FrobCoeff = P::Fp;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 3;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP3_C1;
    const FROBENIUS_COEFF_C2: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP3_C2;

    #[inline(always)]
    fn mul_base_field_by_nonresidue(fe: &Self::BaseField) -> Self::BaseField {
        P::mul_fp_by_nonresidue(fe)
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

pub type Fp3<P> = CubicExtField<Fp3ParamsWrapper<P>>;

impl<P: Fp3Parameters> Fp3<P> {
    pub fn mul_assign_by_fp(&mut self, value: &P::Fp) {
        self.c0.mul_assign(value);
        self.c1.mul_assign(value);
        self.c2.mul_assign(value);
    }

    /// Returns the value of QNR^T.
    #[inline]
    pub fn qnr_to_t() -> Self {
        Self::new(
            P::QUADRATIC_NONRESIDUE_TO_T.0,
            P::QUADRATIC_NONRESIDUE_TO_T.1,
            P::QUADRATIC_NONRESIDUE_TO_T.2,
        )
    }
}

impl<P: Fp3Parameters> SquareRootField for Fp3<P> {
    /// Returns the Legendre symbol.
    fn legendre(&self) -> LegendreSymbol {
        self.norm().legendre()
    }

    /// Returns the square root of self, if it exists.
    fn sqrt(&self) -> Option<Self> {
        sqrt_impl!(Self, P, self)
    }

    /// Sets `self` to be the square root of `self`, if it exists.
    fn sqrt_in_place(&mut self) -> Option<&mut Self> {
        (*self).sqrt().map(|sqrt| {
            *self = sqrt;
            self
        })
    }
}
