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

/// Multipliers used to define the 3 standard deviation range of a Gaussian-like curve.
/// For example, a multiplier of 0.25 means the +/-3 standard deviation bounds are +/-25% of the
/// original value.
static THREE_SIGMA_MULTIPLIERS: &[f64] = &[0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];

/// Mutator that randomly increments or decrements an uint or int.
pub(crate) trait IncrementDecrementMutator: Sized + Copy + Debug {
    fn validate(old: Self, new: Self, size: usize) -> Option<Self>;

    #[instrument(
        name = "mutator::increment_decrement",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
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

/// Mutator that changes the current value of an uint or int by applying gaussian noise.
pub(crate) trait GaussianNoiseMutator: Sized + Copy + Debug {
    fn mutate_with_gaussian_noise(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
}

impl GaussianNoiseMutator for U256 {
    #[instrument(
        name = "U256::mutate_with_gaussian_noise",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_with_gaussian_noise(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let scale_factor = sample_gaussian_scale(&mut test_runner.rng())?;
        let mut bytes: [u8; 32] = self.to_be_bytes();
        apply_scale_to_bytes(&mut bytes[32 - size / 8..], scale_factor)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl GaussianNoiseMutator for I256 {
    #[instrument(
        name = "I256::mutate_with_gaussian_noise",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_with_gaussian_noise(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let scale_factor = sample_gaussian_scale(&mut test_runner.rng())?;
        let mut bytes: [u8; 32] = self.to_be_bytes();
        apply_scale_to_bytes(&mut bytes[32 - size / 8..], scale_factor)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

/// Mutator that bounds the current value of an uint or int in the given range.
/// The mutated value is always different from the current value.
pub trait BoundMutator: Sized + Copy + Debug {
    /// Returns a value in `[min, max]` other than `self`, or `None` if `self` is outside the
    /// range or the range has no other value.
    ///
    /// Every other value of the range can be returned. The draw favors small offsets from `min`,
    /// and for signed values also from `max`.
    fn bound(self, min: Self, max: Self, test_runner: &mut TestRunner) -> Option<Self>;
}

impl BoundMutator for U256 {
    #[instrument(name = "U256::bound", level = "trace", skip(test_runner), ret)]
    fn bound(self, min: Self, max: Self, test_runner: &mut TestRunner) -> Option<Self> {
        if min >= max || self < min || self > max {
            return None;
        }
        Some(bound_word(self, min, max, random_width_word(test_runner), false))
    }
}

impl BoundMutator for I256 {
    #[instrument(name = "I256::bound", level = "trace", skip(test_runner), ret)]
    fn bound(self, min: Self, max: Self, test_runner: &mut TestRunner) -> Option<Self> {
        if min >= max || self < min || self > max {
            return None;
        }
        let draw = random_width_word(test_runner);
        let flip = test_runner.rng().random();
        let bounded = bound_word(self.into_raw(), min.into_raw(), max.into_raw(), draw, flip);
        Some(Self::from_raw(bounded))
    }
}

/// Maps `draw` to a word in `[min, max]` other than `current`, given words of values ordered as
/// `min <= current <= max` with `min < max`, unsigned or two's complement. With `flip`, the draw
/// counts down from `max` instead of up from `min`.
fn bound_word(current: U256, min: U256, max: U256, draw: U256, flip: bool) -> U256 {
    // The wrapping difference of two words is the exact distance between their ordered values,
    // even for signed values across zero and over the full range.
    let span = max.wrapping_sub(min);
    let mut offset = draw % span;
    if flip {
        offset = span - U256::ONE - offset;
    }
    // Map `[0, span)` one to one onto `[0, span]` without the offset of `current`.
    if offset >= current.wrapping_sub(min) {
        offset += U256::ONE;
    }
    min.wrapping_add(offset)
}

/// Draws a word with a random bit width in `[8, 256]`.
fn random_width_word(test_runner: &mut TestRunner) -> U256 {
    let rng = test_runner.rng();
    let bits = rng.random_range(8..=256);
    U256::from_limbs(rng.random()) >> (256 - bits)
}

/// Mutator that changes the current value by flipping a random bit.
pub(crate) trait BitMutator: Sized + Copy + Debug {
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
}

impl BitMutator for U256 {
    #[instrument(name = "U256::flip_random_bit", level = "trace", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        flip_random_bit_in_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl BitMutator for I256 {
    #[instrument(name = "I256::flip_random_bit", level = "trace", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        flip_random_bit_in_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl BitMutator for Address {
    #[instrument(name = "Address::flip_random_bit", level = "trace", skip(_size, test_runner), ret)]
    fn flip_random_bit(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        flip_random_bit_in_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }
}

impl BitMutator for Word {
    #[instrument(name = "Word::flip_random_bit", level = "trace", skip(size, test_runner), ret)]
    fn flip_random_bit(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        flip_random_bit_in_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
    }
}

/// Mutator that changes the current value by randomly injecting interesting words (for uint, int,
/// address and fixed bytes) - see <https://github.com/AFLplusplus/LibAFL/blob/90cb9a2919faf386e0678870e52784070cdac4b6/crates/libafl/src/mutators/mutations.rs#L88-L123>.
pub(crate) trait InterestingWordMutator: Sized + Copy + Debug {
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self>;
}

impl InterestingWordMutator for U256 {
    #[instrument(
        name = "U256::mutate_interesting_byte",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_byte_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(
        name = "U256::mutate_interesting_word",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_word_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(
        name = "U256::mutate_interesting_dword",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_dword_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_uint_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl InterestingWordMutator for I256 {
    #[instrument(
        name = "I256::mutate_interesting_byte",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_byte_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(
        name = "I256::mutate_interesting_word",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_word_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }

    #[instrument(
        name = "I256::mutate_interesting_dword",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_dword(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes: [u8; 32] = self.to_be_bytes();
        mutate_interesting_dword_slice(&mut bytes[32 - size / 8..], test_runner)?;
        validate_int_mutation(self, Self::from_be_bytes(bytes), size)
    }
}

impl InterestingWordMutator for Address {
    #[instrument(
        name = "Address::mutate_interesting_byte",
        level = "trace",
        skip(_size, test_runner),
        ret
    )]
    fn mutate_interesting_byte(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        mutate_interesting_byte_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }

    #[instrument(
        name = "Address::mutate_interesting_word",
        level = "trace",
        skip(_size, test_runner),
        ret
    )]
    fn mutate_interesting_word(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        mutate_interesting_word_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }

    #[instrument(
        name = "Address::mutate_interesting_dword",
        level = "trace",
        skip(_size, test_runner),
        ret
    )]
    fn mutate_interesting_dword(self, _size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut mutated = self;
        mutate_interesting_dword_slice(mutated.as_mut_slice(), test_runner)?;
        (self != mutated).then_some(mutated)
    }
}

impl InterestingWordMutator for Word {
    #[instrument(
        name = "Word::mutate_interesting_byte",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_byte(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_byte_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
    }

