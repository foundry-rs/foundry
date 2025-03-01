use super::quadratic_extension::*;
use core::marker::PhantomData;
use core::ops::MulAssign;

use crate::fields::{Fp3, Fp3Parameters};

pub trait Fp6Parameters: 'static + Send + Sync {
    type Fp3Params: Fp3Parameters;

    const NONRESIDUE: Fp3<Self::Fp3Params>;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_FP6_C1: &'static [<Self::Fp3Params as Fp3Parameters>::Fp];

    #[inline(always)]
    fn mul_fp3_by_nonresidue(fe: &Fp3<Self::Fp3Params>) -> Fp3<Self::Fp3Params> {
        let mut res = *fe;
        res.c0 = fe.c2;
        res.c1 = fe.c0;
        res.c2 = fe.c1;
        res.c0 = <Self::Fp3Params as Fp3Parameters>::mul_fp_by_nonresidue(&res.c0);
        res
    }
}

pub struct Fp6ParamsWrapper<P: Fp6Parameters>(PhantomData<P>);

impl<P: Fp6Parameters> QuadExtParameters for Fp6ParamsWrapper<P> {
    type BasePrimeField = <P::Fp3Params as Fp3Parameters>::Fp;
    type BaseField = Fp3<P::Fp3Params>;
    type FrobCoeff = Self::BasePrimeField;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 6;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP6_C1;

    #[inline(always)]
    fn mul_base_field_by_nonresidue(fe: &Self::BaseField) -> Self::BaseField {
        P::mul_fp3_by_nonresidue(fe)
    }

    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize) {
        fe.mul_assign_by_fp(&Self::FROBENIUS_COEFF_C1[power % Self::DEGREE_OVER_BASE_PRIME_FIELD]);
    }
}

pub type Fp6<P> = QuadExtField<Fp6ParamsWrapper<P>>;

impl<P: Fp6Parameters> Fp6<P> {
    pub fn mul_by_034(
        &mut self,
        c0: &<P::Fp3Params as Fp3Parameters>::Fp,
        c3: &<P::Fp3Params as Fp3Parameters>::Fp,
        c4: &<P::Fp3Params as Fp3Parameters>::Fp,
    ) {
        let z0 = self.c0.c0;
        let z1 = self.c0.c1;
        let z2 = self.c0.c2;
        let z3 = self.c1.c0;
        let z4 = self.c1.c1;
        let z5 = self.c1.c2;

        let x0 = *c0;
        let x3 = *c3;
        let x4 = *c4;

        let mut tmp1 = x3;
        tmp1.mul_assign(&<P::Fp3Params as Fp3Parameters>::NONRESIDUE);
        let mut tmp2 = x4;
        tmp2.mul_assign(&<P::Fp3Params as Fp3Parameters>::NONRESIDUE);

        self.c0.c0 = x0 * &z0 + &(tmp1 * &z5) + &(tmp2 * &z4);
        self.c0.c1 = x0 * &z1 + &(x3 * &z3) + &(tmp2 * &z5);
        self.c0.c2 = x0 * &z2 + &(x3 * &z4) + &(x4 * &z3);
        self.c1.c0 = x0 * &z3 + &(x3 * &z0) + &(tmp2 * &z2);
        self.c1.c1 = x0 * &z4 + &(x3 * &z1) + &(x4 * &z0);
        self.c1.c2 = x0 * &z5 + &(x3 * &z2) + &(x4 * &z1);
    }

    pub fn mul_by_014(
        &mut self,
        c0: &<P::Fp3Params as Fp3Parameters>::Fp,
        c1: &<P::Fp3Params as Fp3Parameters>::Fp,
        c4: &<P::Fp3Params as Fp3Parameters>::Fp,
    ) {
        let z0 = self.c0.c0;
        let z1 = self.c0.c1;
        let z2 = self.c0.c2;
        let z3 = self.c1.c0;
        let z4 = self.c1.c1;
        let z5 = self.c1.c2;

        let x0 = *c0;
        let x1 = *c1;
        let x4 = *c4;

        let mut tmp1 = x1;
        tmp1.mul_assign(&<P::Fp3Params as Fp3Parameters>::NONRESIDUE);
        let mut tmp2 = x4;
        tmp2.mul_assign(&<P::Fp3Params as Fp3Parameters>::NONRESIDUE);

        self.c0.c0 = x0 * &z0 + &(tmp1 * &z2) + &(tmp2 * &z4);
        self.c0.c1 = x0 * &z1 + &(x1 * &z0) + &(tmp2 * &z5);
        self.c0.c2 = x0 * &z2 + &(x1 * &z1) + &(x4 * &z3);
        self.c1.c0 = x0 * &z3 + &(tmp1 * &z5) + &(tmp2 * &z2);
        self.c1.c1 = x0 * &z4 + &(x1 * &z3) + &(x4 * &z0);
        self.c1.c2 = x0 * &z5 + &(x1 * &z4) + &(x4 * &z1);
    }
}
