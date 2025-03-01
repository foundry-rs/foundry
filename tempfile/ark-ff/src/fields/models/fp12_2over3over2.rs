use ark_std::Zero;

use super::quadratic_extension::*;
use crate::{
    fields::{fp6_3over2::*, Field, Fp2, Fp2Config as Fp2ConfigTrait},
    CyclotomicMultSubgroup,
};
use core::{
    marker::PhantomData,
    ops::{AddAssign, Not, SubAssign},
};

type Fp2Config<P> = <<P as Fp12Config>::Fp6Config as Fp6Config>::Fp2Config;

pub trait Fp12Config: 'static + Send + Sync + Copy {
    type Fp6Config: Fp6Config;

    /// This *must* equal (0, 1, 0);
    /// see [[DESD06, Section 6.1]](https://eprint.iacr.org/2006/471.pdf).
    const NONRESIDUE: Fp6<Self::Fp6Config>;

    /// Coefficients for the Frobenius automorphism.
    const FROBENIUS_COEFF_FP12_C1: &'static [Fp2<Fp2Config<Self>>];

    /// Multiply by quadratic nonresidue v.
    #[inline(always)]
    fn mul_fp6_by_nonresidue_in_place(fe: &mut Fp6<Self::Fp6Config>) -> &mut Fp6<Self::Fp6Config> {
        // see [[DESD06, Section 6.1]](https://eprint.iacr.org/2006/471.pdf).
        let old_c1 = fe.c1;
        fe.c1 = fe.c0;
        fe.c0 = fe.c2;
        Self::Fp6Config::mul_fp2_by_nonresidue_in_place(&mut fe.c0);
        fe.c2 = old_c1;
        fe
    }
}

pub struct Fp12ConfigWrapper<P: Fp12Config>(PhantomData<P>);

impl<P: Fp12Config> QuadExtConfig for Fp12ConfigWrapper<P> {
    type BasePrimeField = <Fp2Config<P> as Fp2ConfigTrait>::Fp;
    type BaseField = Fp6<P::Fp6Config>;
    type FrobCoeff = Fp2<Fp2Config<P>>;

    const DEGREE_OVER_BASE_PRIME_FIELD: usize = 12;

    const NONRESIDUE: Self::BaseField = P::NONRESIDUE;

    const FROBENIUS_COEFF_C1: &'static [Self::FrobCoeff] = P::FROBENIUS_COEFF_FP12_C1;

    #[inline(always)]
    fn mul_base_field_by_nonresidue_in_place(fe: &mut Self::BaseField) -> &mut Self::BaseField {
        P::mul_fp6_by_nonresidue_in_place(fe)
    }

    fn mul_base_field_by_frob_coeff(fe: &mut Self::BaseField, power: usize) {
        fe.mul_assign_by_fp2(Self::FROBENIUS_COEFF_C1[power % Self::DEGREE_OVER_BASE_PRIME_FIELD]);
    }
}

pub type Fp12<P> = QuadExtField<Fp12ConfigWrapper<P>>;

impl<P: Fp12Config> Fp12<P> {
    pub fn mul_by_fp(&mut self, element: &<Self as Field>::BasePrimeField) {
        self.c0.mul_by_fp(element);
        self.c1.mul_by_fp(element);
    }

    pub fn mul_by_034(
        &mut self,
        c0: &Fp2<Fp2Config<P>>,
        c3: &Fp2<Fp2Config<P>>,
        c4: &Fp2<Fp2Config<P>>,
    ) {
        let a0 = self.c0.c0 * c0;
        let a1 = self.c0.c1 * c0;
        let a2 = self.c0.c2 * c0;
        let a = Fp6::new(a0, a1, a2);
        let mut b = self.c1;
        b.mul_by_01(c3, c4);

        let c0 = *c0 + c3;
        let c1 = c4;
        let mut e = self.c0 + &self.c1;
        e.mul_by_01(&c0, c1);
        self.c1 = e - &(a + &b);
        self.c0 = b;
        P::mul_fp6_by_nonresidue_in_place(&mut self.c0);
        self.c0 += &a;
    }

    pub fn mul_by_014(
        &mut self,
        c0: &Fp2<Fp2Config<P>>,
        c1: &Fp2<Fp2Config<P>>,
        c4: &Fp2<Fp2Config<P>>,
    ) {
        let mut aa = self.c0;
        aa.mul_by_01(c0, c1);
        let mut bb = self.c1;
        bb.mul_by_1(c4);
        let mut o = *c1;
        o.add_assign(c4);
        self.c1.add_assign(&self.c0);
        self.c1.mul_by_01(c0, &o);
        self.c1.sub_assign(&aa);
        self.c1.sub_assign(&bb);
        self.c0 = bb;
        P::mul_fp6_by_nonresidue_in_place(&mut self.c0);
        self.c0.add_assign(&aa);
    }
}

