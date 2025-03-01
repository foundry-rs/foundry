use crate::{biginteger::BigInteger, Field, FpParameters, PrimeField};
use ark_std::vec::Vec;

/// Types that can be converted to a vector of `F` elements. Useful for
/// specifying how public inputs to a constraint system should be represented
/// inside that constraint system.
pub trait ToConstraintField<F: Field> {
    fn to_field_elements(&self) -> Option<Vec<F>>;
}

impl<F: Field> ToConstraintField<F> for bool {
    fn to_field_elements(&self) -> Option<Vec<F>> {
        if *self {
            Some(vec![F::one()])
        } else {
            Some(vec![F::zero()])
        }
    }
}

impl<F: PrimeField> ToConstraintField<F> for F {
    fn to_field_elements(&self) -> Option<Vec<F>> {
        Some(vec![*self])
    }
}

// Impl for base field
impl<F: Field> ToConstraintField<F> for [F] {
    #[inline]
    fn to_field_elements(&self) -> Option<Vec<F>> {
        Some(self.to_vec())
    }
}

impl<ConstraintF: Field> ToConstraintField<ConstraintF> for () {
    #[inline]
    fn to_field_elements(&self) -> Option<Vec<ConstraintF>> {
        Some(Vec::new())
    }
}

impl<ConstraintF: PrimeField> ToConstraintField<ConstraintF> for [u8] {
    #[inline]
    fn to_field_elements(&self) -> Option<Vec<ConstraintF>> {
        use core::convert::TryFrom;
        let max_size = usize::try_from(<ConstraintF as PrimeField>::Params::CAPACITY / 8).unwrap();
        let bigint_size = <ConstraintF as PrimeField>::BigInt::NUM_LIMBS * 8;
        let fes = self
            .chunks(max_size)
            .map(|chunk| {
                let mut bigint = vec![0u8; bigint_size];
                bigint.iter_mut().zip(chunk).for_each(|(a, b)| *a = *b);
                ConstraintF::read(bigint.as_slice()).ok()
            })
            .collect::<Option<Vec<_>>>()?;
        Some(fes)
    }
}

impl<ConstraintF: PrimeField> ToConstraintField<ConstraintF> for [u8; 32] {
    #[inline]
    fn to_field_elements(&self) -> Option<Vec<ConstraintF>> {
        self.as_ref().to_field_elements()
    }
}

impl<ConstraintF: PrimeField> ToConstraintField<ConstraintF> for Vec<u8> {
    #[inline]
    fn to_field_elements(&self) -> Option<Vec<ConstraintF>> {
        self.as_slice().to_field_elements()
    }
}
