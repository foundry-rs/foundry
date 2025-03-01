//! A base library for interfacing with streams of vectors and matrices.
//!
//! This library presents the abstraction layer for the _streaming model_.
//! Essentially, it provides a set of handy utilities as a wrapper around
//! iterators.

mod rev;
pub use rev::Reverse;

/// The trait [`Iterable`] represents a streamable object that can produce
/// an arbitrary number of streams of length [`Iterable::len`](Iterable::len).
///
/// An Iterable is pretty much like an [`IntoIterator`] that can be copied over
/// and over, and has an hint of the length.  Copies are meant to be shared
/// across threads safely.
///
/// # Examples
///
/// ```
/// use ark_std::borrow::Borrow;
/// use ark_std::iterable::Iterable;
///
/// // Relying only on standard library
/// fn f(xs: impl IntoIterator<Item=impl Borrow<u32>> + Clone) -> u32 {
///     xs.clone().into_iter().fold(1, |x, y| x.borrow() * y.borrow()) +
///     xs.clone().into_iter().fold(0, |x, y| x.borrow() + y.borrow()) +
///     xs.into_iter().size_hint().0 as u32
/// }
///
/// // Relying on the trait below
/// fn g(xs: impl Iterable<Item=impl Borrow<u32>>) -> u32 {
///     xs.iter().fold(1, |x, y| x.borrow() * y.borrow()) +
///     xs.iter().fold(0, |x, y| x.borrow() + y.borrow()) +
///     xs.len() as u32
/// }
///
/// // Test over a slice (which implements both traits).
/// let xs = &[1, 2, 3, 4];
/// assert_eq!(f(xs), g(xs));
/// ```
///
/// # Efficency
///
/// For efficiency, functions using iterables are often times relying on
/// [`Borrow`](std::borrow::Borrow) in order to avoid copying the contents of
/// the iterator..
///
/// The `Iter` associated type has a lifetime that is independent from that of
/// the [`Iterable`] object. This means that implicitly a copy of the relevant
/// contents of the object will happen whenever
/// [`Iterable::iter`](crate::iterable::Iterable::iter) is called. This might
/// change in the future as associated type constructors
/// [[RFC1598](https://github.com/rust-lang/rfcs/blob/master/text/1598-generic_associated_types.md#declaring--assigning-an-associated-type-constructor)]
/// stabilize.
///
/// # Future implementation
///
/// A lot of stream operations must be performed symbolically.
/// We expect that, in the future, this trait will accommodate for additional
/// streaming function, e.g. `Iterable::hadamard(&self, other: &Iterable)` to
/// perform the Hadamard product of two streams, or `Iterable::add(&self, other:
/// &Iterable)` to perform the addition of two streams.
pub trait Iterable: Send + Sync {
    /// The type of the element being streamed.
    type Item;
    /// The type of the iterator being generated.
    type Iter: Iterator<Item = Self::Item>;

    ///  Return the iterator associated to the current instance.
    ///
    /// In the so-called _streaming model_ [BCHO22], this is equivalent to
    /// instantiating a new stream tape.
    /// For base types, this acts in the same way as the `.iter()` method.
    ///
    ///  ```
    /// use ark_std::iterable::Iterable;
    ///
    /// let x = &[1, 2, 4];
    /// let mut iterator = x.iter();
    ///  ```
    fn iter(&self) -> Self::Iter;

    /// Return a hint on the length of the stream.
    ///
    /// Careful: different objects might have different indications of what
    /// _length_ means; this might not be the actual size in terms of
    /// elements.
    fn len(&self) -> usize;

    /// Return `true` if the stream is empty, else `false`.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<I> Iterable for I
where
    I: IntoIterator + Copy + Send + Sync,
    I::IntoIter: ExactSizeIterator,
{
    type Item = <I as IntoIterator>::Item;
    type Iter = <I as IntoIterator>::IntoIter;

    fn iter(&self) -> Self::Iter {
        self.into_iter()
    }

    fn len(&self) -> usize {
        self.into_iter().len()
    }
}
