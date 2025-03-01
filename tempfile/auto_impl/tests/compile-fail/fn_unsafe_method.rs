use auto_impl::auto_impl;


#[auto_impl(Fn)]
trait Foo {
    unsafe fn a(&self);
}


fn main() {}
