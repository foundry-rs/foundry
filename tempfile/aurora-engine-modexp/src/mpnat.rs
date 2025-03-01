use crate::{
    arith::{
        big_wrapping_mul, big_wrapping_pow, borrowing_sub, carrying_add, compute_r_mod_n,
        in_place_add, in_place_mul_sub, in_place_shl, in_place_shr, join_as_double, mod_inv,
        monpro, monsq,
    },
    maybe_std::{vec, Vec},
};

pub type Word = u64;
pub type DoubleWord = u128;
pub const WORD_BYTES: usize = size_of::<Word>();
pub const WORD_BITS: usize = Word::BITS as usize;
pub const BASE: DoubleWord = (Word::MAX as DoubleWord) + 1;

/// Multi-precision natural number, represented in base `Word::MAX + 1 = 2^WORD_BITS`.
/// The digits are stored in little-endian order, i.e. digits[0] is the least
/// significant digit.
#[derive(Debug)]
pub struct MPNat {
    pub digits: Vec<Word>,
}

impl MPNat {
    fn strip_leading_zeroes(a: &[u8]) -> (&[u8], bool) {
        let len = a.len();
        let end = a.iter().position(|&x| x != 0).unwrap_or(len);

        if end == len {
            (&[], true)
        } else {
            (&a[end..], false)
        }
    }

    // Koç's algorithm for inversion mod 2^k
    // https://eprint.iacr.org/2017/411.pdf
    fn koc_2017_inverse(aa: &Self, k: usize) -> Self {
        debug_assert!(aa.is_odd());

        let length = k / WORD_BITS;
        let mut b = Self {
            digits: vec![0; length + 1],
        };
        b.digits[0] = 1;

        let mut a = Self {
            digits: aa.digits.clone(),
        };
        a.digits.resize(length + 1, 0);

        let mut neg: bool = false;

        let mut res = Self {
            digits: vec![0; length + 1],
        };

        let (mut wordpos, mut bitpos) = (0, 0);

        for _ in 0..k {
            let x = b.digits[0] & 1;
            if x != 0 {
                if neg {
                    // b = b - a
                    in_place_add(&mut b.digits, &a.digits);
                } else {
                    // b = a - b
                    let mut tmp = Self {
                        digits: a.digits.clone(),
                    };
                    in_place_mul_sub(&mut tmp.digits, &b.digits, 1);
                    b = tmp;
                    neg = true;
                }
            }

            in_place_shr(&mut b.digits, 1);

            res.digits[wordpos] |= x << bitpos;

            bitpos += 1;
            if bitpos == WORD_BITS {
                bitpos = 0;
                wordpos += 1;
            }
        }

        res
    }

