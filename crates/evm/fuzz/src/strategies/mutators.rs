use alloy_primitives::{Address, I256, Sign, U256};
use proptest::{prelude::*, test_runner::TestRunner};
use rand::seq::IndexedRandom;

// Interesting 8-bit values to inject.
static INTERESTING_8: [i8; 9] = [-128, -1, 0, 1, 16, 32, 64, 100, 127];

/// Interesting 16-bit values to inject.
static INTERESTING_16: [i16; 19] = [
    -128, -1, 0, 1, 16, 32, 64, 100, 127, -32768, -129, 128, 255, 256, 512, 1000, 1024, 4096, 32767,
];

/// Interesting 32-bit values to inject.
static INTERESTING_32: [i32; 27] = [
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
pub(crate) trait IncrementDecrementMutator: Sized + Copy {
    fn increment_decrement(self, size: usize, rng: &mut TestRunner) -> Option<Self>;
}

macro_rules! impl_increment_decrement {
    ($($t:ty),*) => {
        $(
            impl IncrementDecrementMutator for $t {
                fn increment_decrement(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
                    let mutated = if test_runner.rng().random::<bool>() {
                        self.wrapping_add(Self::ONE)
                    } else {
                        self.wrapping_sub(Self::ONE)
                    };
                    Self::validate(self, mutated, size)
                }
            }
        )*
    };
}

impl_increment_decrement!(U256, I256);

/// Mutator that flips random bit of uint, int or address.
pub(crate) trait BitFlipMutator: Sized + Copy {
    fn flip_random_bit(
        value: Self,
        size: Option<usize>,
        test_runner: &mut TestRunner,
    ) -> Option<Self>;
}

impl BitFlipMutator for U256 {
    fn flip_random_bit(
        value: Self,
        size: Option<usize>,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        let bit_index = test_runner.rng().random_range(0..size?);
        let mask = Self::from(1u8) << bit_index;
        let flipped = value ^ mask;
        validate_uint_mutation(value, flipped, size?)
    }
}

impl BitFlipMutator for I256 {
    fn flip_random_bit(value: Self, size: Option<usize>, rng: &mut TestRunner) -> Option<Self> {
        let bit_index = rng.rng().random_range(0..size?);
        let (sign, mut abs): (Sign, U256) = value.into_sign_and_abs();
        abs ^= U256::from(1u8) << bit_index;
        let flipped = Self::checked_from_sign_and_abs(sign, abs)?;
        validate_int_mutation(value, flipped, size?)
    }
}

impl BitFlipMutator for Address {
    fn flip_random_bit(
        value: Self,
        _size: Option<usize>,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        let bit_index = test_runner.rng().random_range(0..160);
        let mut bytes = value.0;
        bytes[bit_index / 8] ^= 1 << (bit_index % 8);
        let mutated_val = Self::from(bytes);
        trace!(target: "abi_mutation", "Address flip random bit: {value} -> {mutated_val}");
        Some(mutated_val)
    }
}

/// Mutator that randomly inserts interesting words in uint and int.
/// See <https://github.com/AFLplusplus/LibAFL/blob/90cb9a2919faf386e0678870e52784070cdac4b6/crates/libafl/src/mutators/mutations.rs#L88-L123>.
pub(crate) trait InterestingMutator: Sized + Copy {
    fn to_be_bytes(&self) -> [u8; 32];
    fn from_be_bytes(bytes: [u8; 32]) -> Self;
    fn validate(old: Self, new: Self, size: usize) -> Option<Self>;
}

pub(crate) trait InterestingByteMutator: InterestingMutator {
    fn mutate_interesting_byte(
        value: Self,
        size: usize,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        let byte_size = size / 8;
        let mut bytes = value.to_be_bytes();
        let byte_index = test_runner.rng().random_range(32 - byte_size..32);
        let interesting = *INTERESTING_8.choose(&mut test_runner.rng()).unwrap() as u8;
        bytes[byte_index] = interesting;
        let mutated = Self::from_be_bytes(bytes);
        Self::validate(value, mutated, size)
    }
}

pub(crate) trait InterestingWordMutator: InterestingMutator {
    fn mutate_interesting_word(
        value: Self,
        size: usize,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        let byte_size = size / 8;
        if byte_size < 2 {
            return None;
        }

        let mut bytes = value.to_be_bytes();
        let word_index = test_runner.rng().random_range(16 - byte_size / 2..16);
        let interesting = *INTERESTING_16.choose(&mut test_runner.rng()).unwrap() as u16;
        let word_bytes = interesting.to_be_bytes();
        let start = word_index * 2;
        bytes[start..start + 2].copy_from_slice(&word_bytes);
        let mutated = Self::from_be_bytes(bytes);
        Self::validate(value, mutated, size)
    }
}

pub(crate) trait InterestingDWordMutator: InterestingMutator {
    fn mutate_interesting_dword(
        value: Self,
        size: usize,
        test_runner: &mut TestRunner,
    ) -> Option<Self> {
        let byte_size = size / 16;
        if byte_size < 4 {
            return None;
        }

        let mut bytes = value.to_be_bytes();
        let dword_index = test_runner.rng().random_range(8 - byte_size / 4..8);
        let interesting = *INTERESTING_32.choose(&mut test_runner.rng()).unwrap() as u32;
        let start = dword_index * 4;
        bytes[start..start + 4].copy_from_slice(&interesting.to_be_bytes());
        let mutated = Self::from_be_bytes(bytes);
        Self::validate(value, mutated, size)
    }
}

impl InterestingMutator for U256 {
    fn to_be_bytes(&self) -> [u8; 32] {
        Self::to_be_bytes(self)
    }

    fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self::from_be_bytes(bytes)
    }

    /// Returns mutated uint value if different from the original value and if it fits in the given
    /// size, otherwise None.
    fn validate(old: Self, new: Self, size: usize) -> Option<Self> {
        validate_uint_mutation(old, new, size)
    }
}

impl InterestingByteMutator for U256 {}
impl InterestingWordMutator for U256 {}
impl InterestingDWordMutator for U256 {}

impl InterestingMutator for I256 {
    fn to_be_bytes(&self) -> [u8; 32] {
        Self::to_be_bytes(self)
    }

    fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self::from_be_bytes(bytes)
    }

    /// Returns mutated int value if different from the original value and if it fits in the given
    /// size, otherwise None.
    fn validate(old: Self, new: Self, size: usize) -> Option<Self> {
        validate_int_mutation(old, new, size)
    }
}

impl InterestingByteMutator for I256 {}
impl InterestingWordMutator for I256 {}
impl InterestingDWordMutator for I256 {}

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
                let flipped = U256::flip_random_bit(value, Some(size), &mut runner);
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
                let flipped = I256::flip_random_bit(value, Some(size), &mut runner);
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
    fn test_bit_flip_address() {
        let mut runner = TestRunner::new(Config::default());
        let value = Address::ZERO;

        for _ in 0..100 {
            let flipped = Address::flip_random_bit(value, None, &mut runner);
            assert!(flipped.is_some());
            assert_ne!(flipped.unwrap(), value);
        }
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
