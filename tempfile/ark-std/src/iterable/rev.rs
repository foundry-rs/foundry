use super::Iterable;
use crate::iter::Rev;

/// Stream that goes over an `[ExactSizeIterator]` in reverse order.
///
/// This stream allows to switch fast from little endian ordering used in
/// time-efficient algorithms, e.g. in slices `&[T]` into big endia ordering
/// (used in space-efficient algorithms.
///
/// # Examples
/// ```
/// use ark_std::iterable::{Iterable, Reverse};
///
/// let le_v = &[1, 2, 3];
/// let be_v = Reverse(le_v);
/// let mut be_v_iter = be_v.iter();
/// assert_eq!(be_v_iter.next(), Some(&3));
/// assert_eq!(be_v_iter.next(), Some(&2));
/// assert_eq!(be_v_iter.next(), Some(&1));
/// ```
#[derive(Clone, Copy)]
pub struct Reverse<I>(pub I)
where
    I: Iterable,
    I::Iter: DoubleEndedIterator;

impl<I> Iterable for Reverse<I>
where
    I: Iterable,
    I::Iter: DoubleEndedIterator,
{
    type Item = I::Item;
    type Iter = Rev<I::Iter>;

    #[inline]
    fn iter(&self) -> Self::Iter {
        self.0.iter().rev()
    }

    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
}