    pub fn from_big_endian(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self { digits: vec![0] };
        }
        // Remainder on division by WORD_BYTES
        let r = bytes.len() & (WORD_BYTES - 1);
        let n_digits = if r == 0 {
            bytes.len() / WORD_BYTES
        } else {
            // Need an extra digit for the remainder
            (bytes.len() / WORD_BYTES) + 1
        };
        let mut digits = vec![0; n_digits];
        // buffer to hold Word-sized slices of the input bytes
        let mut buf = [0u8; WORD_BYTES];
        let mut i = n_digits - 1;
        if r != 0 {
            buf[(WORD_BYTES - r)..].copy_from_slice(&bytes[0..r]);
            digits[i] = Word::from_be_bytes(buf);
            if i == 0 {
                // Special case where there is just one digit
                return Self { digits };
            }
            i -= 1;
        }
        let mut j = r;
        loop {
            let next_j = j + WORD_BYTES;
            buf.copy_from_slice(&bytes[j..next_j]);
            digits[i] = Word::from_be_bytes(buf);
            if i == 0 {
                break;
            }

            i -= 1;
            j = next_j;
        }
        // throw away leading zeros
        while digits.len() > 1 && digits[digits.len() - 1] == 0 {
            digits.pop();
        }
        Self { digits }
    }

    pub fn is_power_of_two(&self) -> bool {
        // A multi-precision number is a power of 2 iff exactly one digit
        // is a power of 2 and all others are zero.
        let mut found_power_of_two = false;
        for &d in &self.digits {
            let is_p2 = d.is_power_of_two();
            if (!is_p2 && d != 0) || (is_p2 && found_power_of_two) {
                return false;
            } else if is_p2 {
                found_power_of_two = true;
            }
        }
        found_power_of_two
    }

    pub fn is_odd(&self) -> bool {
        // A binary number is odd iff its lowest order bit is set.
        self.digits[0] & 1 == 1
    }

    /// Computes `self ^ exp mod modulus`. `exp` must be given as big-endian bytes.
    #[allow(clippy::too_many_lines, clippy::debug_assert_with_mut_call)]
    pub fn modpow(&mut self, exp: &[u8], modulus: &Self) -> Self {
        // exp must be stripped because it is iterated over in
        // `big_wrapping_pow` and `modpow_montgomery`, and a large
        // zero-padded exp leads to performance issues.
        let (exp, exp_is_zero) = Self::strip_leading_zeroes(exp);

        // base^0 is always 1, regardless of base.
        // Hence, the result is 0 for (base^0) % 1, and 1
        // for every modulus larger than 1.
        //
        // The case of modulus being 0 should have already been
        // handled in modexp().
        debug_assert!(!(modulus.digits.len() == 1 && modulus.digits[0] == 0));
        if exp_is_zero {
            if modulus.digits.len() == 1 && modulus.digits[0] == 1 {
                return Self { digits: vec![0] };
            }

            return Self { digits: vec![1] };
        }

        if exp.len() <= size_of::<usize>() {
            let exp_as_number = {
                let mut tmp: usize = 0;
                for d in exp {
                    tmp *= 256;
                    tmp += (*d) as usize;
                }
                tmp
            };

            if let Some(max_output_digits) = self.digits.len().checked_mul(exp_as_number) {
                if modulus.digits.len() > max_output_digits {
                    // Special case: modulus is larger than `base ^ exp`, so division is not relevant
                    let mut scratch_space = vec![0; max_output_digits];
                    return big_wrapping_pow(self, exp, &mut scratch_space);
                }
            }
        }

        if modulus.is_power_of_two() {
            return self.modpow_with_power_of_two(exp, modulus);
        } else if modulus.is_odd() {
            return self.modpow_montgomery(exp, modulus);
        }

        // If the modulus is not a power of two and not an odd number then
        // it is a product of some power of two with an odd number. In this
        // case we will use the Chinese remainder theorem to get the result.
        // See http://www.people.vcu.edu/~jwang3/CMSC691/j34monex.pdf

        let trailing_zeros = modulus.digits.iter().take_while(|x| x == &&0).count();
        let additional_zero_bits = modulus.digits[trailing_zeros].trailing_zeros() as usize;
        let power_of_two = {
            let mut tmp = Self {
                digits: vec![0; trailing_zeros + 1],
            };
            tmp.digits[trailing_zeros] = 1 << additional_zero_bits;
            tmp
        };
        let power_of_two_mask = *power_of_two.digits.last().unwrap() - 1;
        let odd = {
            let num_digits = modulus.digits.len() - trailing_zeros;
            let mut tmp = Self {
                digits: vec![0; num_digits],
            };
            if additional_zero_bits > 0 {
                tmp.digits[0] = modulus.digits[trailing_zeros] >> additional_zero_bits;
                for i in 1..num_digits {
                    let d = modulus.digits[trailing_zeros + i];
                    tmp.digits[i - 1] +=
                        (d & power_of_two_mask) << (WORD_BITS - additional_zero_bits);
                    tmp.digits[i] = d >> additional_zero_bits;
                }
            } else {
                tmp.digits
                    .copy_from_slice(&modulus.digits[trailing_zeros..]);
            }
            while tmp.digits.last() == Some(&0) {
                tmp.digits.pop();
            }
            tmp
        };
        debug_assert!(power_of_two.is_power_of_two(), "Factored out power of two");
        debug_assert!(
            odd.is_odd(),
            "Remaining number is odd after factoring out powers of two"
        );
        debug_assert!(
            {
                let mut tmp = vec![0; modulus.digits.len()];
                big_wrapping_mul(&power_of_two, &odd, &mut tmp);
                tmp == modulus.digits
            },
            "modulus is factored"
        );

        let mut base_copy = Self {
            digits: self.digits.clone(),
        };
        let x1 = base_copy.modpow_montgomery(exp, &odd);
        let x2 = self.modpow_with_power_of_two(exp, &power_of_two);

        let odd_inv =
            Self::koc_2017_inverse(&odd, trailing_zeros * WORD_BITS + additional_zero_bits);

        let s = power_of_two.digits.len();
        let mut scratch = vec![0; s];
        let diff = {
            scratch.fill(0);
            let mut b = false;
            for (i, scratch_digit) in scratch.iter_mut().enumerate().take(s) {
                let (diff, borrow) = borrowing_sub(
                    x2.digits.get(i).copied().unwrap_or(0),
                    x1.digits.get(i).copied().unwrap_or(0),
                    b,
                );
                *scratch_digit = diff;
                b = borrow;
            }
            Self { digits: scratch }
        };
        let y = {
            let mut out = vec![0; s];
            big_wrapping_mul(&diff, &odd_inv, &mut out);
            *out.last_mut().unwrap() &= power_of_two_mask;
            Self { digits: out }
        };

        // Re-use allocation for efficiency
        let mut digits = diff.digits;
        let s = modulus.digits.len();
        digits.fill(0);
        digits.resize(s, 0);
        big_wrapping_mul(&odd, &y, &mut digits);
        let mut c = false;
        for (i, out_digit) in digits.iter_mut().enumerate() {
            let (sum, carry) = carrying_add(x1.digits.get(i).copied().unwrap_or(0), *out_digit, c);
            c = carry;
            *out_digit = sum;
        }
        Self { digits }
    }

    // Computes `self ^ exp mod modulus` using Montgomery multiplication.
    // See https://www.microsoft.com/en-us/research/wp-content/uploads/1996/01/j37acmon.pdf
    fn modpow_montgomery(&mut self, exp: &[u8], modulus: &Self) -> Self {
        // The montgomery method only works with odd modulus.
        debug_assert!(modulus.is_odd());

        // n_prime satisfies `r * (r^(-1)) - modulus * n' = 1`, where
        // `r = 2^(WORD_BITS*modulus.digits.len())`.
        let n_prime = Word::MAX - mod_inv(modulus.digits[0]) + 1;
        let s = modulus.digits.len();

        let mut x_bar = Self { digits: vec![0; s] };
        // Initialize result as `r mod modulus` (Montgomery form of 1)
        compute_r_mod_n(modulus, &mut x_bar.digits);

        // Reduce base mod modulus
        self.sub_to_same_size(modulus);

        // Need to compute a_bar = base * r mod modulus;
        // First directly multiply base * r to get a 2s-digit number,
        // then reduce mod modulus.
        let a_bar = {
            let mut tmp = Self {
                digits: vec![0; 2 * s],
            };
            big_wrapping_mul(self, &x_bar, &mut tmp.digits);
            tmp.sub_to_same_size(modulus);
            tmp
        };

        // scratch space for monpro algorithm
        let mut scratch = vec![0; 2 * s + 1];
        let monpro_len = s + 2;

        // Use binary method for computing exp, but with monpro as the multiplication
        for &b in exp {
            let mut mask: u8 = 1 << 7;
            while mask > 0 {
                monsq(&x_bar, modulus, n_prime, &mut scratch);
                x_bar.digits.copy_from_slice(&scratch[0..s]);
                scratch.fill(0);
                if b & mask != 0 {
                    monpro(
                        &x_bar,
                        &a_bar,
                        modulus,
                        n_prime,
                        &mut scratch[0..monpro_len],
                    );
                    x_bar.digits.copy_from_slice(&scratch[0..s]);
                    scratch.fill(0);
                }
                mask >>= 1;
            }
        }

        // Convert out of Montgomery form by computing monpro with 1
        let one = {
            // We'll reuse the memory space from a_bar for efficiency.
            let mut digits = a_bar.digits;
            digits.fill(0);
            digits[0] = 1;
            Self { digits }
        };
        monpro(&x_bar, &one, modulus, n_prime, &mut scratch[0..monpro_len]);
        scratch.resize(s, 0);
        Self { digits: scratch }
    }

    fn modpow_with_power_of_two(&mut self, exp: &[u8], modulus: &Self) -> Self {
        debug_assert!(modulus.is_power_of_two());
        // We know `modulus` is a power of 2. So reducing is as easy as bit shifting.
        // We also know the modulus is non-zero because 0 is not a power of 2.

        // First reduce self to be the same size as the modulus
        self.force_same_size(modulus);
        // The modulus is a power of 2 but that power may not be a multiple of a whole word.
        // We can clear out any higher order bits to fix this.
        let modulus_mask = *modulus.digits.last().unwrap() - 1;
        *self.digits.last_mut().unwrap() &= modulus_mask;

        // We know that `totient(2^k) = 2^(k-1)`, therefore by Euler's theorem
        // we can also reduce the exponent mod `2^(k-1)`. Effectively this means
        // throwing away bytes to make `exp` shorter. Note: Euler's theorem only applies
        // if the base and modulus are coprime (which in this case means the base is odd).
        let exp = if self.is_odd() && (exp.len() > WORD_BYTES * modulus.digits.len()) {
            &exp[(exp.len() - WORD_BYTES * modulus.digits.len())..]
        } else {
            exp
        };

        let mut scratch_space = vec![0; modulus.digits.len()];
        let mut result = big_wrapping_pow(self, exp, &mut scratch_space);

        // The modulus is a power of 2 but that power may not be a multiple of a whole word.
        // We can clear out any higher order bits to fix this.
        *result.digits.last_mut().unwrap() &= modulus_mask;

        result
    }

    /// Makes `self` have the same number of digits as `other` by
    /// pushing 0s or dropping higher order digits as needed.
    /// This is equivalent to reducing `self` modulo `2^(WORD_BITS*k)` where
    /// `k` is the number of digits in `other`.
    fn force_same_size(&mut self, other: &Self) {
        self.digits.resize(other.digits.len(), 0);

        // This is here to just drive the point home about what the
        // invariant is after calling this function.
        debug_assert_eq!(self.digits.len(), other.digits.len());
    }

    /// Assumes `self` has more digits than `other`.
    /// Makes `self` have the same number of digits as `other` by subtracting off multiples
    /// of `other`. This is a partial reduction of `self` modulo `other`, but rather
    /// than doing the full division, the goal is simply to make the two numbers have the
    /// same number of digits.
    fn sub_to_same_size(&mut self, other: &Self) {
        // Remove leading zeros before starting
        while self.digits.len() > 1 && self.digits.last() == Some(&0) {
            self.digits.pop();
        }

        let n = other.digits.len();
        let m = self.digits.len().saturating_sub(n);
        if m == 0 {
            return;
        }

        let other_most_sig = *other.digits.last().unwrap() as DoubleWord;

        if self.digits.len() == 2 {
            // This is the smallest case since `n >= 1` and `m > 0`
            // implies that `self.digits.len() >= 2`.
            // In this case we can use DoubleWord-sized arithmetic
            // to get the answer directly.
            let self_most_sig = self.digits.pop().unwrap();
            let a = join_as_double(self_most_sig, self.digits[0]);
            let b = other_most_sig;
            self.digits[0] = (a % b) as Word;
            return;
        }

        if n == 1 {
            // The divisor is only 1 digit, so the long-division
            // algorithm is easy.
            let k = self.digits.len() - 1;
            for j in (0..k).rev() {
                let self_most_sig = self.digits.pop().unwrap();
                let self_second_sig = self.digits[j];
                let r = join_as_double(self_most_sig, self_second_sig) % other_most_sig;
                self.digits[j] = r as Word;
            }
            return;
        }

        // At this stage we know that `n >= 2` and `self.digits.len() >= 3`.
        // The smaller cases are covered in the if-statements above.

        // The algorithm below only works well when the divisor's
        // most significant digit is at least `BASE / 2`.
        // If it is too small then we "normalize" by multiplying
        // both numerator and denominator by a common factor
        // and run the algorithm on those numbers.
        // See Knuth The Art of Computer Programming vol. 2 section 4.3 for details.
        let shift = (other_most_sig as Word).leading_zeros();
        if shift > 0 {
            // Normalize self
            let overflow = in_place_shl(&mut self.digits, shift);
            self.digits.push(overflow);

            // Normalize other
            let mut normalized = other.digits.clone();
            let overflow = in_place_shl(&mut normalized, shift);
            debug_assert_eq!(overflow, 0, "Normalizing modulus cannot overflow");
            debug_assert_eq!(
                normalized[n - 1].leading_zeros(),
                0,
                "Most significant bit is set"
            );

            // Run algorithm on normalized values
            self.sub_to_same_size(&Self { digits: normalized });

            // need to de-normalize to get the correct result
            in_place_shr(&mut self.digits, shift);

            return;
        }

        let other_second_sig = other.digits[n - 2] as DoubleWord;
        let mut self_most_sig: Word = 0;
        for j in (0..=m).rev() {
            let self_second_sig = *self.digits.last().unwrap();
            let self_third_sig = self.digits[self.digits.len() - 2];

            let a = join_as_double(self_most_sig, self_second_sig);
            let mut q_hat = a / other_most_sig;
            let mut r_hat = a % other_most_sig;

            loop {
                let a = q_hat * other_second_sig;
                let b = join_as_double(r_hat as Word, self_third_sig);
                if q_hat >= BASE || a > b {
                    q_hat -= 1;
                    r_hat += other_most_sig;
                    if BASE <= r_hat {
                        break;
                    }
                } else {
                    break;
                }
            }

            let mut borrow = in_place_mul_sub(&mut self.digits[j..], &other.digits, q_hat as Word);
            if borrow > self_most_sig {
                // q_hat was too large, add back one multiple of the modulus
                let carry = in_place_add(&mut self.digits[j..], &other.digits);
                debug_assert!(
                    carry,
                    "Adding back should cause overflow to cancel the borrow"
                );
                borrow -= 1;
            }
            // Most significant digit of self has been cancelled out
            debug_assert_eq!(borrow, self_most_sig);
            self_most_sig = self.digits.pop().unwrap();
        }

        self.digits.push(self_most_sig);
        debug_assert!(self.digits.len() <= n);
    }

    pub fn to_big_endian(&self) -> Vec<u8> {
        if self.digits.iter().all(|x| x == &0) {
            return vec![0];
        }

        // Safety: unwrap is safe since `self.digits` is always non-empty.
        let most_sig_bytes: [u8; WORD_BYTES] = self.digits.last().unwrap().to_be_bytes();
        // The most significant digit may not need 4 bytes.
        // Only include as many bytes as needed in the output.
        let be_initial_bytes = {
            let mut tmp: &[u8] = &most_sig_bytes;
            while !tmp.is_empty() && tmp[0] == 0 {
                tmp = &tmp[1..];
            }
            tmp
        };

        let mut result = vec![0u8; (self.digits.len() - 1) * WORD_BYTES + be_initial_bytes.len()];
        result[0..be_initial_bytes.len()].copy_from_slice(be_initial_bytes);
        for (i, d) in self.digits.iter().take(self.digits.len() - 1).enumerate() {
            let bytes = d.to_be_bytes();
            let j = result.len() - WORD_BYTES * i;
            result[(j - WORD_BYTES)..j].copy_from_slice(&bytes);
        }
        result
    }
}

