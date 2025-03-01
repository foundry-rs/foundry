//! Example to demonstrate how to use the `keep_default_for` attribute.
//!
//! The generated `impl` blocks generate an item for each trait item by
//! default. This means that default methods in traits are also implemented via
//! the proxy type. Sometimes, this is not what you want. One special case is
//! when the default method has where bounds that don't apply to the proxy
//! type.
use auto_impl::auto_impl;

#[auto_impl(&, Box)]
trait Foo {
    fn required(&self) -> String;

    // The generated impl for `&T` will not override this method.
    #[auto_impl(keep_default_for(&))]
    fn provided(&self) {
        println!("Hello {}", self.required());
    }
}

impl Foo for String {
    fn required(&self) -> String {
        self.clone()
    }

    fn provided(&self) {
        println!("привет {}", self);
    }
}

fn test_foo(x: impl Foo) {
    x.provided();
}

fn main() {
    let s = String::from("Peter");

    // Output: "привет Peter", because `String` has overwritten the default
    // method.
    test_foo(s.clone());

    // Output: "Hello Peter", because the method is not overwritten for the
    // `&T` impl block.
    test_foo(&s);

    // Output: "привет Peter", because the `Box<T>` impl overwrites the method
    // by default, if you don't specify `keep_default_for`.
    test_foo(Box::new(s));
}
