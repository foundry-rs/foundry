/// This modular multiplication algorithm uses Montgomery
/// reduction for efficient implementation. It also additionally
/// uses the "no-carry optimization" outlined
/// [here](https://hackmd.io/@zkteam/modular_multiplication) if
/// `P::MODULUS` has (a) a non-zero MSB, and (b) at least one
/// zero bit in the rest of the modulus.
macro_rules! impl_field_mul_assign {
    ($limbs:expr) => {
        #[inline]
        #[ark_ff_asm::unroll_for_loops]
        fn mul_assign(&mut self, other: &Self) {
            // Checking the modulus at compile time
            let first_bit_set = P::MODULUS.0[$limbs - 1] >> 63 != 0;
            // $limbs can be 1, hence we can run into a case with an unused mut.
            #[allow(unused_mut)]
            let mut all_bits_set = P::MODULUS.0[$limbs - 1] == !0 - (1 << 63);
            for i in 1..$limbs {
                all_bits_set &= P::MODULUS.0[$limbs - i - 1] == !0u64;
            }
            let _no_carry: bool = !(first_bit_set || all_bits_set);

            // No-carry optimisation applied to CIOS
            if _no_carry {
                #[cfg(use_asm)]
                #[allow(unsafe_code, unused_mut)]
                {
                    // Tentatively avoid using assembly for `$limbs == 1`.
                    if $limbs <= 6 && $limbs > 1 {
                        ark_ff_asm::x86_64_asm_mul!($limbs, (self.0).0, (other.0).0);
                        self.reduce();
                        return;
                    }
                }
                let mut r = [0u64; $limbs];
                let mut carry1 = 0u64;
                let mut carry2 = 0u64;

                for i in 0..$limbs {
                    r[0] = fa::mac(r[0], (self.0).0[0], (other.0).0[i], &mut carry1);
                    let k = r[0].wrapping_mul(P::INV);
                    fa::mac_discard(r[0], k, P::MODULUS.0[0], &mut carry2);
                    for j in 1..$limbs {
                        r[j] = mac_with_carry!(r[j], (self.0).0[j], (other.0).0[i], &mut carry1);
                        r[j - 1] = mac_with_carry!(r[j], k, P::MODULUS.0[j], &mut carry2);
                    }
                    r[$limbs - 1] = carry1 + carry2;
                }
                (self.0).0 = r;
                self.reduce();
            // Alternative implementation
            } else {
                *self = self.mul_without_reduce(other, P::MODULUS, P::INV);
                self.reduce();
            }
        }
    };
}

macro_rules! impl_field_into_repr {
    ($limbs:expr, $BigIntegerType:ty) => {
        #[inline]
        #[ark_ff_asm::unroll_for_loops]
        #[allow(clippy::modulo_one)]
        fn into_repr(&self) -> $BigIntegerType {
            let mut tmp = self.0;
            let mut r = tmp.0;
            // Montgomery Reduction
            for i in 0..$limbs {
                let k = r[i].wrapping_mul(P::INV);
                let mut carry = 0;

                mac_with_carry!(r[i], k, P::MODULUS.0[0], &mut carry);
                for j in 1..$limbs {
                    r[(j + i) % $limbs] =
                        mac_with_carry!(r[(j + i) % $limbs], k, P::MODULUS.0[j], &mut carry);
                }
                r[i % $limbs] = carry;
            }
            tmp.0 = r;
            tmp
        }
    };
}

