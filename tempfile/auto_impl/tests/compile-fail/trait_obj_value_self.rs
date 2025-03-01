use auto_impl::auto_impl;


#[auto_impl(Box)]
trait Trait {
    fn foo(self);
}

fn assert_impl<T: Trait>() {}

fn main() {
    assert_impl::<Box<dyn Trait>>();
}
