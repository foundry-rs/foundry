use alloy_dyn_abi::Word;
use alloy_primitives::{Address, I256, Sign, U256};
use proptest::{prelude::*, test_runner::TestRunner};
use rand::seq::IndexedRandom;
use std::fmt::Debug;

// Interesting 8-bit values to inject.
static INTERESTING_8: &[i8] = &[-128, -1, 0, 1, 16, 32, 64, 100, 127];

/// Interesting 16-bit values to inject.
static INTERESTING_16: &[i16] = &[
    -128, -1, 0, 1, 16, 32, 64, 100, 127, -32768, -129, 128, 255, 256, 512, 1000, 1024, 4096, 32767,
];

/// Interesting 32-bit values to inject.
static INTERESTING_32: &[i32] = &[
    -128,
    -1,
    0,
    1,
    16,
    32,
    64,
    100,
    127,
    -32768,
    -129,
    128,
    255,
    256,
    512,
    1000,
    1024,
    4096,
    32767,
    -2147483648,
    -100663046,
    -32769,
    32768,
    65535,
    65536,
    100663045,
    2147483647,
];

/// Mutator that randomly increments or decrements an uint or int.
pub(crate) trait IncrementDecrementMutator: Sized + Copy + Debug {
    fn validate(old: Self, new: Self, size: usize) -> Option<Self>;

    #[instrument(name = "mutator::increment_decrement", skip(size, test_runner), ret)]
    fn increment_decrement(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mutated = if test_runner.rng().random::<bool>() {
            self.wrapping_add(Self::ONE)
        } else {
            self.wrapping_sub(Self::ONE)
        };
        Self::validate(self, mutated, size)
    }

    fn wrapping_add(self, rhs: Self) -> Self;
    fn wrapping_sub(self, rhs: Self) -> Self;
    const ONE: Self;
}

macro_rules! impl_increment_decrement_mutator {
    ($ty:ty, $validate_fn:path) => {
        impl IncrementDecrementMutator for $ty {
            fn validate(old: Self, new: Self, size: usize) -> Option<Self> {
                $validate_fn(old, new, size)
            }

            fn wrapping_add(self, rhs: Self) -> Self {
                Self::wrapping_add(self, rhs)
            }

            fn wrapping_sub(self, rhs: Self) -> Self {
                Self::wrapping_sub(self, rhs)
            }

            const ONE: Self = Self::ONE;
        }
    };
}

impl_increment_decrement_mutator!(U256, validate_uint_mutation);
impl_increment_decrement_mutator!(I256, validate_int_mutation);

/// ABI mutator that changes current value by flipping a random bit and randomly injecting
/// interesting words - see <https://github.com/AFLplusplus/LibAFL/blob/90cb9a2919faf386e0678870e52784070cdac4b6/crates/libafl/src/mutators/mutations.rs#L88-L123>.
/// Implemented for uint, int, address and fixed bytes.
pub(crate) trait AbiMutator: Sized + Copy + Debug {
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
}

impl AbiMutator for U256 {
    #[instrument(name = "U256::flip_random_bit", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        flip_random_bit_in_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(name = "U256::mutate_interesting_byte", skip(size, test_runner), ret)]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_byte_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(name = "U256::mutate_interesting_word", skip(size, test_runner), ret)]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_word_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(name = "U256::mutate_interesting_dword", skip(size, test_runner), ret)]
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_dword_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl AbiMutator for I256 {
    #[instrument(name = "I256::flip_random_bit", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        flip_random_bit_in_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(name = "I256::mutate_interesting_byte", skip(size, test_runner), ret)]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_byte_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(name = "I256::mutate_interesting_word", skip(size, test_runner), ret)]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_word_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(name = "I256::mutate_interesting_dword", skip(size, test_runner), ret)]
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_dword_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl AbiMutator for Address {
    #[instrument(name = "Address::flip_random_bit", skip(_size, test_runner), ret)]
    fn flip_random_bit(mut self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        flip_random_bit_in_slice(self.as_mut_slice(), test_runner)?;
        Some(self)
    }

    #[instrument(name = "Address::mutate_interesting_byte", skip(_size, test_runner), ret)]
    fn mutate_interesting_byte(
        mut self,
        _size: usize,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        mutate_interesting_byte_slice(self.as_mut_slice(), test_runner)?;
        Some(self)
    }

    #[instrument(name = "Address::mutate_interesting_word", skip(_size, test_runner), ret)]
    fn mutate_interesting_word(
        mut self,
        _size: usize,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        mutate_interesting_word_slice(self.as_mut_slice(), test_runner)?;
        Some(self)
    }

    #[instrument(name = "Address::mutate_interesting_dword", skip(_size, test_runner), ret)]
    fn mutate_interesting_dword(
        mut self,
        _size: usize,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        mutate_interesting_dword_slice(self.as_mut_slice(), test_runner)?;
        Some(self)
    }
}

impl AbiMutator for Word {
    #[instrument(name = "Word::flip_random_bit", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        flip_random_bit_in_slice(slice, test_runner)?;
        Some(bytes)
    }

    #[instrument(name = "Word::mutate_interesting_byte", skip(size, test_runner), ret)]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_byte_slice(slice, test_runner)?;
        Some(bytes)
    }

