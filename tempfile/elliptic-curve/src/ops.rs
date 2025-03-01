//! Traits for arithmetic operations on elliptic curve field elements.

pub use core::ops::{Add, AddAssign, Mul, Neg, Shr, ShrAssign, Sub, SubAssign};

use crypto_bigint::Integer;
use group::Group;
use subtle::{Choice, ConditionallySelectable, CtOption};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Perform an inversion on a field element (i.e. base field element or scalar)
pub trait Invert {
    /// Field element type
    type Output;

    /// Invert a field element.
    fn invert(&self) -> Self::Output;

    /// Invert a field element in variable time.
    ///
    /// ⚠️ WARNING!
    ///
    /// This method should not be used with secret values, as its variable-time
    /// operation can potentially leak secrets through sidechannels.
    fn invert_vartime(&self) -> Self::Output {
        // Fall back on constant-time implementation by default.
        self.invert()
    }
}

/// Perform a batched inversion on a sequence of field elements (i.e. base field elements or scalars)
/// at an amortized cost that should be practically as efficient as a single inversion.
pub trait BatchInvert<FieldElements: ?Sized>: Invert + Sized {
    /// The output of batch inversion. A container of field elements.
    type Output: AsRef<[Self]>;

    /// Invert a batch of field elements.
    fn batch_invert(
        field_elements: &FieldElements,
    ) -> CtOption<<Self as BatchInvert<FieldElements>>::Output>;
}

impl<const N: usize, T> BatchInvert<[T; N]> for T
where
    T: Invert<Output = CtOption<Self>>
        + Mul<Self, Output = Self>
        + Copy
        + Default
        + ConditionallySelectable,
{
    type Output = [Self; N];

    fn batch_invert(field_elements: &[Self; N]) -> CtOption<[Self; N]> {
        let mut field_elements_multiples = [Self::default(); N];
        let mut field_elements_multiples_inverses = [Self::default(); N];
        let mut field_elements_inverses = [Self::default(); N];

        let inversion_succeeded = invert_batch_internal(
            field_elements,
            &mut field_elements_multiples,
            &mut field_elements_multiples_inverses,
            &mut field_elements_inverses,
        );

        CtOption::new(field_elements_inverses, inversion_succeeded)
    }
}

#[cfg(feature = "alloc")]
impl<T> BatchInvert<[T]> for T
where
    T: Invert<Output = CtOption<Self>>
        + Mul<Self, Output = Self>
        + Copy
        + Default
        + ConditionallySelectable,
{
    type Output = Vec<Self>;

    fn batch_invert(field_elements: &[Self]) -> CtOption<Vec<Self>> {
        let mut field_elements_multiples: Vec<Self> = vec![Self::default(); field_elements.len()];
        let mut field_elements_multiples_inverses: Vec<Self> =
            vec![Self::default(); field_elements.len()];
        let mut field_elements_inverses: Vec<Self> = vec![Self::default(); field_elements.len()];

        let inversion_succeeded = invert_batch_internal(
            field_elements,
            field_elements_multiples.as_mut(),
            field_elements_multiples_inverses.as_mut(),
            field_elements_inverses.as_mut(),
        );

        CtOption::new(
            field_elements_inverses.into_iter().collect(),
            inversion_succeeded,
        )
    }
}

/// Implements "Montgomery's trick", a trick for computing many modular inverses at once.
///
/// "Montgomery's trick" works by reducing the problem of computing `n` inverses
/// to computing a single inversion, plus some storage and `O(n)` extra multiplications.
///
/// See: https://iacr.org/archive/pkc2004/29470042/29470042.pdf section 2.2.
fn invert_batch_internal<
    T: Invert<Output = CtOption<T>> + Mul<T, Output = T> + Default + ConditionallySelectable,
