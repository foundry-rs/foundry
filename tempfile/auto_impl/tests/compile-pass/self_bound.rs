use auto_impl::auto_impl;


#[auto_impl(&)]
trait Trait {
    fn foo(&self)
        where Self: Clone;
}

#[derive(Clone)]
struct Foo {}
impl Trait for Foo {
    fn foo(&self)
        where Self: Clone,
    {}
}

fn assert_impl<T: Trait>() {}

fn main() {
    assert_impl::<Foo>();
    assert_impl::<&Foo>();
}
