use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait Foo {
    type Out;

    fn a(&self);
}


fn main() {}
