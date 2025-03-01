use ark_std::vec::Vec;

/// Calculate a + b + carry, returning the sum and modifying the
/// carry value.
macro_rules! adc {
    ($a:expr, $b:expr, &mut $carry:expr$(,)?) => {{
        let tmp = ($a as u128) + ($b as u128) + ($carry as u128);

        $carry = (tmp >> 64) as u64;

        tmp as u64
    }};
}

/// Calculate a + (b * c) + carry, returning the least significant digit
/// and setting carry to the most significant digit.
macro_rules! mac_with_carry {
    ($a:expr, $b:expr, $c:expr, &mut $carry:expr$(,)?) => {{
        let tmp = ($a as u128) + ($b as u128 * $c as u128) + ($carry as u128);

        $carry = (tmp >> 64) as u64;

        tmp as u64
    }};
}

/// Calculate a - b - borrow, returning the result and modifying
/// the borrow value.
macro_rules! sbb {
    ($a:expr, $b:expr, &mut $borrow:expr$(,)?) => {{
        let tmp = (1u128 << 64) + ($a as u128) - ($b as u128) - ($borrow as u128);

        $borrow = if tmp >> 64 == 0 { 1 } else { 0 };

        tmp as u64
    }};
}

#[inline(always)]
pub(crate) fn mac(a: u64, b: u64, c: u64, carry: &mut u64) -> u64 {
    let tmp = (u128::from(a)) + u128::from(b) * u128::from(c);

    *carry = (tmp >> 64) as u64;

    tmp as u64
}

#[inline(always)]
pub(crate) fn mac_discard(a: u64, b: u64, c: u64, carry: &mut u64) {
    let tmp = (u128::from(a)) + u128::from(b) * u128::from(c);

    *carry = (tmp >> 64) as u64;
}

pub fn find_wnaf(num: &[u64]) -> Vec<i64> {
    let is_zero = |num: &[u64]| num.iter().all(|x| *x == 0u64);
    let is_odd = |num: &[u64]| num[0] & 1 == 1;
    let sub_noborrow = |num: &mut [u64], z: u64| {
        let mut other = vec![0u64; num.len()];
        other[0] = z;
        let mut borrow = 0;

        for (a, b) in num.iter_mut().zip(other) {
            *a = sbb!(*a, b, &mut borrow);
        }
    };
    let add_nocarry = |num: &mut [u64], z: u64| {
        let mut other = vec![0u64; num.len()];
        other[0] = z;
        let mut carry = 0;

        for (a, b) in num.iter_mut().zip(other) {
            *a = adc!(*a, b, &mut carry);
        }
    };
    let div2 = |num: &mut [u64]| {
        let mut t = 0;
        for i in num.iter_mut().rev() {
            let t2 = *i << 63;
            *i >>= 1;
            *i |= t;
            t = t2;
        }
    };

    let mut num = num.to_vec();
    let mut res = vec![];

    while !is_zero(&num) {
        let z: i64;
        if is_odd(&num) {
            z = 2 - (num[0] % 4) as i64;
            if z >= 0 {
                sub_noborrow(&mut num, z as u64)
            } else {
                add_nocarry(&mut num, (-z) as u64)
            }
        } else {
            z = 0;
        }
        res.push(z);
        div2(&mut num);
    }

    res
}
