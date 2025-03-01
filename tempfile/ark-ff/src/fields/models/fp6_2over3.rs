use ark_std::Zero;

use super::quadratic_extension::*;
use core::{
    marker::PhantomData,
    ops::{MulAssign, Not},
};

use crate::{
    fields::{Fp3, Fp3Config},
    CyclotomicMultSubgroup,
};

pub trait Fp6Config: 'static + Send + Sync {
    type Fp3Config: Fp3Config;

    const NONRESIDUE: Fp3<Self::Fp3Config>;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_FP6_C1: &'static [<Self::Fp3Config as Fp3Config>::Fp];

    #[inline(always)]
    fn mul_fp3_by_nonresidue_in_place(fe: &mut Fp3<Self::Fp3Config>) -> &mut Fp3<Self::Fp3Config> {
        let old_c1 = fe.c1;
        fe.c1 = fe.c0;
        fe.c0 = fe.c2;
        <Self::Fp3Config as Fp3Config>::mul_fp_by_nonresidue_in_place(&mut fe.c0);
        fe.c2 = old_c1;
        fe
    }
}

pub struct Fp6ConfigWrapper<P: Fp6Config>(PhantomData<P>);

impl<P: Fp6Config> QuadExtConfig for Fp6ConfigWrapper<P> {
    type BasePrimeField = <P::Fp3Config as Fp3Config>::Fp;
    type BaseField = Fp3<P::Fp3Config>;
    type FrobCoeff = Self::BasePrimeField;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 6;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP6_C1;

    #[inline(always)]
    fn mul_base_field_by_nonresidue_in_place(fe: &mut Self::BaseField) -> &mut Self::BaseField {
        P::mul_fp3_by_nonresidue_in_place(fe);
        fe
    }

    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize) {
        fe.mul_assign_by_fp(&Self::FROBENIUS_COEFF_C1[power % Self::DEGREE_OVER_BASE_PRIME_FIELD]);
    }
}

pub type Fp6<P> = QuadExtField<Fp6ConfigWrapper<P>>;

impl<P: Fp6Config> Fp6<P> {
    pub fn mul_by_034(
        &mut self,
        c0: &<P::Fp3Config as Fp3Config>::Fp,
        c3: &<P::Fp3Config as Fp3Config>::Fp,
        c4: &<P::Fp3Config as Fp3Config>::Fp,
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
        tmp1.mul_assign(&<P::Fp3Config as Fp3Config>::NONRESIDUE);
        let mut tmp2 = x4;
        tmp2.mul_assign(&<P::Fp3Config as Fp3Config>::NONRESIDUE);

        self.c0.c0 = x0 * &z0 + &(tmp1 * &z5) + &(tmp2 * &z4);
        self.c0.c1 = x0 * &z1 + &(x3 * &z3) + &(tmp2 * &z5);
        self.c0.c2 = x0 * &z2 + &(x3 * &z4) + &(x4 * &z3);
        self.c1.c0 = x0 * &z3 + &(x3 * &z0) + &(tmp2 * &z2);
        self.c1.c1 = x0 * &z4 + &(x3 * &z1) + &(x4 * &z0);
        self.c1.c2 = x0 * &z5 + &(x3 * &z2) + &(x4 * &z1);
    }

    pub fn mul_by_014(
        &mut self,
        c0: &<P::Fp3Config as Fp3Config>::Fp,
        c1: &<P::Fp3Config as Fp3Config>::Fp,
        c4: &<P::Fp3Config as Fp3Config>::Fp,
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
        tmp1.mul_assign(&<P::Fp3Config as Fp3Config>::NONRESIDUE);
        let mut tmp2 = x4;
        tmp2.mul_assign(&<P::Fp3Config as Fp3Config>::NONRESIDUE);

        self.c0.c0 = x0 * &z0 + &(tmp1 * &z2) + &(tmp2 * &z4);
        self.c0.c1 = x0 * &z1 + &(x1 * &z0) + &(tmp2 * &z5);
        self.c0.c2 = x0 * &z2 + &(x1 * &z1) + &(x4 * &z3);
        self.c1.c0 = x0 * &z3 + &(tmp1 * &z5) + &(tmp2 * &z2);
        self.c1.c1 = x0 * &z4 + &(x1 * &z3) + &(x4 * &z0);
        self.c1.c2 = x0 * &z5 + &(x1 * &z4) + &(x4 * &z1);
    }
}

impl<P: Fp6Config> CyclotomicMultSubgroup for Fp6<P> {
    const INVERSE_IS_FAST: bool = true;
    fn cyclotomic_inverse_in_place(&mut self) -> Option<&mut Self> {
        self.is_zero().not().then(|| {
            self.conjugate_in_place();
            self
        })
    }
}
