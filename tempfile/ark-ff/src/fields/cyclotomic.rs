/// Fields that have a cyclotomic multiplicative subgroup, and which can
/// leverage efficient inversion and squaring algorithms for elements in this subgroup.
/// If a field has multiplicative order p^d - 1, the cyclotomic subgroups refer to subgroups of order φ_n(p),
/// for any n < d, where φ_n is the [n-th cyclotomic polynomial](https://en.wikipedia.org/wiki/Cyclotomic_polynomial).
///
/// ## Note
///
/// Note that this trait is unrelated to the `Group` trait from the `ark_ec` crate. That trait
/// denotes an *additive* group, while this trait denotes a *multiplicative* group.
pub trait CyclotomicMultSubgroup: crate::Field {
    /// Is the inverse fast to compute? For example, in quadratic extensions, the inverse
    /// can be computed at the cost of negating one coordinate, which is much faster than
    /// standard inversion.
    /// By default this is `false`, but should be set to `true` for quadratic extensions.
    const INVERSE_IS_FAST: bool = false;

    /// Compute a square in the cyclotomic subgroup. By default this is computed using [`Field::square`](crate::Field::square), but for
    /// degree 12 extensions, this can be computed faster than normal squaring.
    ///
    /// # Warning
    ///
    /// This method should be invoked only when `self` is in the cyclotomic subgroup.
    fn cyclotomic_square(&self) -> Self {
        let mut result = *self;
        *result.cyclotomic_square_in_place()
    }

    /// Square `self` in place. By default this is computed using
    /// [`Field::square_in_place`](crate::Field::square_in_place), but for degree 12 extensions,
    /// this can be computed faster than normal squaring.
    ///
    /// # Warning
    ///
    /// This method should be invoked only when `self` is in the cyclotomic subgroup.
    fn cyclotomic_square_in_place(&mut self) -> &mut Self {
        self.square_in_place()
    }

    /// Compute the inverse of `self`. See [`Self::INVERSE_IS_FAST`] for details.
    /// Returns [`None`] if `self.is_zero()`, and [`Some`] otherwise.
    ///
    /// # Warning
    ///
    /// This method should be invoked only when `self` is in the cyclotomic subgroup.
    fn cyclotomic_inverse(&self) -> Option<Self> {
        let mut result = *self;
        result.cyclotomic_inverse_in_place().copied()
    }

    /// Compute the inverse of `self`. See [`Self::INVERSE_IS_FAST`] for details.
    /// Returns [`None`] if `self.is_zero()`, and [`Some`] otherwise.
    ///
    /// # Warning
    ///
    /// This method should be invoked only when `self` is in the cyclotomic subgroup.
    fn cyclotomic_inverse_in_place(&mut self) -> Option<&mut Self> {
        self.inverse_in_place()
    }

    /// Compute a cyclotomic exponentiation of `self` with respect to `e`.
    ///
    /// # Warning
    ///
    /// This method should be invoked only when `self` is in the cyclotomic subgroup.
    fn cyclotomic_exp(&self, e: impl AsRef<[u64]>) -> Self {
        let mut result = *self;
        result.cyclotomic_exp_in_place(e);
        result
    }

    /// Set `self` to be the result of exponentiating `self` by `e`,
    /// using efficient cyclotomic algorithms.
    ///
    /// # Warning
    ///
    /// This method should be invoked only when `self` is in the cyclotomic subgroup.
    fn cyclotomic_exp_in_place(&mut self, e: impl AsRef<[u64]>) {
        if self.is_zero() {
            return;
        }

        if Self::INVERSE_IS_FAST {
            // We only use NAF-based exponentiation if inverses are fast to compute.
            let naf = crate::biginteger::arithmetic::find_naf(e.as_ref());
            exp_loop(self, naf.into_iter().rev())
        } else {
            exp_loop(
                self,
                crate::bits::BitIteratorBE::without_leading_zeros(e.as_ref()).map(|e| e as i8),
            )
        };
    }
}

/// Helper function to calculate the double-and-add loop for exponentiation.
fn exp_loop<F: CyclotomicMultSubgroup, I: Iterator<Item = i8>>(f: &mut F, e: I) {
    // If the inverse is fast and we're using naf, we compute the inverse of the base.
    // Otherwise we do nothing with the variable, so we default it to one.
    let self_inverse = if F::INVERSE_IS_FAST {
        f.cyclotomic_inverse().unwrap() // The inverse must exist because self is not zero.
    } else {
        F::one()
    };
    let mut res = F::one();
    let mut found_nonzero = false;
    for value in e {
        if found_nonzero {
            res.cyclotomic_square_in_place();
        }

        if value != 0 {
            found_nonzero = true;

            if value > 0 {
                res *= &*f;
            } else if F::INVERSE_IS_FAST {
                // only use naf if inversion is fast.
                res *= &self_inverse;
            }
        }
    }
    *f = res;
}