    #[instrument(
        name = "Word::mutate_interesting_word",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
    fn mutate_interesting_word(self, size: usize, test_runner: &mut TestRunner) -> Option<Self> {
        let mut bytes = self;
        let slice = &mut bytes[..size];
        mutate_interesting_word_slice(slice, test_runner)?;
        (self != bytes).then_some(bytes)
    }

    #[instrument(
        name = "Word::mutate_interesting_dword",
        level = "trace",
        skip(size, test_runner),
        ret
    )]
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

/// Samples a scale factor from a pseudo-Gaussian distribution centered around 1.0.
///
/// - Select a random standard deviation multiplier from a predefined set.
/// - Approximates a standard normal distribution using the Irwin-Hall method (sum of uniform
///   samples).
/// - Scales the normal value by the chosen standard deviation multiplier, divided by 3 to get
///   standard deviation.
/// - Adds 1.0 to center the scale factor around 1.0 (no mutation).
///
/// Returns a scale factor that, when applied to a number, mimics Gaussian noise.
fn sample_gaussian_scale<R: Rng>(rng: &mut R) -> Option<f64> {
    let num_samples = 8;
    let chosen_3rd_sigma = *THREE_SIGMA_MULTIPLIERS.choose(rng).unwrap_or(&1.0);

    let mut sum = 0.0;
    for _ in 0..num_samples {
        sum += rng.random::<f64>();
    }

    let standard_normal = sum - (num_samples as f64 / 2.0);
    let mut scale_factor = (chosen_3rd_sigma / 3.0) * standard_normal;
    scale_factor += 1.0;

    if scale_factor < 0.0 || (scale_factor - 1.0).abs() < f64::EPSILON {
        None
    } else {
        Some(scale_factor)
    }
}

/// Applies a floating-point scale factor to a byte slice representing an unsigned or signed
/// integer.
fn apply_scale_to_bytes(bytes: &mut [u8], scale_factor: f64) -> Option<()> {
    let mut carry_down = 0.0;

    for i in (0..bytes.len()).rev() {
        let byte_val = bytes[i] as f64;
        let scaled = f64::mul_add(carry_down, 256.0, byte_val) * scale_factor;

        if i == 0 && scaled >= 256.0 {
            for b in bytes.iter_mut() {
                *b = 0xFF;
            }
            return Some(());
        }

        bytes[i] = (scaled % 256.0).floor() as u8;

        let mut carry_up = (scaled / 256.0).floor();
        carry_down = (scaled % 1.0) / scale_factor;

        let mut j = i;
        // Propagate carry_up until it is zero or no more bytes left
        while carry_up > 0.0 && j > 0 {
            j -= 1;
            let new_val = bytes[j] as f64 + carry_up;
            if j == 0 && new_val >= 256.0 {
                for b in bytes.iter_mut() {
                    *b = 0xFF;
                }
                return Some(());
            }
            bytes[j] = (new_val % 256.0).floor() as u8;
            carry_up = (new_val / 256.0).floor();
        }
    }

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
    let max = if size < 256 { (U256::ONE << size) - U256::ONE } else { U256::MAX };
    (mutated <= max).then_some(mutated)
}

/// Returns mutated int value if different from the original value and if it fits in the given size,
/// otherwise None.
fn validate_int_mutation(original: I256, mutated: I256, size: usize) -> Option<I256> {
    // Early return if mutated value is the same as original value.
    if mutated == original {
        return None;
    }

    // Check if mutated value fits the given size.
    // Valid signed range for `size` bits is `[-2^(size-1), 2^(size-1) - 1]`.
    let max =
        I256::overflowing_from_sign_and_abs(Sign::Positive, (U256::ONE << (size - 1)) - U256::ONE)
            .0;
    let min = I256::overflowing_from_sign_and_abs(Sign::Negative, U256::ONE << (size - 1)).0;
    match mutated.sign() {
        Sign::Positive => mutated <= max,
        Sign::Negative => mutated >= min,
    }
    .then_some(mutated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::test_runner::Config;

    #[test]
    fn validate_uint_mutation_accepts_inclusive_max() {
        // For width `size`, valid unsigned values are 0..=max where max = 2^size - 1.
        // Regression: `< max` incorrectly rejected `mutated == max`; must use `<= max`.
        for size in [8usize, 16, 32, 64, 128] {
            let max = (U256::ONE << size) - U256::ONE;
            assert_eq!(
                super::validate_uint_mutation(U256::ZERO, max, size),
                Some(max),
                "size={size}"
            );
        }
        let original = U256::ONE;
        assert_eq!(super::validate_uint_mutation(original, U256::MAX, 256), Some(U256::MAX));
    }

    #[test]
    fn validate_int_mutation_accepts_inclusive_bounds() {
        // For width `size`, valid signed values are [-2^(size-1), 2^(size-1) - 1].
        // Regression: strict comparisons rejected the inclusive max and min bounds.
        for size in [8usize, 16, 32, 64, 128] {
            let max = I256::overflowing_from_sign_and_abs(
                Sign::Positive,
                (U256::ONE << (size - 1)) - U256::ONE,
            )
            .0;
            let min =
                I256::overflowing_from_sign_and_abs(Sign::Negative, U256::ONE << (size - 1)).0;
            assert_eq!(
                super::validate_int_mutation(I256::ZERO, max, size),
                Some(max),
                "size={size} max"
            );
            assert_eq!(
                super::validate_int_mutation(I256::ZERO, min, size),
                Some(min),
                "size={size} min"
            );
        }
        assert_eq!(super::validate_int_mutation(I256::ZERO, I256::MAX, 256), Some(I256::MAX));
        assert_eq!(super::validate_int_mutation(I256::ZERO, I256::MIN, 256), Some(I256::MIN));
    }

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

    #[test]
    fn test_u256_bound() {
        let mut runner = TestRunner::new(Config::default());
        let min = U256::ZERO;
        let max = U256::from(200u64);
        let original = U256::from(100u64);

        for _ in 0..50 {
            let result = original.bound(min, max, &mut runner);
            assert!(result.is_some(), "Mutation should occur");

            let mutated = result.unwrap();
            assert!(mutated >= min, "Mutated value >= min");
            assert!(mutated <= max, "Mutated value <= max");
            assert_ne!(mutated, original, "mutated value should differ from original");
        }

        // Test bound in [min, max] range.
        let result = original.bound(U256::MIN, U256::MAX, &mut runner);
        assert!(result.is_some(), "Mutation should occur");
    }

    #[test]
    fn u256_bound_reaches_every_other_value() {
        let bound_u256 = |current, min, max, draw| bound_word(current, min, max, draw, false);
        for (min, max) in [(0u64, 1u64), (0, 5), (7, 12)] {
            for current in min..=max {
                let mut hit = (0..max - min)
                    .map(|draw| {
                        bound_u256(
                            U256::from(current),
                            U256::from(min),
                            U256::from(max),
                            U256::from(draw),
                        )
                    })
                    .collect::<Vec<_>>();
                hit.sort();
                let expected =
                    (min..=max).filter(|v| *v != current).map(U256::from).collect::<Vec<_>>();
                assert_eq!(hit, expected, "current {current} in [{min}, {max}]");
            }
        }

        // Full range: `max` and values above `2^128` are reachable, `current` is skipped, and
        // draws wrap modulo the `2^256 - 1` other values.
        let last = U256::MAX - U256::ONE;
        assert_eq!(bound_u256(U256::ZERO, U256::ZERO, U256::MAX, last), U256::MAX);
        assert_eq!(bound_u256(U256::ZERO, U256::ZERO, U256::MAX, U256::ZERO), U256::ONE);
        assert_eq!(bound_u256(U256::MAX, U256::ZERO, U256::MAX, last), last);
        assert_eq!(bound_u256(U256::MAX, U256::ZERO, U256::MAX, U256::MAX), U256::ZERO);
        let high = U256::ONE << 200;
        assert_eq!(bound_u256(U256::ZERO, U256::ZERO, U256::MAX, high - U256::ONE), high);

        // Narrow range at the top of the domain, with `current` at either end.
        let (min, max) = (U256::MAX - U256::ONE, U256::MAX);
        assert_eq!(bound_u256(max, min, max, U256::MAX), min);
        assert_eq!(bound_u256(min, min, max, U256::MAX), max);

        let mut runner = TestRunner::new(Config::default());
        let wide = (0..256)
            .filter(|_| U256::ZERO.bound(U256::ZERO, U256::MAX, &mut runner).unwrap() > high)
            .count();
        assert!(wide > 0, "random draws reach values above 2^200");
    }

    #[test]
    fn i256_bound_reaches_every_other_value() {
        let int = |value: i64| I256::try_from(value).unwrap();
        let bound_i256 = |current: I256, min: I256, max: I256, draw, flip| {
            I256::from_raw(bound_word(
                current.into_raw(),
                min.into_raw(),
                max.into_raw(),
                draw,
                flip,
            ))
        };
        for (min, max) in [(-1i64, 0i64), (-3, 3), (2, 6), (-7, -2)] {
            for current in min..=max {
                for flip in [false, true] {
                    let mut hit = (0..(max - min) as u64)
                        .map(|draw| {
                            bound_i256(int(current), int(min), int(max), U256::from(draw), flip)
                        })
                        .collect::<Vec<_>>();
                    hit.sort();
                    let expected =
                        (min..=max).filter(|v| *v != current).map(int).collect::<Vec<_>>();
                    assert_eq!(hit, expected, "current {current} in [{min}, {max}], flip {flip}");
                }
            }
        }

        // Full range: both ends are reachable from either direction, and `current` is skipped.
        let (min, max) = (I256::MIN, I256::MAX);
        let last = U256::MAX - U256::ONE;
        assert_eq!(bound_i256(I256::ZERO, min, max, U256::ZERO, false), min);
        assert_eq!(bound_i256(I256::ZERO, min, max, last, false), max);
        assert_eq!(bound_i256(I256::ZERO, min, max, U256::ZERO, true), max);
        assert_eq!(bound_i256(I256::ZERO, min, max, last, true), min);
        assert_eq!(bound_i256(min, min, max, U256::ZERO, false), min + I256::ONE);
        assert_eq!(bound_i256(max, min, max, U256::ZERO, true), max - I256::ONE);
        assert_eq!(bound_i256(min, min, max, last, true), min + I256::ONE);
        assert_eq!(bound_i256(max, min, max, last, false), max - I256::ONE);
        // Offset `2^255` crosses zero onto the first non-negative value.
        let half = U256::ONE << 255;
        assert_eq!(bound_i256(min, min, max, half - U256::ONE, false), I256::ZERO);

        // Narrow ranges at both ends of the domain.
        assert_eq!(bound_i256(min, min, min + I256::ONE, U256::MAX, false), min + I256::ONE);
        assert_eq!(bound_i256(max, max - I256::ONE, max, U256::MAX, true), max - I256::ONE);

        let mut runner = TestRunner::new(Config::default());
        let bound = I256::from_raw(U256::ONE << 200);
        let (mut low, mut high) = (false, false);
        for _ in 0..256 {
            let value = I256::ZERO.bound(min, max, &mut runner).unwrap();
            low |= value < -bound;
            high |= value > bound;
        }
        assert!(low && high, "random draws reach values beyond ±2^200");
    }

    #[test]
    fn bound_rejects_ranges_without_another_value() {
        let mut runner = TestRunner::new(Config::default());
        let one = U256::ONE;
        assert_eq!(one.bound(one, one, &mut runner), None);
        assert_eq!(one.bound(U256::from(2), U256::from(3), &mut runner), None);
        assert_eq!(one.bound(U256::from(3), U256::ZERO, &mut runner), None);
        assert_eq!(I256::MIN.bound(I256::ZERO, I256::MAX, &mut runner), None);
        assert_eq!(I256::ZERO.bound(I256::ZERO, I256::ZERO, &mut runner), None);
        assert_eq!(I256::ZERO.bound(I256::ONE, I256::MINUS_ONE, &mut runner), None);
    }

    #[test]
    fn test_i256_bound() {
        let mut runner = TestRunner::new(Config::default());
        let min = I256::from_dec_str("-100").unwrap();
        let max = I256::from_dec_str("100").unwrap();
        let original = I256::from_dec_str("10").unwrap();

        for _ in 0..50 {
            let result = original.bound(min, max, &mut runner);
            assert!(result.is_some(), "Mutation should occur");

            let mutated = result.unwrap();
            assert!(mutated >= min, "Mutated value >= min");
            assert!(mutated <= max, "Mutated value <= max");
            assert_ne!(mutated, original, "Mutated value should not equal current");
        }

        // Test bound in [min, max] range.
        let result = original.bound(I256::MIN, I256::MAX, &mut runner);
        assert!(result.is_some(), "Mutation should occur");
    }
}
