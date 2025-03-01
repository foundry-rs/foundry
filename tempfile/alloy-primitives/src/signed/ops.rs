use super::{
    utils::{handle_overflow, twos_complement},
    Sign, Signed,
};
use core::{cmp, iter, ops};
use ruint::Uint;

// ops impl
impl<const BITS: usize, const LIMBS: usize> Signed<BITS, LIMBS> {
    /// Computes the absolute value of `self`.
    ///
    /// # Overflow behavior
    ///
    /// The absolute value of `Self::MIN` cannot be represented as `Self` and
    /// attempting to calculate it will cause an overflow. This means that code
    /// in debug mode will trigger a panic on this case and optimized code will
    /// return `Self::MIN` without a panic.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn abs(self) -> Self {
        handle_overflow(self.overflowing_abs())
    }

    /// Computes the absolute value of `self`.
    ///
    /// Returns a tuple of the absolute version of self along with a boolean
    /// indicating whether an overflow happened. If self is the minimum
    /// value then the minimum value will be returned again and true will be
    /// returned for an overflow happening.
    #[inline]
    #[must_use]
    pub fn overflowing_abs(self) -> (Self, bool) {
        if BITS == 0 {
            return (self, false);
        }
        if self == Self::MIN {
            (self, true)
        } else {
            (Self(self.unsigned_abs()), false)
        }
    }

    /// Checked absolute value. Computes `self.abs()`, returning `None` if `self
    /// == MIN`.
    #[inline]
    #[must_use]
    pub fn checked_abs(self) -> Option<Self> {
        match self.overflowing_abs() {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Saturating absolute value. Computes `self.abs()`, returning `MAX` if
    /// `self == MIN` instead of overflowing.
    #[inline]
    #[must_use]
    pub fn saturating_abs(self) -> Self {
        match self.overflowing_abs() {
            (value, false) => value,
            _ => Self::MAX,
        }
    }

    /// Wrapping absolute value. Computes `self.abs()`, wrapping around at the
    /// boundary of the type.
    #[inline]
    #[must_use]
    pub fn wrapping_abs(self) -> Self {
        self.overflowing_abs().0
    }

    /// Computes the absolute value of `self` without any wrapping or panicking.
    #[inline]
    #[must_use]
    pub fn unsigned_abs(self) -> Uint<BITS, LIMBS> {
        self.into_sign_and_abs().1
    }

    /// Negates self, overflowing if this is equal to the minimum value.
    ///
    /// Returns a tuple of the negated version of self along with a boolean
    /// indicating whether an overflow happened. If `self` is the minimum
    /// value, then the minimum value will be returned again and `true` will
    /// be returned for an overflow happening.
    #[inline]
    #[must_use]
    pub fn overflowing_neg(self) -> (Self, bool) {
        if BITS == 0 {
            return (self, false);
        }
        if self == Self::MIN {
            (self, true)
        } else {
            (Self(twos_complement(self.0)), false)
        }
    }

    /// Checked negation. Computes `-self`, returning `None` if `self == MIN`.
    #[inline]
    #[must_use]
    pub fn checked_neg(self) -> Option<Self> {
        match self.overflowing_neg() {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Saturating negation. Computes `-self`, returning `MAX` if `self == MIN`
    /// instead of overflowing.
    #[inline]
    #[must_use]
    pub fn saturating_neg(self) -> Self {
        match self.overflowing_neg() {
            (value, false) => value,
            _ => Self::MAX,
        }
    }

    /// Wrapping (modular) negation. Computes `-self`, wrapping around at the
    /// boundary of the type.
    ///
    /// The only case where such wrapping can occur is when one negates `MIN` on
    /// a signed type (where `MIN` is the negative minimal value for the
    /// type); this is a positive value that is too large to represent in
    /// the type. In such a case, this function returns `MIN` itself.
    #[inline]
    #[must_use]
    pub fn wrapping_neg(self) -> Self {
        self.overflowing_neg().0
    }

    /// Calculates `self` + `rhs`
    ///
    /// Returns a tuple of the addition along with a boolean indicating whether
    /// an arithmetic overflow would occur. If an overflow would have
    /// occurred then the wrapped value is returned.
    #[inline]
    #[must_use]
    pub const fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        let (unsigned, _) = self.0.overflowing_add(rhs.0);
        let result = Self(unsigned);

        // NOTE: Overflow is determined by checking the sign of the operands and
        //   the result.
        let overflow = matches!(
            (self.sign(), rhs.sign(), result.sign()),
            (Sign::Positive, Sign::Positive, Sign::Negative)
                | (Sign::Negative, Sign::Negative, Sign::Positive)
        );

        (result, overflow)
    }

    /// Checked integer addition. Computes `self + rhs`, returning `None` if
    /// overflow occurred.
    #[inline]
    #[must_use]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.overflowing_add(rhs) {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Saturating integer addition. Computes `self + rhs`, saturating at the
    /// numeric bounds instead of overflowing.
    #[inline]
    #[must_use]
    pub const fn saturating_add(self, rhs: Self) -> Self {
        let (result, overflow) = self.overflowing_add(rhs);
        if overflow {
            match result.sign() {
                Sign::Positive => Self::MIN,
                Sign::Negative => Self::MAX,
            }
        } else {
            result
        }
    }

    /// Wrapping (modular) addition. Computes `self + rhs`, wrapping around at
    /// the boundary of the type.
    #[inline]
    #[must_use]
    pub const fn wrapping_add(self, rhs: Self) -> Self {
        self.overflowing_add(rhs).0
    }

    /// Calculates `self` - `rhs`
    ///
    /// Returns a tuple of the subtraction along with a boolean indicating
    /// whether an arithmetic overflow would occur. If an overflow would
    /// have occurred then the wrapped value is returned.
    #[inline]
    #[must_use]
    pub const fn overflowing_sub(self, rhs: Self) -> (Self, bool) {
        // NOTE: We can't just compute the `self + (-rhs)` because `-rhs` does
        //   not always exist, specifically this would be a problem in case
        //   `rhs == Self::MIN`

        let (unsigned, _) = self.0.overflowing_sub(rhs.0);
        let result = Self(unsigned);

        // NOTE: Overflow is determined by checking the sign of the operands and
        //   the result.
        let overflow = matches!(
            (self.sign(), rhs.sign(), result.sign()),
            (Sign::Positive, Sign::Negative, Sign::Negative)
                | (Sign::Negative, Sign::Positive, Sign::Positive)
        );

        (result, overflow)
    }

    /// Checked integer subtraction. Computes `self - rhs`, returning `None` if
    /// overflow occurred.
    #[inline]
    #[must_use]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.overflowing_sub(rhs) {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Saturating integer subtraction. Computes `self - rhs`, saturating at the
    /// numeric bounds instead of overflowing.
    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, rhs: Self) -> Self {
        let (result, overflow) = self.overflowing_sub(rhs);
        if overflow {
            match result.sign() {
                Sign::Positive => Self::MIN,
                Sign::Negative => Self::MAX,
            }
        } else {
            result
        }
    }

    /// Wrapping (modular) subtraction. Computes `self - rhs`, wrapping around
    /// at the boundary of the type.
    #[inline]
    #[must_use]
    pub const fn wrapping_sub(self, rhs: Self) -> Self {
        self.overflowing_sub(rhs).0
    }

    /// Calculates `self` * `rhs`
    ///
    /// Returns a tuple of the multiplication along with a boolean indicating
    /// whether an arithmetic overflow would occur. If an overflow would
    /// have occurred then the wrapped value is returned.
    #[inline]
    #[must_use]
    pub fn overflowing_mul(self, rhs: Self) -> (Self, bool) {
        if self.is_zero() || rhs.is_zero() {
            return (Self::ZERO, false);
        }
        let sign = self.sign() * rhs.sign();
        let (unsigned, overflow_mul) = self.unsigned_abs().overflowing_mul(rhs.unsigned_abs());
        let (result, overflow_conv) = Self::overflowing_from_sign_and_abs(sign, unsigned);

        (result, overflow_mul || overflow_conv)
    }

    /// Checked integer multiplication. Computes `self * rhs`, returning None if
    /// overflow occurred.
    #[inline]
    #[must_use]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        match self.overflowing_mul(rhs) {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Saturating integer multiplication. Computes `self * rhs`, saturating at
    /// the numeric bounds instead of overflowing.
    #[inline]
    #[must_use]
    pub fn saturating_mul(self, rhs: Self) -> Self {
        let (result, overflow) = self.overflowing_mul(rhs);
        if overflow {
            match self.sign() * rhs.sign() {
                Sign::Positive => Self::MAX,
                Sign::Negative => Self::MIN,
            }
        } else {
            result
        }
    }

    /// Wrapping (modular) multiplication. Computes `self * rhs`, wrapping
    /// around at the boundary of the type.
    #[inline]
    #[must_use]
    pub fn wrapping_mul(self, rhs: Self) -> Self {
        self.overflowing_mul(rhs).0
    }

    /// Calculates `self` / `rhs`
    ///
    /// Returns a tuple of the divisor along with a boolean indicating whether
    /// an arithmetic overflow would occur. If an overflow would occur then
    /// self is returned.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn overflowing_div(self, rhs: Self) -> (Self, bool) {
        assert!(!rhs.is_zero(), "attempt to divide by zero");
        let sign = self.sign() * rhs.sign();
        // Note, signed division can't overflow!
        let unsigned = self.unsigned_abs() / rhs.unsigned_abs();
        let (result, overflow_conv) = Self::overflowing_from_sign_and_abs(sign, unsigned);

        (result, overflow_conv && !result.is_zero())
    }

    /// Checked integer division. Computes `self / rhs`, returning `None` if
    /// `rhs == 0` or the division results in overflow.
    #[inline]
    #[must_use]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.is_zero() || (self == Self::MIN && rhs == Self::MINUS_ONE) {
            None
        } else {
            Some(self.overflowing_div(rhs).0)
        }
    }

    /// Saturating integer division. Computes `self / rhs`, saturating at the
    /// numeric bounds instead of overflowing.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn saturating_div(self, rhs: Self) -> Self {
        match self.overflowing_div(rhs) {
            (value, false) => value,
            // MIN / -1 is the only possible saturating overflow
            _ => Self::MAX,
        }
    }

    /// Wrapping (modular) division. Computes `self / rhs`, wrapping around at
    /// the boundary of the type.
    ///
    /// The only case where such wrapping can occur is when one divides `MIN /
    /// -1` on a signed type (where `MIN` is the negative minimal value for
    /// the type); this is equivalent to `-MIN`, a positive value that is
    /// too large to represent in the type. In such a case, this function
    /// returns `MIN` itself.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn wrapping_div(self, rhs: Self) -> Self {
        self.overflowing_div(rhs).0
    }

    /// Calculates `self` % `rhs`
    ///
    /// Returns a tuple of the remainder after dividing along with a boolean
    /// indicating whether an arithmetic overflow would occur. If an
    /// overflow would occur then 0 is returned.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn overflowing_rem(self, rhs: Self) -> (Self, bool) {
        if self == Self::MIN && rhs == Self::MINUS_ONE {
            (Self::ZERO, true)
        } else {
            let div_res = self / rhs;
            (self - div_res * rhs, false)
        }
    }

    /// Checked integer remainder. Computes `self % rhs`, returning `None` if
    /// `rhs == 0` or the division results in overflow.
    #[inline]
    #[must_use]
    pub fn checked_rem(self, rhs: Self) -> Option<Self> {
        if rhs.is_zero() || (self == Self::MIN && rhs == Self::MINUS_ONE) {
            None
        } else {
            Some(self.overflowing_rem(rhs).0)
        }
    }

    /// Wrapping (modular) remainder. Computes `self % rhs`, wrapping around at
    /// the boundary of the type.
    ///
    /// Such wrap-around never actually occurs mathematically; implementation
    /// artifacts make `x % y` invalid for `MIN / -1` on a signed type
    /// (where `MIN` is the negative minimal value). In such a case, this
    /// function returns `0`.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn wrapping_rem(self, rhs: Self) -> Self {
        self.overflowing_rem(rhs).0
    }

    /// Calculates the quotient of Euclidean division of `self` by `rhs`.
    ///
    /// This computes the integer `q` such that `self = q * rhs + r`, with
    /// `r = self.rem_euclid(rhs)` and `0 <= r < abs(rhs)`.
    ///
    /// In other words, the result is `self / rhs` rounded to the integer `q`
    /// such that `self >= q * rhs`.
    /// If `self > 0`, this is equal to round towards zero (the default in
    /// Rust); if `self < 0`, this is equal to round towards +/- infinity.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0 or the division results in overflow.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn div_euclid(self, rhs: Self) -> Self {
        let q = self / rhs;
        if (self % rhs).is_negative() {
            if rhs.is_positive() {
                q - Self::ONE
            } else {
                q + Self::ONE
            }
        } else {
            q
        }
    }

    /// Calculates the quotient of Euclidean division `self.div_euclid(rhs)`.
    ///
    /// Returns a tuple of the divisor along with a boolean indicating whether
    /// an arithmetic overflow would occur. If an overflow would occur then
    /// `self` is returned.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn overflowing_div_euclid(self, rhs: Self) -> (Self, bool) {
        if self == Self::MIN && rhs == Self::MINUS_ONE {
            (self, true)
        } else {
            (self.div_euclid(rhs), false)
        }
    }

    /// Checked Euclidean division. Computes `self.div_euclid(rhs)`, returning
    /// `None` if `rhs == 0` or the division results in overflow.
    #[inline]
    #[must_use]
    pub fn checked_div_euclid(self, rhs: Self) -> Option<Self> {
        if rhs.is_zero() || (self == Self::MIN && rhs == Self::MINUS_ONE) {
            None
        } else {
            Some(self.div_euclid(rhs))
        }
    }

    /// Wrapping Euclidean division. Computes `self.div_euclid(rhs)`,
    /// wrapping around at the boundary of the type.
    ///
    /// Wrapping will only occur in `MIN / -1` on a signed type (where `MIN` is
    /// the negative minimal value for the type). This is equivalent to
    /// `-MIN`, a positive value that is too large to represent in the type.
    /// In this case, this method returns `MIN` itself.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn wrapping_div_euclid(self, rhs: Self) -> Self {
        self.overflowing_div_euclid(rhs).0
    }

    /// Calculates the least nonnegative remainder of `self (mod rhs)`.
    ///
    /// This is done as if by the Euclidean division algorithm -- given `r =
    /// self.rem_euclid(rhs)`, `self = rhs * self.div_euclid(rhs) + r`, and
    /// `0 <= r < abs(rhs)`.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0 or the division results in overflow.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn rem_euclid(self, rhs: Self) -> Self {
        let r = self % rhs;
        if r < Self::ZERO {
            if rhs < Self::ZERO {
                r - rhs
            } else {
                r + rhs
            }
        } else {
            r
        }
    }

    /// Overflowing Euclidean remainder. Calculates `self.rem_euclid(rhs)`.
    ///
    /// Returns a tuple of the remainder after dividing along with a boolean
    /// indicating whether an arithmetic overflow would occur. If an
    /// overflow would occur then 0 is returned.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn overflowing_rem_euclid(self, rhs: Self) -> (Self, bool) {
        if self == Self::MIN && rhs == Self::MINUS_ONE {
            (Self::ZERO, true)
        } else {
            (self.rem_euclid(rhs), false)
        }
    }

    /// Wrapping Euclidean remainder. Computes `self.rem_euclid(rhs)`, wrapping
    /// around at the boundary of the type.
    ///
    /// Wrapping will only occur in `MIN % -1` on a signed type (where `MIN` is
    /// the negative minimal value for the type). In this case, this method
    /// returns 0.
    ///
    /// # Panics
    ///
    /// If `rhs` is 0.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn wrapping_rem_euclid(self, rhs: Self) -> Self {
        self.overflowing_rem_euclid(rhs).0
    }

    /// Checked Euclidean remainder. Computes `self.rem_euclid(rhs)`, returning
    /// `None` if `rhs == 0` or the division results in overflow.
    #[inline]
    #[must_use]
    pub fn checked_rem_euclid(self, rhs: Self) -> Option<Self> {
        if rhs.is_zero() || (self == Self::MIN && rhs == Self::MINUS_ONE) {
            None
        } else {
            Some(self.rem_euclid(rhs))
        }
    }

    /// Returns the sign of `self` to the exponent `exp`.
    ///
    /// Note that this method does not actually try to compute the `self` to the
    /// exponent `exp`, but instead uses the property that a negative number to
    /// an odd exponent will be negative. This means that the sign of the result
    /// of exponentiation can be computed even if the actual result is too large
    /// to fit in 256-bit signed integer.
    #[inline]
    pub(crate) const fn pow_sign(self, exp: Uint<BITS, LIMBS>) -> Sign {
        let is_exp_odd = BITS != 0 && exp.as_limbs()[0] % 2 == 1;
        if is_exp_odd && self.is_negative() {
            Sign::Negative
        } else {
            Sign::Positive
        }
    }

    /// Create `10**n` as this type.
    ///
    /// # Panics
    ///
    /// If the result overflows the type.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn exp10(n: usize) -> Self {
        Uint::<BITS, LIMBS>::from(10).pow(Uint::from(n)).try_into().expect("overflow")
    }

    /// Raises self to the power of `exp`, using exponentiation by squaring.
    ///
    /// # Panics
    ///
    /// If the result overflows the type in debug mode.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn pow(self, exp: Uint<BITS, LIMBS>) -> Self {
        handle_overflow(self.overflowing_pow(exp))
    }

    /// Raises self to the power of `exp`, using exponentiation by squaring.
    ///
    /// Returns a tuple of the exponentiation along with a bool indicating
    /// whether an overflow happened.
    #[inline]
    #[must_use]
    pub fn overflowing_pow(self, exp: Uint<BITS, LIMBS>) -> (Self, bool) {
        if BITS == 0 {
            return (Self::ZERO, false);
        }

        let sign = self.pow_sign(exp);

        let (unsigned, overflow_pow) = self.unsigned_abs().overflowing_pow(exp);
        let (result, overflow_conv) = Self::overflowing_from_sign_and_abs(sign, unsigned);

        (result, overflow_pow || overflow_conv)
    }

    /// Checked exponentiation. Computes `self.pow(exp)`, returning `None` if
    /// overflow occurred.
    #[inline]
    #[must_use]
    pub fn checked_pow(self, exp: Uint<BITS, LIMBS>) -> Option<Self> {
        let (result, overflow) = self.overflowing_pow(exp);
        if overflow {
            None
        } else {
            Some(result)
        }
    }

    /// Saturating integer exponentiation. Computes `self.pow(exp)`, saturating
    /// at the numeric bounds instead of overflowing.
    #[inline]
    #[must_use]
    pub fn saturating_pow(self, exp: Uint<BITS, LIMBS>) -> Self {
        let (result, overflow) = self.overflowing_pow(exp);
        if overflow {
            match self.pow_sign(exp) {
                Sign::Positive => Self::MAX,
                Sign::Negative => Self::MIN,
            }
        } else {
            result
        }
    }

    /// Raises self to the power of `exp`, wrapping around at the
    /// boundary of the type.
    #[inline]
    #[must_use]
    pub fn wrapping_pow(self, exp: Uint<BITS, LIMBS>) -> Self {
        self.overflowing_pow(exp).0
    }

    /// Shifts self left by `rhs` bits.
    ///
    /// Returns a tuple of the shifted version of self along with a boolean
    /// indicating whether the shift value was larger than or equal to the
    /// number of bits.
    #[inline]
    #[must_use]
    pub fn overflowing_shl(self, rhs: usize) -> (Self, bool) {
        if rhs >= 256 {
            (Self::ZERO, true)
        } else {
            (Self(self.0 << rhs), false)
        }
    }

    /// Checked shift left. Computes `self << rhs`, returning `None` if `rhs` is
    /// larger than or equal to the number of bits in `self`.
    #[inline]
    #[must_use]
    pub fn checked_shl(self, rhs: usize) -> Option<Self> {
        match self.overflowing_shl(rhs) {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Wrapping shift left. Computes `self << rhs`, returning 0 if larger than
    /// or equal to the number of bits in `self`.
    #[inline]
    #[must_use]
    pub fn wrapping_shl(self, rhs: usize) -> Self {
        self.overflowing_shl(rhs).0
    }

    /// Shifts self right by `rhs` bits.
    ///
    /// Returns a tuple of the shifted version of self along with a boolean
    /// indicating whether the shift value was larger than or equal to the
    /// number of bits.
    #[inline]
    #[must_use]
    pub fn overflowing_shr(self, rhs: usize) -> (Self, bool) {
        if rhs >= 256 {
            (Self::ZERO, true)
        } else {
            (Self(self.0 >> rhs), false)
        }
    }

    /// Checked shift right. Computes `self >> rhs`, returning `None` if `rhs`
    /// is larger than or equal to the number of bits in `self`.
    #[inline]
    #[must_use]
    pub fn checked_shr(self, rhs: usize) -> Option<Self> {
        match self.overflowing_shr(rhs) {
            (value, false) => Some(value),
            _ => None,
        }
    }

    /// Wrapping shift right. Computes `self >> rhs`, returning 0 if larger than
    /// or equal to the number of bits in `self`.
    #[inline]
    #[must_use]
    pub fn wrapping_shr(self, rhs: usize) -> Self {
        self.overflowing_shr(rhs).0
    }

    /// Arithmetic shift right operation. Computes `self >> rhs` maintaining the
    /// original sign. If the number is positive this is the same as logic
    /// shift right.
    #[inline]
    #[must_use]
    pub fn asr(self, rhs: usize) -> Self {
        // Avoid shifting if we are going to know the result regardless of the value.
        if rhs == 0 || BITS == 0 {
            return self;
        }

        if rhs >= BITS - 1 {
            match self.sign() {
                Sign::Positive => return Self::ZERO,
                Sign::Negative => return Self::MINUS_ONE,
            }
        }

        match self.sign() {
            // Perform the shift.
            Sign::Positive => self.wrapping_shr(rhs),
            Sign::Negative => {
                // We need to do: `for 0..shift { self >> 1 | 2^255 }`
                // We can avoid the loop by doing: `self >> shift | ~(2^(255 - shift) - 1)`
                // where '~' represents ones complement
                let two: Uint<BITS, LIMBS> = Uint::from(2);
                let bitwise_or = Self::from_raw(
                    !(two.pow(Uint::<BITS, LIMBS>::from(BITS - rhs))
                        - Uint::<BITS, LIMBS>::from(1)),
                );
                (self.wrapping_shr(rhs)) | bitwise_or
            }
        }
    }

    /// Arithmetic shift left operation. Computes `self << rhs`, checking for
    /// overflow on the final result.
    ///
    /// Returns `None` if the operation overflowed (most significant bit
    /// changes).
    #[inline]
    #[must_use]
    pub fn asl(self, rhs: usize) -> Option<Self> {
        if rhs == 0 || BITS == 0 {
            Some(self)
        } else {
            let result = self.wrapping_shl(rhs);
            if result.sign() != self.sign() {
                // Overflow occurred
                None
            } else {
                Some(result)
            }
        }
    }

    /// Compute the [two's complement](https://en.wikipedia.org/wiki/Two%27s_complement) of this number.
    #[inline]
    #[must_use]
    pub fn twos_complement(self) -> Uint<BITS, LIMBS> {
        let abs = self.into_raw();
        match self.sign() {
            Sign::Positive => abs,
            Sign::Negative => twos_complement(abs),
        }
    }
}

// Implement Shl and Shr only for types <= usize, since U256 uses .as_usize()
// which panics
macro_rules! impl_shift {
    ($($t:ty),+) => {
        // We are OK with wrapping behaviour here because it's how Rust behaves with the primitive
        // integer types.

        // $t <= usize: cast to usize
        $(
            impl<const BITS: usize, const LIMBS: usize> ops::Shl<$t> for Signed<BITS, LIMBS> {
                type Output = Self;

                #[inline]
                fn shl(self, rhs: $t) -> Self::Output {
                    self.wrapping_shl(rhs as usize)
                }
            }

            impl<const BITS: usize, const LIMBS: usize> ops::ShlAssign<$t> for Signed<BITS, LIMBS> {
                #[inline]
                fn shl_assign(&mut self, rhs: $t) {
                    *self = *self << rhs;
                }
            }

            impl<const BITS: usize, const LIMBS: usize> ops::Shr<$t> for Signed<BITS, LIMBS> {
                type Output = Self;

                #[inline]
                fn shr(self, rhs: $t) -> Self::Output {
                    self.wrapping_shr(rhs as usize)
                }
            }

            impl<const BITS: usize, const LIMBS: usize> ops::ShrAssign<$t> for Signed<BITS, LIMBS> {
                #[inline]
                fn shr_assign(&mut self, rhs: $t) {
                    *self = *self >> rhs;
                }
            }
        )+
    };
}

#[cfg(target_pointer_width = "16")]
impl_shift!(i8, u8, i16, u16, isize, usize);

#[cfg(target_pointer_width = "32")]
impl_shift!(i8, u8, i16, u16, i32, u32, isize, usize);

#[cfg(target_pointer_width = "64")]
impl_shift!(i8, u8, i16, u16, i32, u32, i64, u64, isize, usize);

// cmp
impl<const BITS: usize, const LIMBS: usize> cmp::PartialOrd for Signed<BITS, LIMBS> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const BITS: usize, const LIMBS: usize> cmp::Ord for Signed<BITS, LIMBS> {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // TODO(nlordell): Once subtraction is implemented:
        // self.saturating_sub(*other).signum64().partial_cmp(&0)

        use cmp::Ordering::*;
        use Sign::*;

        match (self.into_sign_and_abs(), other.into_sign_and_abs()) {
            ((Positive, _), (Negative, _)) => Greater,
            ((Negative, _), (Positive, _)) => Less,
            ((Positive, this), (Positive, other)) => this.cmp(&other),
            ((Negative, this), (Negative, other)) => other.cmp(&this),
        }
    }
}

