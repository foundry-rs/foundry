use super::quadratic_extension::*;
use core::marker::PhantomData;

use crate::fields::{Fp2, Fp2Parameters};

pub trait Fp4Parameters: 'static + Send + Sync {
    type Fp2Params: Fp2Parameters;

    /// This *must* equal (0, 1);
    /// see [[DESD06, Section 5.1]](https://eprint.iacr.org/2006/471.pdf).
    const NONRESIDUE: Fp2<Self::Fp2Params>;

    /// Coefficients for the Frobenius automorphism.
    /// non_residue^((modulus^i-1)/4) for i=0,1,2,3
    const FROBENIUS_COEFF_FP4_C1: &'static [<Self::Fp2Params as Fp2Parameters>::Fp];

    #[inline(always)]
    fn mul_fp2_by_nonresidue(fe: &Fp2<Self::Fp2Params>) -> Fp2<Self::Fp2Params> {
        // see [[DESD06, Section 5.1]](https://eprint.iacr.org/2006/471.pdf).
        Fp2::new(
            <Self::Fp2Params as Fp2Parameters>::NONRESIDUE * &fe.c1,
            fe.c0,
        )
    }
}

pub struct Fp4ParamsWrapper<P: Fp4Parameters>(PhantomData<P>);

impl<P: Fp4Parameters> QuadExtParameters for Fp4ParamsWrapper<P> {
    type BasePrimeField = <P::Fp2Params as Fp2Parameters>::Fp;
    type BaseField = Fp2<P::Fp2Params>;
    type FrobCoeff = Self::BasePrimeField;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 4;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP4_C1;

    #[inline(always)]
    fn mul_base_field_by_nonresidue(fe: &Self::BaseField) -> Self::BaseField {
        P::mul_fp2_by_nonresidue(fe)
    }

    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize) {
        fe.mul_assign_by_fp(&Self::FROBENIUS_COEFF_C1[power % Self::DEGREE_OVER_BASE_PRIME_FIELD]);
    }
}

pub type Fp4<P> = QuadExtField<Fp4ParamsWrapper<P>>;

impl<P: Fp4Parameters> Fp4<P> {
    pub fn mul_by_fp(&mut self, element: &<P::Fp2Params as Fp2Parameters>::Fp) {
        self.c0.mul_assign_by_fp(element);
        self.c1.mul_assign_by_fp(element);
    }

    pub fn mul_by_fp2(&mut self, element: &Fp2<P::Fp2Params>) {
        self.c0 *= element;
        self.c1 *= element;
    }
}