#[test]
fn test_modpow_even() {
    fn check_modpow_even(base: u128, exp: u128, modulus: u128, expected: u128) {
        let mut x = MPNat::from_big_endian(&base.to_be_bytes());
        let m = MPNat::from_big_endian(&modulus.to_be_bytes());
        let result = x.modpow(&exp.to_be_bytes(), &m);
        let result = crate::arith::mp_nat_to_u128(&result);
        assert_eq!(result, expected);
    }

    check_modpow_even(3, 5, 500, 243);
    check_modpow_even(3, 5, 20, 3);

    check_modpow_even(
        0x2ff4f4df4c518867207c84b57a77aa50,
        0xca83c2925d17c577c9a03598b6f360,
        0xf863d4f17a5405d84814f54c92f803c8,
        0x8d216c9a1fb275ed18eb340ed43cacc0,
    );
    check_modpow_even(
        0x13881e1614244c56d15ac01096b070e7,
        0x336df5b4567cbe4c093271dc151e6c72,
        0x7540f399a0b6c220f1fc60d2451a1ff0,
        0x1251d64c552e8f831f5b841d2811f9c1,
    );
    check_modpow_even(
        0x774d5b2494a449d8f22b22ea542d4ddf,
        0xd2f602e1688f271853e7794503c2837e,
        0xa80d20ebf75f92192159197b60f36e8e,
        0x3fbbba42489b27fc271fb39f54aae2e1,
    );
    check_modpow_even(
        0x756e409cc3583a6b68ae27ccd9eb3d50,
        0x16dafb38a334288954d038bedbddc970,
        0x1f9b2237f09413d1fc44edf9bd02b8bc,
        0x9347445ac61536a402723cd07a3f5a4,
    );
    check_modpow_even(
        0x6dcb8405e2cc4dcebee3e2b14861b47d,
        0xe6c1e5251d6d5deb8dddd0198481d671,
        0xe34a31d814536e8b9ff6cc5300000000,
        0xaa86af638386880334694967564d0c3d,
    );
    check_modpow_even(
        0x9c12fe4a1a97d17c1e4573247a43b0e5,
        0x466f3e0a2e8846b8c48ecbf612b96412,
        0x710d7b9d5718acff0000000000000000,
        0x569bf65929e71cd10a553a8623bdfc99,
    );
    check_modpow_even(
        0x6d018fdeaa408222cb10ff2c36124dcf,
        0x8e35fc05d490bb138f73c2bc284a67a7,
        0x6c237160750d78400000000000000000,
        0x3fe14e11392c6c6be8efe956c965d5af,
    );

    let base: Vec<u8> = vec![
        0x36, 0xAB, 0xD4, 0x52, 0x4E, 0x89, 0xA3, 0x4C, 0x89, 0xC4, 0x20, 0x94, 0x25, 0x47, 0xE1,
        0x2C, 0x7B, 0xE1,
    ];
    let exponent: Vec<u8> = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x05, 0x17, 0xEA, 0x78];
    let modulus: Vec<u8> = vec![
        0x02, 0xF0, 0x75, 0x8C, 0x6A, 0x04, 0x20, 0x09, 0x55, 0xB6, 0x49, 0xC3, 0x57, 0x22, 0xB8,
        0x00, 0x00, 0x00, 0x00,
    ];
    let result = crate::modexp(&base, &exponent, &modulus);
    assert_eq!(
        result,
        vec![2, 63, 79, 118, 41, 54, 235, 9, 115, 212, 107, 110, 173, 181, 157, 104, 208, 97, 1]
    );

    let base = hex::decode("36abd4524e89a34c89c420942547e12c7be1").unwrap();
    let exponent = hex::decode("01000000000517ea78").unwrap();
    let modulus = hex::decode("02f0758c6a04200955b649c35722b800000000").unwrap();
    let result = crate::modexp(&base, &exponent, &modulus);
    assert_eq!(
        hex::encode(result),
        "023f4f762936eb0973d46b6eadb59d68d06101"
    );

    // Test empty exp
    let base = hex::decode("00").unwrap();
    let exponent = hex::decode("").unwrap();
    let modulus = hex::decode("02").unwrap();
    let result = crate::modexp(&base, &exponent, &modulus);
    assert_eq!(hex::encode(result), "01");

    // Test zero exp
    let base = hex::decode("00").unwrap();
    let exponent = hex::decode("00").unwrap();
    let modulus = hex::decode("02").unwrap();
    let result = crate::modexp(&base, &exponent, &modulus);
    assert_eq!(hex::encode(result), "01");
}