macro_rules! impl_field_square_in_place {
    ($limbs: expr) => {
        #[inline]
        #[ark_ff_asm::unroll_for_loops]
        #[allow(unused_braces, clippy::absurd_extreme_comparisons)]
        fn square_in_place(&mut self) -> &mut Self {
            if $limbs == 1 {
                // We default to multiplying with `self` using the `Mul` impl
                // for the 1 limb case
                *self = *self * *self;
                return self;
            }
            #[cfg(use_asm)]
            #[allow(unsafe_code, unused_mut)]
            {
                // Checking the modulus at compile time
                let first_bit_set = P::MODULUS.0[$limbs - 1] >> 63 != 0;
                let mut all_bits_set = P::MODULUS.0[$limbs - 1] == !0 - (1 << 63);
                for i in 1..$limbs {
                    all_bits_set &= P::MODULUS.0[$limbs - i - 1] == core::u64::MAX;
                }
                let _no_carry: bool = !(first_bit_set || all_bits_set);

                if $limbs <= 6 && _no_carry {
                    ark_ff_asm::x86_64_asm_square!($limbs, (self.0).0);
                    self.reduce();
                    return self;
                }
            }
            let mut r = [0u64; $limbs * 2];

            let mut carry = 0;
            for i in 0..$limbs {
                if i < $limbs - 1 {
                    for j in 0..$limbs {
                        if j > i {
                            r[i + j] =
                                mac_with_carry!(r[i + j], (self.0).0[i], (self.0).0[j], &mut carry);
                        }
                    }
                    r[$limbs + i] = carry;
                    carry = 0;
                }
            }
            r[$limbs * 2 - 1] = r[$limbs * 2 - 2] >> 63;
            for i in 0..$limbs {
                // This computes `r[2 * ($limbs - 1) - (i + 1)]`, but additionally
                // handles the case where the index underflows.
                // Note that we should never hit this case because it only occurs
                // when `$limbs == 1`, but we handle that separately above.
                let subtractor = (2 * ($limbs - 1usize))
                    .checked_sub(i + 1)
                    .map(|index| r[index])
                    .unwrap_or(0);
                r[2 * ($limbs - 1) - i] = (r[2 * ($limbs - 1) - i] << 1) | (subtractor >> 63);
            }
            for i in 3..$limbs {
                r[$limbs + 1 - i] = (r[$limbs + 1 - i] << 1) | (r[$limbs - i] >> 63);
            }
            r[1] <<= 1;

            for i in 0..$limbs {
                r[2 * i] = mac_with_carry!(r[2 * i], (self.0).0[i], (self.0).0[i], &mut carry);
                // need unused assignment because the last iteration of the loop produces an
                // assignment to `carry` that is unused.
                #[allow(unused_assignments)]
                {
                    r[2 * i + 1] = adc!(r[2 * i + 1], 0, &mut carry);
                }
            }
            // Montgomery reduction
            let mut _carry2 = 0;
            for i in 0..$limbs {
                let k = r[i].wrapping_mul(P::INV);
                let mut carry = 0;
                mac_with_carry!(r[i], k, P::MODULUS.0[0], &mut carry);
                for j in 1..$limbs {
                    r[j + i] = mac_with_carry!(r[j + i], k, P::MODULUS.0[j], &mut carry);
                }
                r[$limbs + i] = adc!(r[$limbs + i], _carry2, &mut carry);
                _carry2 = carry;
            }
            (self.0).0.copy_from_slice(&r[$limbs..]);
            self.reduce();
            self
        }
    };
}

macro_rules! impl_field_bigint_conv {
    ($field: ident, $bigint: ident, $params: ident) => {
        impl<P: $params> Into<$bigint> for $field<P> {
            fn into(self) -> $bigint {
                self.into_repr()
            }
        }

        impl<P: $params> From<$bigint> for $field<P> {
            /// Converts `Self::BigInteger` into `Self`
            ///
            /// # Panics
            /// This method panics if `int` is larger than `P::MODULUS`.
            fn from(int: $bigint) -> Self {
                Self::from_repr(int).unwrap()
            }
        }
    };
}