// arithmetic ops - implemented above
impl<T, const BITS: usize, const LIMBS: usize> ops::Add<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    type Output = Self;

    #[inline]
    #[track_caller]
    fn add(self, rhs: T) -> Self::Output {
        handle_overflow(self.overflowing_add(rhs.into()))
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::AddAssign<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn add_assign(&mut self, rhs: T) {
        *self = *self + rhs;
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::Sub<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    type Output = Self;

    #[inline]
    #[track_caller]
    fn sub(self, rhs: T) -> Self::Output {
        handle_overflow(self.overflowing_sub(rhs.into()))
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::SubAssign<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn sub_assign(&mut self, rhs: T) {
        *self = *self - rhs;
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::Mul<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    type Output = Self;

    #[inline]
    #[track_caller]
    fn mul(self, rhs: T) -> Self::Output {
        handle_overflow(self.overflowing_mul(rhs.into()))
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::MulAssign<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn mul_assign(&mut self, rhs: T) {
        *self = *self * rhs;
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::Div<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    type Output = Self;

    #[inline]
    #[track_caller]
    fn div(self, rhs: T) -> Self::Output {
        handle_overflow(self.overflowing_div(rhs.into()))
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::DivAssign<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn div_assign(&mut self, rhs: T) {
        *self = *self / rhs;
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::Rem<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    type Output = Self;

    #[inline]
    #[track_caller]
    fn rem(self, rhs: T) -> Self::Output {
        handle_overflow(self.overflowing_rem(rhs.into()))
    }
}

impl<T, const BITS: usize, const LIMBS: usize> ops::RemAssign<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn rem_assign(&mut self, rhs: T) {
        *self = *self % rhs;
    }
}

impl<T, const BITS: usize, const LIMBS: usize> iter::Sum<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn sum<I: Iterator<Item = T>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |acc, x| acc + x)
    }
}

impl<T, const BITS: usize, const LIMBS: usize> iter::Product<T> for Signed<BITS, LIMBS>
where
    T: Into<Self>,
{
    #[inline]
    #[track_caller]
    fn product<I: Iterator<Item = T>>(iter: I) -> Self {
        iter.fold(Self::ONE, |acc, x| acc * x)
    }
}

// bitwise ops - delegated to U256
impl<const BITS: usize, const LIMBS: usize> ops::BitAnd for Signed<BITS, LIMBS> {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl<const BITS: usize, const LIMBS: usize> ops::BitAndAssign for Signed<BITS, LIMBS> {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs;
    }
}

impl<const BITS: usize, const LIMBS: usize> ops::BitOr for Signed<BITS, LIMBS> {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl<const BITS: usize, const LIMBS: usize> ops::BitOrAssign for Signed<BITS, LIMBS> {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

impl<const BITS: usize, const LIMBS: usize> ops::BitXor for Signed<BITS, LIMBS> {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self::Output {
        Self(self.0 ^ rhs.0)
    }
}

impl<const BITS: usize, const LIMBS: usize> ops::BitXorAssign for Signed<BITS, LIMBS> {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Self) {
        *self = *self ^ rhs;
    }
}

// unary ops
impl<const BITS: usize, const LIMBS: usize> ops::Neg for Signed<BITS, LIMBS> {
    type Output = Self;

    #[inline]
    #[track_caller]
    fn neg(self) -> Self::Output {
        handle_overflow(self.overflowing_neg())
    }
}

impl<const BITS: usize, const LIMBS: usize> ops::Not for Signed<BITS, LIMBS> {
    type Output = Self;

    #[inline]
    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}
