use auto_impl::auto_impl;

/// This simple trait can be implemented for `Fn` types, but not for `FnMut` or
/// `FnOnce` types. The latter two types require a mutable reference to `self`
/// or a `self` by value to be called, but `greet()` only has an immutable
/// reference. Try creating an auto-impl for `FnMut`: you should get an error.
///
/// This attribute expands to the following impl (not exactly this code, but
/// equivalent, slightly uglier code):
///
/// ```
/// impl<F: Fn(&str)> Greeter for F {
///     fn greet(&self, name: &str) {
///         self(name)
///     }
/// }
/// ```
#[auto_impl(Fn)]
trait Greeter {
    fn greet(&self, name: &str);
}

fn greet_people(greeter: impl Greeter) {
    greeter.greet("Anna");
    greeter.greet("Bob");
}

fn main() {
    // We can simply pass a closure here, since this specific closure
    // implements `Fn(&str)` and therefore also `Greeter`. Note that we need
    // explicit type annotations here. This has nothing to do with `auto_impl`,
    // but is simply a limitation of type inference.
    greet_people(|name: &str| println!("Hallo {} :)", name));
}
