use std::fmt::Display;

use auto_impl::auto_impl;

/// This trait can be implemented for all reference or pointer types: &, &mut,
/// Box, Rc and Arc.
///
/// This attribute expands to the following impl (not exactly this code, but
/// equivalent, slightly uglier code):
///
/// ```
/// impl<'a, T: 'a + DisplayCollection> DisplayCollection for &'a T {
///     type Out = T::Out;
///     fn display_at(&self, index: usize) -> Option<&Self::Out> {
///         (**self).display_at(index)
///     }
/// }
///
/// impl<T: DisplayCollection> DisplayCollection for Box<T> {
///     type Out = T::Out;
///     fn display_at(&self, index: usize) -> Option<&Self::Out> {
///         (**self).display_at(index)
///     }
/// }
/// ```
#[auto_impl(&, Box)]
trait DisplayCollection {
    /// If the length is statically known, this is `Some(len)`.
    const LEN: Option<usize>;
    type Out: Display;
    fn display_at(&self, index: usize) -> Option<&Self::Out>;
}

impl<T: Display> DisplayCollection for Vec<T> {
    type Out = T;

    const LEN: Option<usize> = None;

    fn display_at(&self, index: usize) -> Option<&Self::Out> {
        self.get(index)
    }
}

fn show_first(c: impl DisplayCollection) {
    match c.display_at(0) {
        Some(x) => println!("First: {}", x),
        None => println!("Nothing :/"),
    }
}

#[allow(clippy::needless_borrow)]
#[rustfmt::skip]
fn main() {
    let v = vec!["dog", "cat"];
    let boxed = Box::new(v.clone());

    show_first(v.clone());      // Vec<&str>    (our manual impl)
    show_first(&v);             // &Vec<&str>   (auto-impl)
    show_first(&&v);            // &&Vec<&str>  (works too, of course)
    show_first(boxed.clone());  // Box<Vec<&str>> (auto-impl)
    show_first(&boxed);         // &Box<Vec<&str>>
}
