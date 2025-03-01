use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait Foo {
    fn a(&self);
    fn b(&self);
}


fn main() {}
