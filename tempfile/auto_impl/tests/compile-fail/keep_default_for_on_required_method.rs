use auto_impl::auto_impl;


#[auto_impl(&)]
trait Foo {
    #[auto_impl(keep_default_for(&))]
    fn required(&self);
}


fn main() {}