macro_rules! impl_prime_field_standard_sample {
    ($field: ident, $params: ident) => {
        impl<P: $params> ark_std::rand::distributions::Distribution<$field<P>>
            for ark_std::rand::distributions::Standard
        {
            #[inline]
            fn sample<R: ark_std::rand::Rng + ?Sized>(&self, rng: &mut R) -> $field<P> {
                loop {
                    let mut tmp = $field(
                        rng.sample(ark_std::rand::distributions::Standard),
                        PhantomData,
                    );

                    // Mask away the unused bits at the beginning.
                    assert!(P::REPR_SHAVE_BITS <= 64);
                    let mask = if P::REPR_SHAVE_BITS == 64 {
                        0
                    } else {
                        core::u64::MAX >> P::REPR_SHAVE_BITS
                    };
                    tmp.0.as_mut().last_mut().map(|val| *val &= mask);

                    if tmp.is_valid() {
                        return tmp;
                    }
                }
            }
        }
    };
}

macro_rules! impl_prime_field_from_int {
    ($field: ident, 128, $params: ident, $limbs:expr) => {
        impl<P: $params> From<u128> for $field<P> {
            fn from(other: u128) -> Self {
                let mut default_int = P::BigInt::default();
                if $limbs == 1 {
                    default_int.0[0] = (other % u128::from(P::MODULUS.0[0])) as u64;
                } else {
                    let upper = (other >> 64) as u64;
                    let lower = ((other << 64) >> 64) as u64;
                    // This is equivalent to the following, but satisfying the compiler:
                    // default_int.0[0] = lower;
                    // default_int.0[1] = upper;
                    let limbs = [lower, upper];
                    for (cur, other) in default_int.0.iter_mut().zip(&limbs) {
                        *cur = *other;
                    }
                }
                Self::from_repr(default_int).unwrap()
            }
        }

        impl <P: $params> From<i128> for $field<P> {
            fn from(other: i128) -> Self {
                let abs = Self::from(other.unsigned_abs());
                if other.is_positive() {
                    abs
                } else {
                    -abs
                }
            }
        }
    };
    ($field: ident, bool, $params: ident, $limbs:expr) => {
        impl<P: $params> From<bool> for $field<P> {
            fn from(other: bool) -> Self {
                if $limbs == 1 {
                    Self::from_repr(P::BigInt::from(u64::from(other) % P::MODULUS.0[0])).unwrap()
                } else {
                    Self::from_repr(P::BigInt::from(u64::from(other))).unwrap()
                }
            }
        }
    };
    ($field: ident, $int: expr, $params: ident, $limbs:expr) => {
        paste::paste!{
            impl<P: $params> From<[<u $int>]> for $field<P> {
                fn from(other: [<u $int>]) -> Self {
                    if $limbs == 1 {
                        Self::from_repr(P::BigInt::from(u64::from(other) % P::MODULUS.0[0])).unwrap()
                    } else {
                        Self::from_repr(P::BigInt::from(u64::from(other))).unwrap()
                    }
                }
            }

            impl<P: $params> From<[<i $int>]> for $field<P> {
                fn from(other: [<i $int>]) -> Self {
                    let abs = Self::from(other.unsigned_abs());
                    if other.is_positive() {
                        abs
                    } else {
                        -abs
                    }
                }
            }
        }
    };
}

