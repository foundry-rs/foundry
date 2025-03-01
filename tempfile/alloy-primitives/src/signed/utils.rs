use crate::signed::Signed;
use ruint::Uint;

/// Panic if overflow on debug mode.
#[inline]
#[track_caller]
pub(super) fn handle_overflow<const BITS: usize, const LIMBS: usize>(
    (result, overflow): (Signed<BITS, LIMBS>, bool),
) -> Signed<BITS, LIMBS> {
    debug_assert!(!overflow, "overflow");
    result
}

/// Compute the two's complement of a U256.
#[inline]
pub(super) fn twos_complement<const BITS: usize, const LIMBS: usize>(
    u: Uint<BITS, LIMBS>,
) -> Uint<BITS, LIMBS> {
    if BITS == 0 {
        return u;
    }
    (!u).overflowing_add(Uint::<BITS, LIMBS>::from(1)).0
}

/// Compile-time equality of signed integers.
#[inline]
pub(super) const fn const_eq<const BITS: usize, const LIMBS: usize>(
    left: &Signed<BITS, LIMBS>,
    right: &Signed<BITS, LIMBS>,
) -> bool {
    if BITS == 0 {
        return true;
    }

    let mut i = 0;
    let llimbs = left.0.as_limbs();
    let rlimbs = right.0.as_limbs();
    while i < LIMBS {
        if llimbs[i] != rlimbs[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Compute the max value at compile time.
pub(super) const fn max<const BITS: usize, const LIMBS: usize>() -> Signed<BITS, LIMBS> {
    if LIMBS == 0 {
        return zero();
    }

    let mut limbs = [u64::MAX; LIMBS];
    limbs[LIMBS - 1] &= Signed::<BITS, LIMBS>::MASK; // unset all high bits
    limbs[LIMBS - 1] &= !Signed::<BITS, LIMBS>::SIGN_BIT; // unset the sign bit
    Signed(Uint::from_limbs(limbs))
}

pub(super) const fn min<const BITS: usize, const LIMBS: usize>() -> Signed<BITS, LIMBS> {
    if LIMBS == 0 {
        return zero();
    }

    let mut limbs = [0; LIMBS];
    limbs[LIMBS - 1] = Signed::<BITS, LIMBS>::SIGN_BIT;
    Signed(Uint::from_limbs(limbs))
}

pub(super) const fn zero<const BITS: usize, const LIMBS: usize>() -> Signed<BITS, LIMBS> {
    let limbs = [0; LIMBS];
    Signed(Uint::from_limbs(limbs))
}

pub(super) const fn one<const BITS: usize, const LIMBS: usize>() -> Signed<BITS, LIMBS> {
    if LIMBS == 0 {
        return zero();
    }

    let mut limbs = [0; LIMBS];
    limbs[0] = 1;
    Signed(Uint::from_limbs(limbs))
}

/// Location of the sign bit within the highest limb.
pub(super) const fn sign_bit(bits: usize) -> u64 {
    if bits == 0 {
        return 0;
    }
    let bits = bits % 64;
    if bits == 0 {
        1 << 63
    } else {
        1 << (bits - 1)
    }
}
