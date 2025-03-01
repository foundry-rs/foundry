use super::cubic_extension::*;
use crate::fields::*;
use core::marker::PhantomData;

pub trait Fp6Parameters: 'static + Send + Sync + Copy {
    type Fp2Params: Fp2Parameters;

    const NONRESIDUE: Fp2<Self::Fp2Params>;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_FP6_C1: &'static [Fp2<Self::Fp2Params>];
    const FROBENIUS_COEFF_FP6_C2: &'static [Fp2<Self::Fp2Params>];

    #[inline(always)]
    fn mul_fp2_by_nonresidue(fe: &Fp2<Self::Fp2Params>) -> Fp2<Self::Fp2Params> {
        Self::NONRESIDUE * fe
    }
}

pub struct Fp6ParamsWrapper<P: Fp6Parameters>(PhantomData<P>);

impl<P: Fp6Parameters> CubicExtParameters for Fp6ParamsWrapper<P> {
    type BasePrimeField = <P::Fp2Params as Fp2Parameters>::Fp;
    type BaseField = Fp2<P::Fp2Params>;
    type FrobCoeff = Fp2<P::Fp2Params>;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 6;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP6_C1;
    const FROBENIUS_COEFF_C2: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP6_C2;

    #[inline(always)]
    fn mul_base_field_by_nonresidue(fe: &Self::BaseField) -> Self::BaseField {
        P::mul_fp2_by_nonresidue(fe)
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

pub type Fp6<P> = CubicExtField<Fp6ParamsWrapper<P>>;

impl<P: Fp6Parameters> Fp6<P> {
    pub fn mul_assign_by_fp2(&mut self, other: Fp2<P::Fp2Params>) {
        self.c0 *= &other;
        self.c1 *= &other;
        self.c2 *= &other;
    }

    pub fn mul_by_fp(&mut self, element: &<P::Fp2Params as Fp2Parameters>::Fp) {
        self.c0.mul_assign_by_fp(&element);
        self.c1.mul_assign_by_fp(&element);
        self.c2.mul_assign_by_fp(&element);
    }

    pub fn mul_by_fp2(&mut self, element: &Fp2<P::Fp2Params>) {
        self.c0.mul_assign(element);
        self.c1.mul_assign(element);
        self.c2.mul_assign(element);
    }

    pub fn mul_by_1(&mut self, c1: &Fp2<P::Fp2Params>) {
        let mut b_b = self.c1;
        b_b.mul_assign(c1);

        let mut t1 = *c1;
        {
            let mut tmp = self.c1;
            tmp.add_assign(&self.c2);

            t1.mul_assign(&tmp);
            t1.sub_assign(&b_b);
            t1 = P::mul_fp2_by_nonresidue(&t1);
        }

        let mut t2 = *c1;
        {
            let mut tmp = self.c0;
            tmp.add_assign(&self.c1);

            t2.mul_assign(&tmp);
            t2.sub_assign(&b_b);
        }

        self.c0 = t1;
        self.c1 = t2;
        self.c2 = b_b;
    }

    pub fn mul_by_01(&mut self, c0: &Fp2<P::Fp2Params>, c1: &Fp2<P::Fp2Params>) {
        let mut a_a = self.c0;
        let mut b_b = self.c1;
        a_a.mul_assign(c0);
        b_b.mul_assign(c1);

        let mut t1 = *c1;
        {
            let mut tmp = self.c1;
            tmp.add_assign(&self.c2);

            t1.mul_assign(&tmp);
            t1.sub_assign(&b_b);
            t1 = P::mul_fp2_by_nonresidue(&t1);
            t1.add_assign(&a_a);
        }

        let mut t3 = *c0;
        {
            let mut tmp = self.c0;
            tmp.add_assign(&self.c2);

            t3.mul_assign(&tmp);
            t3.sub_assign(&a_a);
            t3.add_assign(&b_b);
        }

        let mut t2 = *c0;
        t2.add_assign(c1);
        {
            let mut tmp = self.c0;
            tmp.add_assign(&self.c1);

            t2.mul_assign(&tmp);
            t2.sub_assign(&a_a);
            t2.sub_assign(&b_b);
        }

        self.c0 = t1;
        self.c1 = t2;
        self.c2 = t3;
    }
}
