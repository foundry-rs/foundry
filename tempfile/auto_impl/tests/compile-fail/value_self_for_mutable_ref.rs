use auto_impl::auto_impl;


#[auto_impl(&mut)]
trait Foo {
    fn foo(self);
}


fn main() {}
