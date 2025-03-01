use std::fmt;
use auto_impl::auto_impl;


#[auto_impl(&)]
trait Trait {
    fn foo(&self)
        where Self: Clone;
    fn bar(&self)
        where Self: Default + fmt::Display;
}

#[derive(Clone, Default)]
struct Foo {}
impl Trait for Foo {
    fn foo(&self)
        where Self: Clone,
    {}
    fn bar(&self)
        where Self: Default + fmt::Display,
    {}
}

impl fmt::Display for Foo {
    fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
        unimplemented!()
    }
}

fn assert_impl<T: Trait>() {}

fn main() {
    assert_impl::<Foo>();
    assert_impl::<&Foo>();
}
