/// Indication of the field element's quadratic residuosity
///
/// # Examples
/// ```
/// # use ark_std::test_rng;
/// # use ark_std::UniformRand;
/// # use ark_test_curves::{LegendreSymbol, Field, bls12_381::Fq as Fp};
/// let a: Fp = Fp::rand(&mut test_rng());
/// let b = a.square();
/// assert_eq!(b.legendre(), LegendreSymbol::QuadraticResidue);
/// ```
#[derive(Debug, PartialEq, Eq)]
pub enum LegendreSymbol {
    Zero = 0,
    QuadraticResidue = 1,
    QuadraticNonResidue = -1,
}

impl LegendreSymbol {
    /// Returns true if `self.is_zero()`.
    ///
    /// # Examples
    /// ```
    /// # use ark_std::test_rng;
    /// # use ark_std::UniformRand;
    /// # use ark_test_curves::{LegendreSymbol, Field, bls12_381::Fq as Fp};
    /// let a: Fp = Fp::rand(&mut test_rng());
    /// let b: Fp = a.square();
    /// assert!(!b.legendre().is_zero());
    /// ```
    pub fn is_zero(&self) -> bool {
        *self == LegendreSymbol::Zero
    }

    /// Returns true if `self` is a quadratic non-residue.
    ///
    /// # Examples
    /// ```
    /// # use ark_test_curves::{Fp2Config, Field, LegendreSymbol, bls12_381::{Fq, Fq2Config}};
    /// let a: Fq = Fq2Config::NONRESIDUE;
    /// assert!(a.legendre().is_qnr());
    /// ```
    pub fn is_qnr(&self) -> bool {
        *self == LegendreSymbol::QuadraticNonResidue
    }

    /// Returns true if `self` is a quadratic residue.
    /// # Examples
    /// ```
    /// # use ark_std::test_rng;
    /// # use ark_test_curves::bls12_381::Fq as Fp;
    /// # use ark_std::UniformRand;
    /// # use ark_ff::{LegendreSymbol, Field};
    /// let a: Fp = Fp::rand(&mut test_rng());
    /// let b: Fp = a.square();
    /// assert!(b.legendre().is_qr());
    /// ```
    pub fn is_qr(&self) -> bool {
        *self == LegendreSymbol::QuadraticResidue
    }
}

/// Precomputation that makes computing square roots faster
/// A particular variant should only be instantiated if the modulus satisfies
/// the corresponding condition.
#[non_exhaustive]
pub enum SqrtPrecomputation<F: crate::Field> {
    // Tonelli-Shanks algorithm works for all elements, no matter what the modulus is.
    TonelliShanks {
        two_adicity: u32,
        quadratic_nonresidue_to_trace: F,
        trace_of_modulus_minus_one_div_two: &'static [u64],
    },
    /// To be used when the modulus is 3 mod 4.
    Case3Mod4 {
        modulus_plus_one_div_four: &'static [u64],
    },
}

impl<F: crate::Field> SqrtPrecomputation<F> {
    pub fn sqrt(&self, elem: &F) -> Option<F> {
        match self {
            Self::TonelliShanks {
                two_adicity,
                quadratic_nonresidue_to_trace,
                trace_of_modulus_minus_one_div_two,
            } => {
                // https://eprint.iacr.org/2012/685.pdf (page 12, algorithm 5)
                // Actually this is just normal Tonelli-Shanks; since `P::Generator`
                // is a quadratic non-residue, `P::ROOT_OF_UNITY = P::GENERATOR ^ t`
                // is also a quadratic non-residue (since `t` is odd).
                if elem.is_zero() {
                    return Some(F::zero());
                }
                // Try computing the square root (x at the end of the algorithm)
                // Check at the end of the algorithm if x was a square root
                // Begin Tonelli-Shanks
                let mut z = *quadratic_nonresidue_to_trace;
                let mut w = elem.pow(trace_of_modulus_minus_one_div_two);
                let mut x = w * elem;
                let mut b = x * &w;

                let mut v = *two_adicity as usize;

                while !b.is_one() {
                    let mut k = 0usize;

                    let mut b2k = b;
                    while !b2k.is_one() {
                        // invariant: b2k = b^(2^k) after entering this loop
                        b2k.square_in_place();
                        k += 1;
                    }

                    if k == (*two_adicity as usize) {
                        // We are in the case where self^(T * 2^k) = x^(P::MODULUS - 1) = 1,
                        // which means that no square root exists.
                        return None;
                    }
                    let j = v - k;
                    w = z;
                    for _ in 1..j {
                        w.square_in_place();
                    }

                    z = w.square();
                    b *= &z;
                    x *= &w;
                    v = k;
                }
                // Is x the square root? If so, return it.
                if x.square() == *elem {
                    Some(x)
                } else {
                    // Consistency check that if no square root is found,
                    // it is because none exists.
                    debug_assert!(!matches!(elem.legendre(), LegendreSymbol::QuadraticResidue));
                    None
                }
            },
            Self::Case3Mod4 {
                modulus_plus_one_div_four,
            } => {
                let result = elem.pow(modulus_plus_one_div_four.as_ref());
                (result.square() == *elem).then_some(result)
            },
        }
    }
}
