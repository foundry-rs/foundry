use crate::counter::Counter;

/// Conversion into a [`Counter`].
///
/// # Examples
///
/// This trait is implemented for unsigned integers over
/// [`ItemsCount`](crate::counter::ItemsCount):
///
/// ```
/// #[divan::bench]
/// fn sort_values(bencher: divan::Bencher) {
///     # type T = String;
///     let mut values: Vec<T> = // ...
///     # Vec::new();
///     bencher
///         .counter(values.len())
///         .bench_local(|| {
///             divan::black_box(&mut values).sort();
///         });
/// }
/// ```
pub trait IntoCounter {
    /// Which kind of counter are we turning this into?
    type Counter: Counter;

    /// Converts into a [`Counter`].
    fn into_counter(self) -> Self::Counter;
}

impl<C: Counter> IntoCounter for C {
    type Counter = C;

    #[inline]
    fn into_counter(self) -> Self::Counter {
        self
    }
}