>(
    field_elements: &[T],
    field_elements_multiples: &mut [T],
    field_elements_multiples_inverses: &mut [T],
    field_elements_inverses: &mut [T],
) -> Choice {
    let batch_size = field_elements.len();
    if batch_size == 0
        || batch_size != field_elements_multiples.len()
        || batch_size != field_elements_multiples_inverses.len()
    {
        return Choice::from(0);
    }

    field_elements_multiples[0] = field_elements[0];
    for i in 1..batch_size {
        // $ a_n = a_{n-1}*x_n $
        field_elements_multiples[i] = field_elements_multiples[i - 1] * field_elements[i];
    }

    field_elements_multiples[batch_size - 1]
        .invert()
        .map(|multiple_of_inverses_of_all_field_elements| {
            field_elements_multiples_inverses[batch_size - 1] =
                multiple_of_inverses_of_all_field_elements;
            for i in (1..batch_size).rev() {
                // $ a_{n-1} = {a_n}^{-1}*x_n $
                field_elements_multiples_inverses[i - 1] =
                    field_elements_multiples_inverses[i] * field_elements[i];
            }

            field_elements_inverses[0] = field_elements_multiples_inverses[0];
            for i in 1..batch_size {
                // $ {x_n}^{-1} = a_{n}^{-1}*a_{n-1} $
                field_elements_inverses[i] =
                    field_elements_multiples_inverses[i] * field_elements_multiples[i - 1];
            }
        })
        .is_some()
}

/// Linear combination.
///
/// This trait enables crates to provide an optimized implementation of
/// linear combinations (e.g. Shamir's Trick), or otherwise provides a default
/// non-optimized implementation.
// TODO(tarcieri): replace this with a trait from the `group` crate? (see zkcrypto/group#25)
pub trait LinearCombination: Group {
    /// Calculates `x * k + y * l`.
    fn lincomb(x: &Self, k: &Self::Scalar, y: &Self, l: &Self::Scalar) -> Self {
        (*x * k) + (*y * l)
    }
}

/// Linear combination (extended version).
///
/// This trait enables providing an optimized implementation of
/// linear combinations (e.g. Shamir's Trick).
// TODO(tarcieri): replace the current `LinearCombination` with this in the next release
pub trait LinearCombinationExt<PointsAndScalars>: group::Curve
where
    PointsAndScalars: AsRef<[(Self, Self::Scalar)]> + ?Sized,
{
    /// Calculates `x1 * k1 + ... + xn * kn`.
    fn lincomb_ext(points_and_scalars: &PointsAndScalars) -> Self {
        points_and_scalars
            .as_ref()
            .iter()
            .copied()
            .map(|(point, scalar)| point * scalar)
            .sum()
    }
}

/// Blanket impl of the legacy [`LinearCombination`] trait for types which impl the new
/// [`LinearCombinationExt`] trait for 2-element arrays.
impl<P: LinearCombinationExt<[(P, Self::Scalar); 2]>> LinearCombination for P {
    fn lincomb(x: &Self, k: &Self::Scalar, y: &Self, l: &Self::Scalar) -> Self {
        Self::lincomb_ext(&[(*x, *k), (*y, *l)])
    }
}

/// Multiplication by the generator.
///
/// May use optimizations (e.g. precomputed tables) when available.
// TODO(tarcieri): replace this with `Group::mul_by_generator``? (see zkcrypto/group#44)
pub trait MulByGenerator: Group {
    /// Multiply by the generator of the prime-order subgroup.
    #[must_use]
    fn mul_by_generator(scalar: &Self::Scalar) -> Self {
        Self::generator() * scalar
    }
}

/// Modular reduction.
pub trait Reduce<Uint: Integer>: Sized {
    /// Bytes used as input to [`Reduce::reduce_bytes`].
    type Bytes: AsRef<[u8]>;

    /// Perform a modular reduction, returning a field element.
    fn reduce(n: Uint) -> Self;

    /// Interpret the given bytes as an integer and perform a modular reduction.
    fn reduce_bytes(bytes: &Self::Bytes) -> Self;
}

/// Modular reduction to a non-zero output.
///
/// This trait is primarily intended for use by curve implementations such
/// as the `k256` and `p256` crates.
///
/// End users should use the [`Reduce`] impl on
/// [`NonZeroScalar`][`crate::NonZeroScalar`] instead.
pub trait ReduceNonZero<Uint: Integer>: Reduce<Uint> + Sized {
    /// Perform a modular reduction, returning a field element.
    fn reduce_nonzero(n: Uint) -> Self;

    /// Interpret the given bytes as an integer and perform a modular reduction
    /// to a non-zero output.
    fn reduce_nonzero_bytes(bytes: &Self::Bytes) -> Self;
}