    #[instrument(name = "Word::mutate_interesting_word", skip(size, test_runner), ret)]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_word_slice(slice, test_runner)?;
        Some(bytes)
    }

    #[instrument(name = "Word::mutate_interesting_dword", skip(size, test_runner), ret)]
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_dword_slice(slice, test_runner)?;
        Some(bytes)
    }
}

/// Flips a random bit in the given mutable byte slice.
fn flip_random_bit_in_slice(bytes: &mut [u8], test_runner: &mut TestRunner) -> Option<()> {
    if bytes.is_empty() {
        return None;
    }
    let bit_index = test_runner.rng().random_range(0..(bytes.len() * 8));
    bytes[bit_index / 8] ^= 1 << (bit_index % 8);
    Some(())
}

/// Mutates a random byte in the given byte slice by replacing it with a randomly chosen
/// interesting 8-bit value.
fn mutate_interesting_byte_slice(bytes: &mut [u8], test_runner: &mut TestRunner) -> Option<()> {
    let index = test_runner.rng().random_range(0..bytes.len());
    let val = *INTERESTING_8.choose(&mut test_runner.rng())? as u8;
    bytes[index] = val;
    Some(())
}

/// Mutates a random 2-byte (16-bit) region in the byte slice with a randomly chosen interesting
/// 16-bit value.
fn mutate_interesting_word_slice(bytes: &mut [u8], test_runner: &mut TestRunner) -> Option<()> {
    if bytes.len() < 2 {
        return None;
    }
    let index = test_runner.rng().random_range(0..=bytes.len() - 2);
    let val = *INTERESTING_16.choose(&mut test_runner.rng())? as u16;
    bytes[index..index + 2].copy_from_slice(&val.to_be_bytes());
    Some(())
}

/// Mutates a random 4-byte (32-bit) region in the byte slice with a randomly chosen interesting
/// 32-bit value.
fn mutate_interesting_dword_slice(bytes: &mut [u8], test_runner: &mut TestRunner) -> Option<()> {
    if bytes.len() < 4 {
        return None;
    }
    let index = test_runner.rng().random_range(0..=bytes.len() - 4);
    let val = *INTERESTING_32.choose(&mut test_runner.rng())? as u32;
    bytes[index..index + 4].copy_from_slice(&val.to_be_bytes());
    Some(())
}

/// Returns mutated uint value if different from the original value and if it fits in the given
/// size, otherwise None.
fn validate_uint_mutation(original: U256, mutated: U256, size: usize) -> Option<U256> {
    // Early return if mutated value is the same as original value.
    if mutated == original {
        return None;
    }

    // Check if mutated value fits the given size.
    let max = if size < 256 { (U256::from(1) << size) - U256::from(1) } else { U256::MAX };
    (mutated < max).then_some(mutated)
}

