use auto_impl::auto_impl;

trait Supi {}

#[auto_impl(Box, &)]
trait Foo: Supi {}


struct Dog;
impl Supi for Dog {}
impl Foo for Dog {}


fn requires_foo<T: Foo>(_: T) {}

fn main() {
    requires_foo(Dog); // should work
    requires_foo(Box::new(Dog)); // shouldn't, because `Box<Dog>: Supi` is not satisfied
}