#[test]
fn test_modpow_montgomery() {
    fn check_modpow_montgomery(base: u128, exp: u128, modulus: u128, expected: u128) {
        let mut x = MPNat::from_big_endian(&base.to_be_bytes());
        let m = MPNat::from_big_endian(&modulus.to_be_bytes());
        let result = x.modpow_montgomery(&exp.to_be_bytes(), &m);
        let result = crate::arith::mp_nat_to_u128(&result);
        assert_eq!(
            result, expected,
            "({base} ^ {exp}) % {modulus} failed check_modpow_montgomery"
        );
    }

    check_modpow_montgomery(3, 5, 0x9346_9d50_1f74_d1c1, 243);
    check_modpow_montgomery(3, 5, 19, 15);
    check_modpow_montgomery(
        0x5c4b74ec760dfb021499f5c5e3c69222,
        0x62b2a34b21cf4cc036e880b3fb59fe09,
        0x7b799c4502cd69bde8bb12601ce3ff15,
        0x10c9d9071d0b86d6a59264d2f461200,
    );
    check_modpow_montgomery(
        0xadb5ce8589030e3a9112123f4558f69c,
        0xb002827068f05b84a87431a70fb763ab,
        0xc4550871a1cfc67af3e77eceb2ecfce5,
        0x7cb78c0e1c1b43f6412e9d1155ea96d2,
    );
    check_modpow_montgomery(
        0x26eb51a5d9bf15a536b6e3c67867b492,
        0xddf007944a79bf55806003220a58cc6,
        0xc96275a80c694a62330872b2690f8773,
        0x23b75090ead913def3a1e0bde863eda7,
    );
    check_modpow_montgomery(
        0xb93fa81979e597f548c78f2ecb6800f3,
        0x5fad650044963a271898d644984cb9f0,
        0xbeb60d6bd0439ea39d447214a4f8d3ab,
        0x354e63e6a5e007014acd3e5ea88dc3ad,
    );
    check_modpow_montgomery(
        0x1993163e4f578869d04949bc005c878f,
        0x8cb960f846475690259514af46868cf5,
        0x52e104dc72423b534d8e49d878f29e3b,
        0x2aa756846258d5cfa6a3f8b9b181a11c,
    );
}

