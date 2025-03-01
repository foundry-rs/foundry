use auto_impl::auto_impl;


#[auto_impl(Box)]
trait Trait {
    fn bar(&self);

    #[auto_impl(keep_default_for(Box))]
    fn foo(&self)
        where Self: Clone
    {}
}

fn assert_impl<T: Trait>() {}

struct Foo {}
impl Trait for Foo {
    fn bar(&self) {}
}

fn main() {
    assert_impl::<Foo>();
    assert_impl::<Box<Foo>>();
}