/// Returns mutated int value if different from the original value and if it fits in the given size,
/// otherwise None.
fn validate_int_mutation(original: I256, mutated: I256, size: usize) -> Option<I256> {
    // Early return if mutated value is the same as original value.
    if mutated == original {
        return None;
    }

    // Check if mutated value fits the given size.
    let max_abs = (U256::from(1) << (size - 1)) - U256::from(1);
    match mutated.sign() {
        Sign::Positive => mutated < I256::overflowing_from_sign_and_abs(Sign::Positive, max_abs).0,
        Sign::Negative => mutated > I256::overflowing_from_sign_and_abs(Sign::Negative, max_abs).0,
    }
    .then_some(mutated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::test_runner::Config;

    #[test]
    fn test_increment_decrement_u256() {
        let mut runner = TestRunner::new(Config::default());

        let mut increment_decrement = |value: U256, expected: Vec<U256>| {
            for _ in 0..100 {
                let mutated = value.increment_decrement(8, &mut runner);
                assert!(
                    mutated.is_none() || mutated.is_some_and(|mutated| expected.contains(&mutated))
                );
            }
        };

        increment_decrement(U256::ZERO, vec![U256::ONE]);
        increment_decrement(U256::from(255), vec![U256::from(254)]);
        increment_decrement(U256::from(64), vec![U256::from(63), U256::from(65)]);
    }

    #[test]
    fn test_increment_decrement_i256() {
        let mut runner = TestRunner::new(Config::default());

        let mut increment_decrement = |value: I256, expected: Vec<I256>| {
            for _ in 0..100 {
                let mutated = value.increment_decrement(8, &mut runner);
                assert!(
                    mutated.is_none() || mutated.is_some_and(|mutated| expected.contains(&mutated))
                );
            }
        };

        increment_decrement(
            I256::from_dec_str("-128").unwrap(),
            vec![I256::from_dec_str("-127").unwrap()],
        );
        increment_decrement(
            I256::from_dec_str("127").unwrap(),
            vec![I256::from_dec_str("126").unwrap()],
        );
        increment_decrement(
            I256::from_dec_str("-47").unwrap(),
            vec![I256::from_dec_str("-48").unwrap(), I256::from_dec_str("-46").unwrap()],
        );
        increment_decrement(
            I256::from_dec_str("47").unwrap(),
            vec![I256::from_dec_str("48").unwrap(), I256::from_dec_str("46").unwrap()],
        );
    }

    #[test]
    fn test_bit_flip_u256() {
        let mut runner = TestRunner::new(Config::default());
        let size = 8;

        let mut test_bit_flip = |value: U256| {
            for _ in 0..100 {
                let flipped = U256::flip_random_bit(value, size, &mut runner);
                assert!(
                    flipped.is_none()
                        || flipped.is_some_and(
                            |flipped| flipped != value && flipped < (U256::from(1) << size)
                        )
                );
            }
        };

        test_bit_flip(U256::ZERO);
        test_bit_flip(U256::ONE);
        test_bit_flip(U256::MAX);
        test_bit_flip(U256::from(255));
    }

    #[test]
    fn test_bit_flip_i256() {
        let mut runner = TestRunner::new(Config::default());
        let size = 8;

        let mut test_bit_flip = |value: I256| {
            for _ in 0..100 {
                let flipped = I256::flip_random_bit(value, size, &mut runner);
                assert!(
                    flipped.is_none()
                        || flipped.is_some_and(|flipped| {
                            flipped != value
                                && flipped.abs().unsigned_abs() < (U256::from(1) << (size - 1))
                        })
                );
            }
        };

        test_bit_flip(I256::from_dec_str("-128").unwrap());
        test_bit_flip(I256::from_dec_str("127").unwrap());
        test_bit_flip(I256::MAX);
        test_bit_flip(I256::MIN);
        test_bit_flip(I256::MINUS_ONE);
    }

    #[test]
    fn test_mutate_interesting_byte_u256() {
        let mut runner = TestRunner::new(Config::default());
        let value = U256::from(0);
        let size = 8;

        for _ in 0..100 {
            let mutated = U256::mutate_interesting_byte(value, size, &mut runner);
            assert!(
                mutated.is_none()
                    || mutated.is_some_and(
                        |mutated| mutated != value && mutated < (U256::from(1) << size)
                    )
            );
        }
    }

    #[test]
    fn test_mutate_interesting_word_u256() {
        let mut runner = TestRunner::new(Config::default());
        let value = U256::from(0);
        let size = 16;

        for _ in 0..100 {
            let mutated = U256::mutate_interesting_word(value, size, &mut runner);
            assert!(
                mutated.is_none()
                    || mutated.is_some_and(
                        |mutated| mutated != value && mutated < (U256::from(1) << size)
                    )
            );
        }
    }

    #[test]
    fn test_mutate_interesting_dword_u256() {
        let mut runner = TestRunner::new(Config::default());
        let value = U256::from(0);
        let size = 32;

        for _ in 0..100 {
            let mutated = U256::mutate_interesting_dword(value, size, &mut runner);
            assert!(
                mutated.is_none()
                    || mutated.is_some_and(
                        |mutated| mutated != value && mutated < (U256::from(1) << size)
                    )
            );
        }
    }

    #[test]
    fn test_mutate_interesting_byte_i256() {
        let mut runner = TestRunner::new(Config::default());
        let value = I256::ZERO;
        let size = 8;

        for _ in 0..100 {
            let mutated = I256::mutate_interesting_byte(value, size, &mut runner);
            assert!(
                mutated.is_none()
                    || mutated.is_some_and(|mutated| mutated != value
                        && mutated.abs().unsigned_abs() < (U256::from(1) << (size - 1)))
            )
        }
    }

    #[test]
    fn test_mutate_interesting_word_i256() {
        let mut runner = TestRunner::new(Config::default());
        let value = I256::ZERO;
        let size = 16;

        for _ in 0..100 {
            let mutated = I256::mutate_interesting_word(value, size, &mut runner);
            assert!(
                mutated.is_none()
                    || mutated.is_some_and(|mutated| mutated != value
                        && mutated.abs().unsigned_abs() < (U256::from(1) << (size - 1)))
            )
        }
    }

    #[test]
    fn test_mutate_interesting_dword_i256() {
        let mut runner = TestRunner::new(Config::default());
        let value = I256::ZERO;
        let size = 32;

        for _ in 0..100 {
            let mutated = I256::mutate_interesting_dword(value, size, &mut runner);
            assert!(
                mutated.is_none()
                    || mutated.is_some_and(|mutated| mutated != value
                        && mutated.abs().unsigned_abs() < (U256::from(1) << (size - 1)))
            )
        }
    }

    #[test]
    fn test_mutate_address() {
        let mut runner = TestRunner::new(Config::default());
        for _ in 0..100 {
            let value = Address::random();
            assert_ne!(value, Address::flip_random_bit(value, 20, &mut runner).unwrap());
            let value1 = Address::random();
            assert_ne!(value1, Address::mutate_interesting_byte(value1, 20, &mut runner).unwrap());
            let value2 = Address::random();
            assert_ne!(value2, Address::mutate_interesting_word(value2, 20, &mut runner).unwrap());
            let value3 = Address::random();
            assert_ne!(value3, Address::mutate_interesting_dword(value3, 20, &mut runner).unwrap());
        }
    }

    #[test]
    fn test_mutate_word() {
        let mut runner = TestRunner::new(Config::default());
        for _ in 0..100 {
            let value = Word::random();
            assert_ne!(value, Word::flip_random_bit(value, 32, &mut runner).unwrap());
            let value1 = Word::random();
            assert_ne!(value1, Word::mutate_interesting_byte(value1, 32, &mut runner).unwrap());
            let value2 = Word::random();
            assert_ne!(value2, Word::mutate_interesting_word(value2, 32, &mut runner).unwrap());
            let value3 = Word::random();
            assert_ne!(value3, Word::mutate_interesting_dword(value3, 32, &mut runner).unwrap());
        }
    }

    #[test]
    fn test_mutate_interesting_word_too_small_returns_none() {
        let mut runner = TestRunner::new(Config::default());
        let value = U256::from(123);
        assert!(U256::mutate_interesting_word(value, 8, &mut runner).is_none());
    }

    #[test]
    fn test_mutate_interesting_dword_too_small_returns_none() {
        let mut runner = TestRunner::new(Config::default());
        let value = I256::from_dec_str("123").unwrap();
        assert!(I256::mutate_interesting_dword(value, 16, &mut runner).is_none());
    }
}