#[test]
fn test_modpow_with_power_of_two() {
    fn check_modpow_with_power_of_two(base: u128, exp: u128, modulus: u128, expected: u128) {
        let mut x = MPNat::from_big_endian(&base.to_be_bytes());
        let m = MPNat::from_big_endian(&modulus.to_be_bytes());
        let result = x.modpow_with_power_of_two(&exp.to_be_bytes(), &m);
        let result = crate::arith::mp_nat_to_u128(&result);
        assert_eq!(result, expected);
    }

    check_modpow_with_power_of_two(3, 2, 1 << 30, 9);
    check_modpow_with_power_of_two(3, 5, 1 << 30, 243);
    check_modpow_with_power_of_two(3, 1_000_000, 1 << 30, 641836289);
    check_modpow_with_power_of_two(3, 1_000_000, 1 << 31, 1715578113);
    check_modpow_with_power_of_two(3, 1_000_000, 1 << 32, 3863061761);
    check_modpow_with_power_of_two(
        0xabcd_ef01_2345_6789_1111,
        0x1234_5678_90ab_cdef,
        1 << 5,
        17,
    );
    check_modpow_with_power_of_two(
        0x3f47_9dc0_d5b9_6003,
        0xa180_e045_e314_8581,
        1 << 118,
        0x0028_3d19_e6cc_b8a0_e050_6abb_b9b1_1a03,
    );
}

