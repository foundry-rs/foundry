use auto_impl::auto_impl;


#[auto_impl(&)]
trait Foo {
    fn required(&self);

    #[auto_impl(keep_default_for(&))]
    fn provided(&self) where Self: Clone {}
}


fn main() {}
