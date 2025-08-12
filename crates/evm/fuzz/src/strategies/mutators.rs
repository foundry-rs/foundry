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
    fn increment_decrement(self, rng: &mut TestRunner) -> Self;
}

macro_rules! impl_increment_decrement {
    ($($t:ty),*) => {
        $(
            impl IncrementDecrementMutator for $t {
                fn increment_decrement(self, test_runner: &mut TestRunner) -> Self {
                    if test_runner.rng().random::<bool>() {
                        self.wrapping_add(Self::ONE)
                    } else {
                        self.wrapping_sub(Self::ONE)
                    }
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
        let mut bytes = value.to_be_bytes();
        let byte_index = test_runner.rng().random_range(0..32);
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
        let mut bytes = value.to_be_bytes();
        let word_index = test_runner.rng().random_range(0..16);
        let interesting = *INTERESTING_16.choose(&mut test_runner.rng()).unwrap() as u16;
        let start = word_index * 2;
        bytes[start..start + 2].copy_from_slice(&interesting.to_be_bytes());
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
        let mut bytes = value.to_be_bytes();
        let word_index = test_runner.rng().random_range(0..8);
        let interesting = *INTERESTING_32.choose(&mut test_runner.rng()).unwrap() as u32;
        let start = word_index * 4;
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

/// Returns mutated uint value if different than the original value and if it fits in the given
/// size, otherwise None.
fn validate_uint_mutation(original_value: U256, mutated_value: U256, size: usize) -> Option<U256> {
    let max_value = if size < 256 { (U256::from(1) << size) - U256::from(1) } else { U256::MAX };
    if original_value != mutated_value && mutated_value < max_value {
        Some(mutated_value)
    } else {
        None
    }
}

/// Returns mutated int value if different than the original value and if it fits in the given size,
/// otherwise None.
fn validate_int_mutation(original_value: I256, mutated_value: I256, size: usize) -> Option<I256> {
    let umax: U256 = (U256::from(1) << (size - 1)) - U256::from(1);
    if original_value != mutated_value
        && match mutated_value.sign() {
            Sign::Positive => {
                mutated_value < I256::overflowing_from_sign_and_abs(Sign::Positive, umax).0
            }
            Sign::Negative => {
                mutated_value >= I256::overflowing_from_sign_and_abs(Sign::Negative, umax).0
            }
        }
    {
        Some(mutated_value)
    } else {
        None
    }
}