#[test]
fn test_sub_to_same_size() {
    fn check_sub_to_same_size(a: u128, n: u128) {
        let mut x = MPNat::from_big_endian(&a.to_be_bytes());
        let y = MPNat::from_big_endian(&n.to_be_bytes());
        x.sub_to_same_size(&y);
        assert!(x.digits.len() <= y.digits.len());
        let result = crate::arith::mp_nat_to_u128(&x);
        assert_eq!(result % n, a % n, "{a} % {n} failed sub_to_same_size check");
    }

    check_sub_to_same_size(0x10_00_00_00_00, 0xFF_00_00_00);
    check_sub_to_same_size(0x10_00_00_00_00, 0x01_00_00_00);
    check_sub_to_same_size(0x35_00_00_00_00, 0x01_00_00_00);
    check_sub_to_same_size(0xEF_00_00_00_00_00_00, 0x02_FF_FF_FF);

    let n = 10;
    let a = 57 + 2 * n + 0x1234_0000_0000 * n + 0x000b_0000_0000_0000_0000 * n;
    check_sub_to_same_size(a, n);

    /* Test that borrow equals self_most_sig at end of sub_to_same_size */
    {
        let mut x = MPNat::from_big_endian(&[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xae, 0x5f, 0xf0, 0x8b, 0xfc, 0x02,
            0x71, 0xa4, 0xfe, 0xe0, 0x49, 0x02, 0xc9, 0xd9, 0x12, 0x61, 0x8e, 0xf5, 0x02, 0x2c,
            0xa0, 0x00, 0x00, 0x00,
        ]);
        let y = MPNat::from_big_endian(&[
            0xae, 0x5f, 0xf0, 0x8b, 0xfc, 0x02, 0x71, 0xa4, 0xfe, 0xe0, 0x49, 0x0f, 0x70, 0x00,
            0x00, 0x00,
        ]);
        x.sub_to_same_size(&y);
    }

    /* Additional test for sub_to_same_size q_hat/r_hat adjustment logic */
    {
        let mut x = MPNat::from_big_endian(&[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff,
            0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ]);
        let y = MPNat::from_big_endian(&[
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
            0x00, 0x00,
        ]);
        x.sub_to_same_size(&y);
    }
}