pub const fn characteristic_square_mod_6_is_one(characteristic: &[u64]) -> bool {
    // char mod 6 = (a_0 + 2**64 * a_1 + ...) mod 6
    //            = a_0 mod 6 + (2**64 * a_1 mod 6) + (...) mod 6
    //            = a_0 mod 6 + (4 * a_1 mod 6) + (4 * ...) mod 6
    let mut char_mod_6 = 0u64;
    crate::const_for!((i in 0..(characteristic.len())) {
        char_mod_6 += if i == 0 {
            characteristic[i] % 6
        } else {
            (4 * (characteristic[i] % 6)) % 6
        };
    });
    (char_mod_6 * char_mod_6) % 6 == 1
}

impl<P: Fp12Config> CyclotomicMultSubgroup for Fp12<P> {
    const INVERSE_IS_FAST: bool = true;

    fn cyclotomic_inverse_in_place(&mut self) -> Option<&mut Self> {
        self.is_zero().not().then(|| self.conjugate_in_place())
    }

    fn cyclotomic_square_in_place(&mut self) -> &mut Self {
        // Faster Squaring in the Cyclotomic Subgroup of Sixth Degree Extensions
        // - Robert Granger and Michael Scott
        //
        if characteristic_square_mod_6_is_one(Self::characteristic()) {
            let fp2_nr = <P::Fp6Config as Fp6Config>::mul_fp2_by_nonresidue;

            let r0 = &self.c0.c0;
            let r4 = &self.c0.c1;
            let r3 = &self.c0.c2;
            let r2 = &self.c1.c0;
            let r1 = &self.c1.c1;
            let r5 = &self.c1.c2;

            // t0 + t1*y = (z0 + z1*y)^2 = a^2
            let mut tmp = *r0 * r1;
            let t0 = (*r0 + r1) * &(fp2_nr(*r1) + r0) - &tmp - &fp2_nr(tmp);
            let t1 = tmp.double();

            // t2 + t3*y = (z2 + z3*y)^2 = b^2
            tmp = *r2 * r3;
            let t2 = (*r2 + r3) * &(fp2_nr(*r3) + r2) - &tmp - &fp2_nr(tmp);
            let t3 = tmp.double();

            // t4 + t5*y = (z4 + z5*y)^2 = c^2
            tmp = *r4 * r5;
            let t4 = (*r4 + r5) * &(fp2_nr(*r5) + r4) - &tmp - &fp2_nr(tmp);
            let t5 = tmp.double();

            let z0 = &mut self.c0.c0;
            let z4 = &mut self.c0.c1;
            let z3 = &mut self.c0.c2;
            let z2 = &mut self.c1.c0;
            let z1 = &mut self.c1.c1;
            let z5 = &mut self.c1.c2;

            // for A

            // z0 = 3 * t0 - 2 * z0
            *z0 = t0 - &*z0;
            z0.double_in_place();
            *z0 += &t0;

            // z1 = 3 * t1 + 2 * z1
            *z1 = t1 + &*z1;
            z1.double_in_place();
            *z1 += &t1;

            // for B

            // z2 = 3 * (xi * t5) + 2 * z2
            tmp = fp2_nr(t5);
            *z2 += tmp;
            z2.double_in_place();
            *z2 += &tmp;

            // z3 = 3 * t4 - 2 * z3
            *z3 = t4 - &*z3;
            z3.double_in_place();
            *z3 += &t4;

            // for C

            // z4 = 3 * t2 - 2 * z4
            *z4 = t2 - &*z4;
            z4.double_in_place();
            *z4 += &t2;

            // z5 = 3 * t3 + 2 * z5
            *z5 += t3;
            z5.double_in_place();
            *z5 += &t3;
            self
        } else {
            self.square_in_place()
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_characteristic_square_mod_6_is_one() {
        use super::*;
        assert!(!characteristic_square_mod_6_is_one(&[36]));
        assert!(characteristic_square_mod_6_is_one(&[37]));
        assert!(!characteristic_square_mod_6_is_one(&[38]));
        assert!(!characteristic_square_mod_6_is_one(&[39]));
        assert!(!characteristic_square_mod_6_is_one(&[40]));
        assert!(characteristic_square_mod_6_is_one(&[41]));

        assert!(!characteristic_square_mod_6_is_one(&[36, 36]));
        assert!(!characteristic_square_mod_6_is_one(&[36, 37]));
        assert!(!characteristic_square_mod_6_is_one(&[36, 38]));
        assert!(!characteristic_square_mod_6_is_one(&[36, 39]));
        assert!(!characteristic_square_mod_6_is_one(&[36, 40]));
        assert!(!characteristic_square_mod_6_is_one(&[36, 41]));

        assert!(!characteristic_square_mod_6_is_one(&[36, 41]));
        assert!(!characteristic_square_mod_6_is_one(&[37, 41]));
        assert!(!characteristic_square_mod_6_is_one(&[38, 41]));
        assert!(characteristic_square_mod_6_is_one(&[39, 41]));
        assert!(!characteristic_square_mod_6_is_one(&[40, 41]));
        assert!(characteristic_square_mod_6_is_one(&[41, 41]));
        assert!(characteristic_square_mod_6_is_one(&[1, u64::MAX]));
    }
}