macro_rules! sqrt_impl {
    ($Self:ident, $P:tt, $self:expr) => {{
        // https://eprint.iacr.org/2012/685.pdf (page 12, algorithm 5)
        // Actually this is just normal Tonelli-Shanks; since `P::Generator`
        // is a quadratic non-residue, `P::ROOT_OF_UNITY = P::GENERATOR ^ t`
        // is also a quadratic non-residue (since `t` is odd).
        if $self.is_zero() {
            return Some($Self::zero());
        }
        // Try computing the square root (x at the end of the algorithm)
        // Check at the end of the algorithm if x was a square root
        // Begin Tonelli-Shanks
        let mut z = $Self::qnr_to_t();
        let mut w = $self.pow($P::T_MINUS_ONE_DIV_TWO);
        let mut x = w * $self;
        let mut b = x * &w;

        let mut v = $P::TWO_ADICITY as usize;

        while !b.is_one() {
            let mut k = 0usize;

            let mut b2k = b;
            while !b2k.is_one() {
                // invariant: b2k = b^(2^k) after entering this loop
                b2k.square_in_place();
                k += 1;
            }

            if k == ($P::TWO_ADICITY as usize) {
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
        if (x.square() == *$self) {
            return Some(x);
        } else {
            // Consistency check that if no square root is found,
            // it is because none exists.
            #[cfg(debug_assertions)]
            {
                use crate::fields::LegendreSymbol::*;
                if ($self.legendre() != QuadraticNonResidue) {
                    panic!("Input has a square root per its legendre symbol, but it was not found")
                }
            }
            None
        }
    }};
}

// Implements AddAssign on Self by deferring to an implementation on &Self
#[macro_export]
macro_rules! impl_additive_ops_from_ref {
    ($type: ident, $params: ident) => {
        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::Add<Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn add(self, other: Self) -> Self {
                let mut result = self;
                result.add_assign(&other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::Add<&'a mut Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn add(self, other: &'a mut Self) -> Self {
                let mut result = self;
                result.add_assign(&*other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::Sub<Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn sub(self, other: Self) -> Self {
                let mut result = self;
                result.sub_assign(&other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::Sub<&'a mut Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn sub(self, other: &'a mut Self) -> Self {
                let mut result = self;
                result.sub_assign(&*other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::iter::Sum<Self> for $type<P> {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::zero(), core::ops::Add::add)
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::iter::Sum<&'a Self> for $type<P> {
            fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.fold(Self::zero(), core::ops::Add::add)
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::AddAssign<Self> for $type<P> {
            fn add_assign(&mut self, other: Self) {
                self.add_assign(&other)
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::SubAssign<Self> for $type<P> {
            fn sub_assign(&mut self, other: Self) {
                self.sub_assign(&other)
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::AddAssign<&'a mut Self> for $type<P> {
            fn add_assign(&mut self, other: &'a mut Self) {
                self.add_assign(&*other)
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::SubAssign<&'a mut Self> for $type<P> {
            fn sub_assign(&mut self, other: &'a mut Self) {
                self.sub_assign(&*other)
            }
        }
    };
}

// Implements AddAssign on Self by deferring to an implementation on &Self
#[macro_export]
macro_rules! impl_multiplicative_ops_from_ref {
    ($type: ident, $params: ident) => {
        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::Mul<Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn mul(self, other: Self) -> Self {
                let mut result = self;
                result.mul_assign(&other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::Div<Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn div(self, other: Self) -> Self {
                let mut result = self;
                result.div_assign(&other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::Mul<&'a mut Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn mul(self, other: &'a mut Self) -> Self {
                let mut result = self;
                result.mul_assign(&*other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::Div<&'a mut Self> for $type<P> {
            type Output = Self;

            #[inline]
            fn div(self, other: &'a mut Self) -> Self {
                let mut result = self;
                result.div_assign(&*other);
                result
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::iter::Product<Self> for $type<P> {
            fn product<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::one(), core::ops::Mul::mul)
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::iter::Product<&'a Self> for $type<P> {
            fn product<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.fold(Self::one(), Mul::mul)
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::MulAssign<Self> for $type<P> {
            fn mul_assign(&mut self, other: Self) {
                self.mul_assign(&other)
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::DivAssign<&'a mut Self> for $type<P> {
            fn div_assign(&mut self, other: &'a mut Self) {
                self.div_assign(&*other)
            }
        }

        #[allow(unused_qualifications)]
        impl<'a, P: $params> core::ops::MulAssign<&'a mut Self> for $type<P> {
            fn mul_assign(&mut self, other: &'a mut Self) {
                self.mul_assign(&*other)
            }
        }

        #[allow(unused_qualifications)]
        impl<P: $params> core::ops::DivAssign<Self> for $type<P> {
            fn div_assign(&mut self, other: Self) {
                self.div_assign(&other)
            }
        }
    };
}