#[test]
fn test_mp_nat_is_odd() {
    fn check_is_odd(n: u128) {
        let mp = MPNat::from_big_endian(&n.to_be_bytes());
        assert_eq!(mp.is_odd(), n % 2 == 1, "{n} failed is_odd test");
    }

    for n in 0..1025 {
        check_is_odd(n);
    }
    for n in 0xFF_FF_FF_FF_00_00_00_00..0xFF_FF_FF_FF_00_00_04_01 {
        check_is_odd(n);
    }
}

#[test]
fn test_mp_nat_is_power_of_two() {
    fn check_is_p2(n: u128, expected_result: bool) {
        let mp = MPNat::from_big_endian(&n.to_be_bytes());
        assert_eq!(
            mp.is_power_of_two(),
            expected_result,
            "{n} failed is_power_of_two test"
        );
    }

    check_is_p2(0, false);
    check_is_p2(1, true);
    check_is_p2(1327, false);
    check_is_p2((1 << 1) + (1 << 35), false);
    check_is_p2(1 << 1, true);
    check_is_p2(1 << 2, true);
    check_is_p2(1 << 3, true);
    check_is_p2(1 << 4, true);
    check_is_p2(1 << 5, true);
    check_is_p2(1 << 31, true);
    check_is_p2(1 << 32, true);
    check_is_p2(1 << 64, true);
    check_is_p2(1 << 65, true);
    check_is_p2(1 << 127, true);
}

#[test]
fn test_mp_nat_be() {
    fn be_round_trip(hex_input: &str) {
        let bytes = hex::decode(hex_input).unwrap();
        let mp = MPNat::from_big_endian(&bytes);
        let output = mp.to_big_endian();
        let hex_output = hex::encode(output);
        let trimmed = match hex_input.trim_start_matches('0') {
            "" => "00",
            x => x,
        };
        assert_eq!(hex_output, trimmed);
    }

    be_round_trip("");
    be_round_trip("00");
    be_round_trip("77");
    be_round_trip("abcd");
    be_round_trip("00000000abcd");
    be_round_trip("abcdef");
    be_round_trip("abcdef00");
    be_round_trip("abcdef0011");
    be_round_trip("abcdef001122");
    be_round_trip("abcdef00112233");
    be_round_trip("abcdef0011223344");
    be_round_trip("abcdef001122334455");
    be_round_trip("abcdef01234567891011121314151617181920");
}
