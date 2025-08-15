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
    fn flip_random_bit(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        flip_random_bit_in_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }

    #[instrument(name = "Address::mutate_interesting_byte", skip(_size, test_runner), ret)]
    fn mutate_interesting_byte(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        mutate_interesting_byte_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }

    #[instrument(name = "Address::mutate_interesting_word", skip(_size, test_runner), ret)]
    fn mutate_interesting_word(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        mutate_interesting_word_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }

    #[instrument(name = "Address::mutate_interesting_dword", skip(_size, test_runner), ret)]
    fn mutate_interesting_dword(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        mutate_interesting_dword_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }
}

impl AbiMutator for Word {
    #[instrument(name = "Word::flip_random_bit", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        flip_random_bit_in_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
    }

    #[instrument(name = "Word::mutate_interesting_byte", skip(size, test_runner), ret)]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_byte_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
    }

    #[instrument(name = "Word::mutate_interesting_word", skip(size, test_runner), ret)]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_word_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
    }

    #[instrument(name = "Word::mutate_interesting_dword", skip(size, test_runner), ret)]
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_dword_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
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
    fn test_mutate_uint() {
        let mut runner = TestRunner::new(Config::default());
        let size = 32;

        let test_values =
            vec![U256::ZERO, U256::ONE, U256::from(12345u64), U256::from(255), U256::MAX];

        #[track_caller]
        fn validate_mutation(value: U256, mutated: Option<U256>) {
            assert!(
                mutated.is_none() || mutated.is_some_and(|m| m != value),
                "Mutation failed: value = {value:?}, mutated = {mutated:?}"
            );
        }

        for value in test_values {
            for _ in 0..100 {
                validate_mutation(value, U256::increment_decrement(value, size, &mut runner));
                validate_mutation(value, U256::flip_random_bit(value, size, &mut runner));
                validate_mutation(value, U256::mutate_interesting_byte(value, size, &mut runner));
                validate_mutation(value, U256::mutate_interesting_word(value, size, &mut runner));
                validate_mutation(value, U256::mutate_interesting_dword(value, size, &mut runner));
            }
        }
    }

    #[test]
    fn test_mutate_int() {
        let mut runner = TestRunner::new(Config::default());
        let size = 32;

        let test_values = vec![
            I256::ZERO,
            I256::ONE,
            I256::MINUS_ONE,
            I256::from_dec_str("12345").unwrap(),
            I256::from_dec_str("-54321").unwrap(),
            I256::from_dec_str("340282366920938463463374607431768211455").unwrap(),
            I256::from_dec_str("-340282366920938463463374607431768211455").unwrap(),
        ];

        #[track_caller]
        fn validate_mutation(value: I256, mutated: Option<I256>) {
            assert!(
                mutated.is_none() || mutated.is_some_and(|m| m != value),
                "Mutation failed: value = {value:?}, mutated = {mutated:?}"
            );
        }

        for value in test_values {
            for _ in 0..100 {
                validate_mutation(value, I256::increment_decrement(value, size, &mut runner));
                validate_mutation(value, I256::flip_random_bit(value, size, &mut runner));
                validate_mutation(value, I256::mutate_interesting_byte(value, size, &mut runner));
                validate_mutation(value, I256::mutate_interesting_word(value, size, &mut runner));
                validate_mutation(value, I256::mutate_interesting_dword(value, size, &mut runner));
            }
        }
    }

    #[test]
    fn test_mutate_address() {
        let mut runner = TestRunner::new(Config::default());
        let value = Address::random();

        #[track_caller]
        fn validate_mutation(value: Address, mutated: Option<Address>) {
            assert!(
                mutated.is_none() || mutated.is_some_and(|mutated| mutated != value),
                "Mutation failed for value: {value:?}, result: {mutated:?}"
            );
        }

        for _ in 0..100 {
            validate_mutation(value, Address::flip_random_bit(value, 20, &mut runner));
            validate_mutation(value, Address::mutate_interesting_byte(value, 20, &mut runner));
            validate_mutation(value, Address::mutate_interesting_word(value, 20, &mut runner));
            validate_mutation(value, Address::mutate_interesting_dword(value, 20, &mut runner));
        }
    }

    #[test]
    fn test_mutate_word() {
        let mut runner = TestRunner::new(Config::default());
        let value = Word::random();

        #[track_caller]
        fn validate_mutation(value: Word, mutated: Option<Word>) {
            assert!(
                mutated.is_none() || mutated.is_some_and(|mutated| mutated != value),
                "Mutation failed for value: {value:?}, result: {mutated:?}"
            );
        }

        for _ in 0..100 {
            validate_mutation(value, Word::flip_random_bit(value, 32, &mut runner));
            validate_mutation(value, Word::mutate_interesting_byte(value, 32, &mut runner));
            validate_mutation(value, Word::mutate_interesting_word(value, 32, &mut runner));
            validate_mutation(value, Word::mutate_interesting_dword(value, 32, &mut runner));
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
